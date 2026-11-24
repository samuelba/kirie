//! Marking up, as the overlay and the editor both do it: the document and its undo history, the
//! paint for new marks, the picked marks, the words being typed, and the controls that change
//! them. Each window keeps its own tool, turns pointer events into image points, and acts on
//! the [`Effect`] an edit has.

use iced::Point;
use iced::advanced::mouse::Click;
use iced::keyboard::Modifiers;
use kirie_annotate::marks::Restack;
use kirie_annotate::{
    Document, Draft, History, Kind, Op, Paint, Press, Rgba, Shape, ShapeId, SpotShape, StyleId,
    Tool, Weight,
};

use super::controls::{self, BrightnessEvent, TextEvent, WeightEvent};
use super::emoji_picker::{self, EmojiPicker};
use super::picker::{self, Picker, Target};
use super::swatches::Swatches;
use super::typing::{self, Choice, Typing};

/// An edit both windows offer.
#[derive(Debug, Clone)]
pub enum Change {
    PickStyle(StyleId),
    ToggleShadow,
    PickColor(Rgba),
    /// Opens the color picker on a swatch, or to add one.
    EditColor(Target),
    Picker(picker::Event),
    Weight(WeightEvent),
    ToggleFill,
    Brightness(BrightnessEvent),
    Spot(SpotShape),
    Look(TextEvent),
    Emoji(emoji_picker::Event),
    /// Moves through or picks the emoji offered while typing.
    Choose(Choice),
    TextEdit(iced::widget::text_editor::Action),
    TextDone,
    Undo,
    Redo,
    Delete,
    /// Copies the picked marks a step aside.
    Duplicate,
    /// Moves the picked marks up or down among the others.
    Restack(Restack),
}

impl Change {
    /// Whether the edit changes the drawing options for good: a style, color, weight, fill or
    /// text look, but not one step of a slider drag, whose last step follows.
    fn settles(&self) -> bool {
        match self {
            Change::PickStyle(_)
            | Change::ToggleShadow
            | Change::PickColor(_)
            | Change::EditColor(_)
            | Change::Picker(_)
            | Change::Weight(WeightEvent::Pick(_) | WeightEvent::Done)
            | Change::ToggleFill
            | Change::Brightness(BrightnessEvent::Done)
            | Change::Spot(_) => true,
            Change::Look(event) => !matches!(event, TextEvent::Size(_)),
            Change::Weight(WeightEvent::Slide(_))
            | Change::Brightness(BrightnessEvent::Slide(_))
            | Change::Emoji(_)
            | Change::Choose(_)
            | Change::TextEdit(_)
            | Change::TextDone
            | Change::Undo
            | Change::Redo
            | Change::Delete
            | Change::Duplicate
            | Change::Restack(_) => false,
        }
    }
}

/// What a window does after an edit.
#[derive(Debug, Default, PartialEq)]
#[must_use]
pub struct Effect {
    /// The text box takes the keyboard.
    pub focus: bool,
    /// The drawing options changed for good, to keep for the next time.
    pub settled: bool,
    /// What went wrong keeping the colors.
    pub error: Option<String>,
}

impl Effect {
    const FOCUS: Effect = Effect {
        focus: true,
        settled: false,
        error: None,
    };
}

#[derive(Debug)]
pub struct Annotator {
    pub history: History,
    /// The paint new marks get.
    pub paint: Paint,
    pub swatches: Swatches,
    pub picker: Option<Picker>,
    pub emojis: EmojiPicker,
    pub draft: Option<Draft>,
    /// The marks the select tool picked; a lone one shows its grips.
    pub picked: Vec<ShapeId>,
    /// A slider is being dragged; its moves undo as one step.
    pub sliding: bool,
    pub typing: Option<Typing>,
    /// The last press, to tell a double click.
    pub last_click: Option<Click>,
    pub modifiers: Modifiers,
}

impl Annotator {
    pub fn new(document: Document, paint: Paint, swatches: Swatches) -> Self {
        Self {
            history: History::new(document),
            paint,
            swatches,
            picker: None,
            emojis: EmojiPicker::load(),
            draft: None,
            picked: Vec::new(),
            sliding: false,
            typing: None,
            last_click: None,
            modifiers: Modifiers::default(),
        }
    }

    pub fn document(&self) -> &Document {
        self.history.current()
    }

    pub fn style(&self) -> StyleId {
        self.document().style
    }

    pub fn update(&mut self, change: Change) -> Effect {
        let settled = change.settles();
        let effect = match change {
            Change::Picker(event) => self.edit_color(event),
            Change::Emoji(event) => {
                if let Some(emoji) = self.emojis.update(event)
                    && let Some(op) =
                        kirie_annotate::marks::restick(self.document(), &self.picked, &emoji)
                {
                    self.history.apply(op);
                }
                Effect::default()
            }
            Change::Choose(choice) => {
                let refocus = self
                    .typing
                    .as_mut()
                    .is_some_and(|typing| typing.choose(choice, &mut self.emojis));
                Effect {
                    focus: refocus,
                    ..Effect::default()
                }
            }
            Change::TextEdit(action) => {
                if let Some(typing) = &mut self.typing {
                    typing.act(action, self.emojis.tone());
                }
                Effect::default()
            }
            Change::TextDone => {
                self.commit_text();
                Effect::default()
            }
            Change::Undo => {
                self.travel(true);
                Effect::default()
            }
            Change::Redo => {
                self.travel(false);
                Effect::default()
            }
            Change::Delete => {
                if !self.picked.is_empty() {
                    let ids = std::mem::take(&mut self.picked);
                    self.history.apply(Op::Remove { ids });
                }
                Effect::default()
            }
            Change::Duplicate => {
                if let Some((op, ids)) =
                    kirie_annotate::marks::duplicate(self.document(), &self.picked)
                {
                    self.history.apply(op);
                    self.picked = ids;
                }
                Effect::default()
            }
            Change::Restack(to) => {
                if let Some(op) = kirie_annotate::marks::restack(self.document(), &self.picked, to)
                {
                    self.history.apply(op);
                }
                Effect::default()
            }
            Change::PickStyle(_)
            | Change::ToggleShadow
            | Change::PickColor(_)
            | Change::EditColor(_)
            | Change::Weight(_)
            | Change::ToggleFill
            | Change::Brightness(_)
            | Change::Spot(_)
            | Change::Look(_) => {
                self.change_style(&change);
                Effect::default()
            }
        };
        Effect { settled, ..effect }
    }

    /// Changes to the style, colors, weight, fill and text look.
    fn change_style(&mut self, change: &Change) {
        match *change {
            Change::PickStyle(style) => {
                let before = self.style();
                let recolor = self.swatches.recolor(before, style);
                self.history.apply(Op::SetStyle { style, recolor });
                self.follow_style(before);
            }
            Change::ToggleShadow => {
                let shadow = !self.document().shadow;
                self.history.apply(Op::SetShadow(shadow));
            }
            Change::PickColor(color) => self.repaint(|paint| paint.color = color),
            Change::EditColor(target) => {
                self.picker = Some(Picker::new(
                    target,
                    self.style(),
                    &self.swatches,
                    self.paint.color,
                ));
            }
            Change::Weight(WeightEvent::Done)
            | Change::Brightness(BrightnessEvent::Done)
            | Change::Look(TextEvent::SizeDone) => self.sliding = false,
            Change::Brightness(BrightnessEvent::Slide(brightness)) => {
                self.adjust(Op::SetSpotlightBrightness(brightness), true);
            }
            Change::Weight(WeightEvent::Pick(weight)) => {
                self.repaint(|paint| paint.weight = weight);
            }
            Change::Weight(WeightEvent::Slide(width)) => {
                self.repaint_gradually(|paint| paint.weight = Weight::new(width), true);
            }
            Change::ToggleFill => self.repaint(|paint| paint.filled = !paint.filled),
            Change::Spot(spot) => self.repaint(|paint| paint.spot = spot),
            Change::Look(event) => {
                self.repaint_gradually(
                    |paint| paint.text = event.apply(paint.text),
                    event.gradual(),
                );
            }
            Change::Picker(_)
            | Change::Emoji(_)
            | Change::Choose(_)
            | Change::TextEdit(_)
            | Change::TextDone
            | Change::Undo
            | Change::Redo
            | Change::Delete
            | Change::Duplicate
            | Change::Restack(_) => {}
        }
    }

    /// Hands `event` to the color picker, keeping the colors when they change.
    pub fn edit_color(&mut self, event: picker::Event) -> Effect {
        let picker::Outcome::Changed { pick } =
            picker::update(&mut self.picker, &mut self.swatches, event)
        else {
            return Effect::default();
        };
        if let Some(color) = pick {
            self.repaint(|paint| paint.color = color);
        }
        Effect {
            error: self.swatches.save().err(),
            ..Effect::default()
        }
    }

    /// Applies `op`; a `gradual` one joins the step the slider drag began.
    pub fn adjust(&mut self, op: Op, gradual: bool) {
        if gradual {
            self.history.apply_gradually(op, self.sliding);
        } else {
            self.history.apply(op);
        }
        self.sliding = gradual;
    }

    /// Readies drawing with `tool`: the words being typed are kept, and new marks take the
    /// tool's color.
    pub fn pick_tool(&mut self, tool: Option<Tool>) {
        self.commit_text();
        if tool != Some(Tool::Select) {
            self.picked.clear();
        }
        self.paint.color =
            controls::tool_color(tool, self.paint.color, self.swatches.first(self.style()));
        self.draft = None;
    }

    /// Puts down one thing at a time: the emoji offered while typing, the color picker, the
    /// words being typed, the mark being drawn, then the picked marks. `None` once there was
    /// nothing to put down.
    pub fn put_down(&mut self) -> Option<Effect> {
        if let Some(typing) = &mut self.typing
            && typing.dismiss()
        {
            return Some(Effect::FOCUS);
        }
        (self.picker.take().is_some()
            || self.typing.take().is_some()
            || self.draft.take().is_some()
            || !std::mem::take(&mut self.picked).is_empty())
        .then(Effect::default)
    }

    /// Whether the eyedropper waits for a click on the picture.
    pub fn sampling(&self) -> bool {
        self.picker.as_ref().is_some_and(Picker::sampling)
    }

    /// Undoes or redoes a step; the words being typed become a step of their own first, and the
    /// drawing color follows the style there.
    fn travel(&mut self, undo: bool) {
        self.commit_text();
        let before = self.style();
        if undo {
            self.history.undo();
        } else {
            self.history.redo();
        }
        kirie_annotate::marks::prune(self.history.current(), &mut self.picked);
        self.follow_style(before);
    }

    /// Keeps drawing in the standard color drawn with before the style changed from `before`,
    /// in the new style's shade of it.
    pub fn follow_style(&mut self, before: StyleId) {
        let now = self.style();
        if now != before {
            let recolor = self.swatches.recolor(before, now);
            self.paint.color = controls::restyled(self.paint.color, &recolor);
        }
    }

    /// Changes the paint for new marks, and the picked mark's paint too.
    fn repaint(&mut self, change: impl Fn(&mut Paint)) {
        self.repaint_gradually(change, false);
    }

    /// Like [`Self::repaint`]; a `gradual` change joins the step the slider drag began.
    fn repaint_gradually(&mut self, change: impl Fn(&mut Paint), gradual: bool) {
        change(&mut self.paint);
        if let Some(op) = kirie_annotate::marks::repaint(self.document(), &self.picked, change) {
            self.adjust(op, gradual);
        }
    }

    /// A press of `tool` at `at` on the picture; `click` is where, to tell a double click, and
    /// `reach` how close to a mark picks it, both as the picture is shown.
    pub fn press(&mut self, tool: Tool, at: kurbo::Point, click: Point, reach: f64) -> Effect {
        self.commit_text();
        let double = typing::double_click(&mut self.last_click, click);
        let words = match tool {
            Tool::Text | Tool::Bubble => typing::words_at(self.document(), at, reach),
            Tool::Select if double => typing::words_at(self.document(), at, reach),
            _ => None,
        };
        if let Some(id) = words {
            return self.edit_text(id);
        }
        let press = kirie_annotate::tools::press(
            tool,
            at,
            self.document(),
            self.paint,
            reach,
            &self.picked,
            self.extending(),
        );
        match press {
            Press::Draft(draft) => {
                kirie_annotate::marks::pressed(&draft, &mut self.picked);
                self.draft = Some(draft);
            }
            Press::Done(op) => {
                self.history.apply(op);
            }
            Press::TypeAt(at) => {
                self.typing = Some(Typing::new(at));
                return Effect::FOCUS;
            }
            Press::PlaceSticker(at) => {
                let size = kirie_annotate::tools::STICKER_SIZE * self.document().unit;
                let draft = Draft::sticker(self.emojis.chosen(), at, size);
                kirie_annotate::marks::pressed(&draft, &mut self.picked);
                self.draft = Some(draft);
            }
            Press::Toggle(id) => kirie_annotate::marks::toggle(&mut self.picked, id),
            Press::Nothing => self.picked.clear(),
        }
        Effect::default()
    }

    /// The pointer moved to `at` on the picture.
    pub fn drag(&mut self, at: kurbo::Point) {
        if let Some(draft) = self.draft.take() {
            self.draft = Some(draft.drag(at, self.modifiers.shift()));
        }
    }

    /// The pointer let go at `at` on the picture.
    pub fn release(&mut self, at: kurbo::Point) -> Effect {
        let Some(draft) = self.draft.take() else {
            return Effect::default();
        };
        let draft = draft.drag(at, self.modifiers.shift());
        if let Some((center, tail)) = draft.bubble() {
            self.typing = Some(Typing::bubble(center, tail));
            return Effect::FOCUS;
        }
        let document = self.history.current();
        if let Some(op) =
            kirie_annotate::marks::release(draft, document, &mut self.picked, self.paint)
        {
            self.history.apply(op);
        }
        Effect::default()
    }

    /// Puts the words being typed on the picture.
    pub fn commit_text(&mut self) {
        if let Some(op) = self
            .typing
            .take()
            .and_then(|typing| typing.finish(self.paint))
        {
            self.history.apply(op);
        }
    }

    /// Opens the words of a placed text mark for typing.
    pub fn edit_text(&mut self, id: ShapeId) -> Effect {
        let Some(typing) = self.document().shape(id).and_then(Typing::edit) else {
            return Effect::default();
        };
        self.typing = Some(typing);
        self.picked = vec![id];
        self.draft = None;
        Effect::FOCUS
    }

    /// The paint the controls show: the first picked mark's, else the one new marks get.
    pub fn shown_paint(&self) -> Paint {
        kirie_annotate::marks::shown_paint(self.document(), &self.picked, self.paint)
    }

    /// Whether the text controls apply: to words being typed, picked text marks and bubbles, or
    /// the next.
    pub fn texting(&self, tool: Option<Tool>) -> bool {
        matches!(tool, Some(Tool::Text | Tool::Bubble))
            || self.typing.is_some()
            || kirie_annotate::marks::all(self.document(), &self.picked, |kind| {
                matches!(kind, Kind::Text { .. } | Kind::Bubble { .. })
            })
    }

    /// Whether the emoji picker shows: for the sticker tool or picked stickers.
    pub fn stickering(&self, tool: Option<Tool>) -> bool {
        kirie_annotate::marks::stickering(tool, self.document(), &self.picked)
    }

    /// Whether the spotlight controls apply: to the spotlight tool or picked spotlights.
    pub fn spotlighting(&self, tool: Option<Tool>) -> bool {
        tool == Some(Tool::Spotlight)
            || kirie_annotate::marks::all(self.document(), &self.picked, |kind| {
                matches!(kind, Kind::Spotlight { .. })
            })
    }

    /// Whether a press adds to the picked marks instead of picking anew.
    fn extending(&self) -> bool {
        self.modifiers.shift() || self.modifiers.command()
    }

    /// The mark being drawn or typed, as it will be once done.
    pub fn preview(&self) -> Option<Shape> {
        let document = self.document();
        self.draft
            .as_ref()
            .and_then(Draft::preview)
            .map(|kind| Shape {
                id: document.next_id(),
                kind,
                paint: self.paint,
                style: None,
                rotation: 0.0,
            })
            .or_else(|| {
                self.typing
                    .as_ref()
                    .map(|typing| typing.preview(document, self.paint))
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn annotator() -> Annotator {
        Annotator::new(
            Document::new(200, 100, 1.0),
            Paint::default(),
            Swatches::default(),
        )
    }

    fn draw_box(annotator: &mut Annotator) {
        let start = kurbo::Point::new(10.0, 10.0);
        let end = kurbo::Point::new(60.0, 50.0);
        let _ = annotator.press(Tool::Rectangle, start, Point::new(10.0, 10.0), 6.0);
        annotator.drag(end);
        let _ = annotator.release(end);
    }

    #[test]
    fn a_drag_draws_a_mark_that_undo_takes_back() {
        let mut annotator = annotator();
        draw_box(&mut annotator);
        assert_eq!(annotator.document().shapes().count(), 1);
        let _ = annotator.update(Change::Undo);
        assert_eq!(annotator.document().shapes().count(), 0);
        let _ = annotator.update(Change::Redo);
        assert_eq!(annotator.document().shapes().count(), 1);
    }

    #[test]
    fn a_slider_drag_undoes_as_one_step_and_settles_when_let_go() {
        let mut annotator = annotator();
        for brightness in [0.2, 0.3, 0.4] {
            let effect = annotator.update(Change::Brightness(BrightnessEvent::Slide(brightness)));
            assert!(!effect.settled);
        }
        assert!(
            annotator
                .update(Change::Brightness(BrightnessEvent::Done))
                .settled
        );
        let _ = annotator.update(Change::Undo);
        assert_eq!(
            annotator.document().spotlight_brightness,
            Document::SPOTLIGHT_BRIGHTNESS
        );
    }

    #[test]
    fn picking_another_tool_lets_go_of_the_picked_marks() {
        let mut annotator = annotator();
        draw_box(&mut annotator);
        annotator.picked = annotator
            .document()
            .shapes()
            .map(|shape| shape.id)
            .collect();
        annotator.pick_tool(Some(Tool::Select));
        assert_eq!(annotator.picked.len(), 1);
        annotator.pick_tool(Some(Tool::Arrow));
        assert!(annotator.picked.is_empty());
    }

    #[test]
    fn putting_down_goes_one_thing_at_a_time() {
        let mut annotator = annotator();
        draw_box(&mut annotator);
        annotator.picked = annotator
            .document()
            .shapes()
            .map(|shape| shape.id)
            .collect();
        annotator.typing = Some(Typing::new(kurbo::Point::new(5.0, 5.0)));
        assert_eq!(annotator.put_down(), Some(Effect::default()));
        assert!(annotator.typing.is_none());
        assert!(!annotator.picked.is_empty());
        assert_eq!(annotator.put_down(), Some(Effect::default()));
        assert!(annotator.picked.is_empty());
        assert_eq!(annotator.put_down(), None);
    }

    #[test]
    fn undo_while_editing_words_takes_back_the_typing_and_redo_brings_it_back() {
        use iced::widget::text_editor::{Action, Edit};

        let text = |annotator: &Annotator| {
            annotator
                .document()
                .shapes()
                .find_map(|shape| match &shape.kind {
                    Kind::Text { text, .. } => Some(text.clone()),
                    _ => None,
                })
        };
        let mut annotator = annotator();
        annotator.history.apply(Op::Add {
            kind: Kind::Text {
                at: kurbo::Point::new(10.0, 10.0),
                text: "Hi".to_owned(),
            },
            paint: Paint::default(),
        });
        let id = annotator.document().shapes().next().unwrap().id;
        let _ = annotator.edit_text(id);
        let _ = annotator.update(Change::TextEdit(Action::Edit(Edit::Insert('!'))));
        let _ = annotator.update(Change::Undo);
        assert!(annotator.typing.is_none());
        assert_eq!(text(&annotator).as_deref(), Some("Hi"));
        let _ = annotator.update(Change::Redo);
        assert_eq!(text(&annotator).as_deref(), Some("Hi!"));
    }

    #[test]
    fn deleting_removes_only_the_picked_marks() {
        let mut annotator = annotator();
        draw_box(&mut annotator);
        let _ = annotator.update(Change::Delete);
        assert_eq!(annotator.document().shapes().count(), 1);
        annotator.picked = annotator
            .document()
            .shapes()
            .map(|shape| shape.id)
            .collect();
        let _ = annotator.update(Change::Delete);
        assert_eq!(annotator.document().shapes().count(), 0);
    }
}
