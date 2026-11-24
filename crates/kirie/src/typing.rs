//! Typing the words of a text mark or speech bubble, new or already placed, in a box that takes
//! several lines: Enter places the text and Shift+Enter starts a new line. A colon starts an
//! emoji: `:sm` offers the emoji whose names start so, and `:smile:` becomes one. The button
//! beside the box opens the emoji picker under it, for emoji whose names you do not know.

use std::sync::Arc;

use std::ops::Range;

use iced::Element;
use iced::advanced::mouse;
use iced::advanced::text::{Highlighter, highlighter::Format};
use iced::keyboard::{Key, key::Named};
use iced::widget::text_editor::{self, Action, Binding, Content, Edit, Motion};
use iced::widget::{button, column, container, row, svg, text};
use kirie_annotate::emoji::{self, Emoji, SkinTone};
use kirie_annotate::{Document, Kind, Op, Paint, Shape, ShapeId};
use kirie_ui::icon::Icon;
use kirie_ui::{Palette, radius, space};

use crate::emoji_picker::{self, EmojiPicker};

/// The most emoji offered for a name typed after a colon.
const OFFERED: usize = 6;
/// The letters a name needs before emoji are offered for it.
const LETTERS: usize = 2;
const WIDTH: f32 = 300.0;
/// The side of the button that opens the emoji picker.
const BROWSE: f32 = 32.0;

/// Text being typed at `at`, for a new mark or in place of the words of `editing`. With a
/// `tail`, the words go in a speech bubble centered on `at`.
#[derive(Debug)]
pub struct Typing {
    pub at: kurbo::Point,
    pub tail: Option<kurbo::Point>,
    pub content: Content,
    pub editing: Option<ShapeId>,
    /// Emoji offered for the name typed after a colon.
    pub offer: Option<Offer>,
    /// Whether the emoji picker shows under the box.
    pub browsing: bool,
}

/// Emoji whose names start with what follows the colon before the cursor.
#[derive(Debug, Clone, PartialEq)]
pub struct Offer {
    /// The name typed so far, without its colon.
    pub name: String,
    pub emoji: Vec<&'static Emoji>,
    pub selected: usize,
}

/// What a key or click does to the emoji offered or to the emoji picker under the box.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    Previous,
    Next,
    Pick(usize),
    /// Opens or closes the emoji picker.
    Browse,
    Picker(emoji_picker::Event),
}

impl Typing {
    pub fn new(at: kurbo::Point) -> Self {
        Self {
            at,
            tail: None,
            content: Content::new(),
            editing: None,
            offer: None,
            browsing: false,
        }
    }

    /// Typing for a new speech bubble centered on `center` whose tail points at `tail`.
    pub fn bubble(center: kurbo::Point, tail: kurbo::Point) -> Self {
        Self {
            tail: Some(tail),
            ..Self::new(center)
        }
    }

    /// Typing that changes the words of `shape`, if it is a text mark or a speech bubble.
    pub fn edit(shape: &Shape) -> Option<Self> {
        let (at, tail, text) = match &shape.kind {
            Kind::Text { at, text } => (*at, None, text),
            Kind::Bubble { center, tail, text } => (*center, Some(*tail), text),
            _ => return None,
        };
        let mut content = Content::with_text(text);
        content.perform(text_editor::Action::Move(Motion::DocumentEnd));
        Some(Self {
            at,
            tail,
            content,
            editing: Some(shape.id),
            offer: None,
            browsing: false,
        })
    }

    /// Applies an edit or a move of the cursor, then turns a finished `:name:` into its emoji in
    /// `tone` and offers emoji for a name being typed.
    pub fn act(&mut self, action: Action, tone: SkinTone) {
        let colon = action == Action::Edit(Edit::Insert(':'));
        self.content.perform(action);
        if colon && self.finish_name(tone) {
            self.offer = None;
            return;
        }
        self.offer = self.offered();
    }

    /// Moves through the emoji offered or picks one, or acts in the emoji picker; an emoji
    /// picked is typed at the cursor and remembered in `emojis`. Whether the box takes the
    /// keyboard back, which it does unless the picker's search has it.
    pub fn choose(&mut self, choice: Choice, emojis: &mut EmojiPicker) -> bool {
        match choice {
            Choice::Previous | Choice::Next => {
                if let Some(offer) = self.offer.as_mut() {
                    let count = offer.emoji.len();
                    let step = if choice == Choice::Next { 1 } else { count - 1 };
                    offer.selected = (offer.selected + step) % count;
                }
            }
            Choice::Pick(index) => {
                if let Some(offer) = self.offer.take()
                    && let Some(picked) = offer.emoji.get(index)
                {
                    let picked = emojis.toned(picked).as_str();
                    self.replace_back(offer.name.chars().count() + 1, picked);
                    emojis.remember(picked);
                }
            }
            Choice::Browse => self.browsing = !self.browsing,
            Choice::Picker(event) => {
                let searching = matches!(event, emoji_picker::Event::Search(_));
                if let Some(picked) = emojis.update(event) {
                    self.content
                        .perform(Action::Edit(Edit::Paste(Arc::new(picked))));
                    self.offer = self.offered();
                }
                return !searching;
            }
        }
        true
    }

    /// Closes the emoji offered or the emoji picker, whichever shows; whether one did.
    pub fn dismiss(&mut self) -> bool {
        self.offer.take().is_some() || std::mem::take(&mut self.browsing)
    }

    /// The text of the cursor's line before it, unless text is selected.
    fn before_cursor(&self) -> Option<String> {
        let cursor = self.content.cursor();
        if cursor.selection.is_some() {
            return None;
        }
        let line = self.content.line(cursor.position.line)?;
        line.text.get(..cursor.position.column).map(str::to_owned)
    }

    fn offered(&self) -> Option<Offer> {
        let before = self.before_cursor()?;
        let name = name_before(&before)?;
        if name.chars().count() < LETTERS {
            return None;
        }
        let found = emoji::search(name, OFFERED);
        (!found.is_empty()).then(|| Offer {
            name: name.to_owned(),
            emoji: found,
            selected: 0,
        })
    }

    /// Turns the `:name:` just closed into its emoji in `tone`, if there is one by that name.
    fn finish_name(&mut self, tone: SkinTone) -> bool {
        let Some(before) = self.before_cursor() else {
            return false;
        };
        let Some(name) = before.strip_suffix(':').and_then(name_before) else {
            return false;
        };
        let Some(found) = emoji::by_shortcode(name) else {
            return false;
        };
        self.replace_back(name.chars().count() + 2, emoji::toned(found, tone).as_str());
        true
    }

    /// Puts `with` in place of the `count` characters before the cursor.
    fn replace_back(&mut self, count: usize, with: &str) {
        for _ in 0..count {
            self.content.perform(Action::Select(Motion::Left));
        }
        self.content
            .perform(Action::Edit(Edit::Paste(Arc::new(with.to_owned()))));
    }

    fn kind(&self, text: String) -> Kind {
        match self.tail {
            Some(tail) => Kind::Bubble {
                center: self.at,
                tail,
                text,
            },
            None => Kind::Text { at: self.at, text },
        }
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    /// The mark as typed so far, to draw in place of the one being edited; `paint` is for a
    /// new mark.
    pub fn preview(&self, document: &Document, paint: Paint) -> Shape {
        let text = self.text();
        let kind = self.kind(if text.is_empty() {
            "|".to_owned()
        } else {
            text
        });
        let edited = self.editing.and_then(|id| document.shape(id));
        match edited {
            Some(shape) => Shape {
                kind,
                ..shape.clone()
            },
            None => Shape {
                id: document.next_id(),
                kind,
                paint,
                style: None,
                rotation: 0.0,
            },
        }
    }

    /// The edit that places the text: a new mark, new words for the edited one, or none.
    /// Clearing the words of a placed mark removes it.
    pub fn finish(self, paint: Paint) -> Option<Op> {
        let text = self.text();
        let blank = text.trim().is_empty();
        let kind = self.kind(text);
        match self.editing {
            Some(id) if blank => Some(Op::Remove { ids: vec![id] }),
            Some(id) => Some(Op::Replace { id, kind }),
            None if blank => None,
            None => Some(Op::Add { kind, paint }),
        }
    }
}

/// The emoji name being typed at the end of `before`: what follows a colon that starts a word.
fn name_before(before: &str) -> Option<&str> {
    let colon = before.rfind(':')?;
    let name = &before[colon + 1..];
    let starts_word = before[..colon]
        .chars()
        .next_back()
        .is_none_or(char::is_whitespace);
    let named = !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '+'));
    (starts_word && named).then_some(name)
}

/// The text mark or speech bubble under `at`, within `reach`.
pub fn words_at(document: &Document, at: kurbo::Point, reach: f64) -> Option<ShapeId> {
    document.hit(at, reach).filter(|id| {
        matches!(
            document.shape(*id).map(|shape| &shape.kind),
            Some(Kind::Text { .. } | Kind::Bubble { .. })
        )
    })
}

/// Whether a press makes a double click with the one before; keeps it for the next.
pub fn double_click(last: &mut Option<mouse::Click>, at: iced::Point) -> bool {
    let click = mouse::Click::new(at, mouse::Button::Left, *last);
    *last = Some(click);
    click.kind() == mouse::click::Kind::Double
}

/// The box the words are typed in, with the emoji offered for a name under it and, while
/// browsing, the emoji picker. Pasting sends `on_paste` when given, for a caller that reads the
/// clipboard itself. While emoji are offered, Up and Down move through them and Tab or Enter
/// picks one.
#[allow(clippy::too_many_arguments)]
pub fn text_box<'a, Message: Clone + 'a>(
    typing: &'a Typing,
    id: &'static str,
    emojis: &'a EmojiPicker,
    palette: Palette,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
    on_done: Message,
    on_paste: Option<Message>,
    on_choice: impl Fn(Choice) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    let offered = typing.offer.as_ref().map(|offer| offer.selected);
    let choose = on_choice.clone();
    let editor = iced::widget::text_editor(&typing.content)
        .id(id)
        .placeholder("Type, then Enter; : for emoji")
        .on_action(on_action)
        .highlight_with::<EmojiFace>((), |(), _| Format {
            color: None,
            font: Some(emoji_picker::FONT),
        })
        .key_binding(move |press| {
            if let Some(selected) = offered {
                let choice = match press.key {
                    Key::Named(Named::ArrowUp) => Some(Choice::Previous),
                    Key::Named(Named::ArrowDown) => Some(Choice::Next),
                    Key::Named(Named::Tab) => Some(Choice::Pick(selected)),
                    Key::Named(Named::Enter) if !press.modifiers.shift() => {
                        Some(Choice::Pick(selected))
                    }
                    _ => None,
                };
                if let Some(choice) = choice {
                    return Some(Binding::Custom(choose(choice)));
                }
            }
            if press.key == Key::Named(Named::Enter) && !press.modifiers.shift() {
                return Some(Binding::Custom(on_done.clone()));
            }
            match (Binding::from_key_press(press), &on_paste) {
                (Some(Binding::Paste), Some(on_paste)) => Some(Binding::Custom(on_paste.clone())),
                (binding, _) => binding,
            }
        })
        .width(WIDTH - BROWSE - space::XS)
        .max_height(240)
        .size(16)
        .padding([space::XS + 2.0, space::S])
        .style(move |_, _| text_editor::Style {
            background: iced::Background::Color(palette.surface[0].scale_alpha(0.92)),
            border: iced::Border {
                color: palette.accent,
                width: 1.5,
                radius: radius::CONTROL.into(),
            },
            placeholder: palette.muted,
            value: palette.text,
            selection: palette.accent.scale_alpha(0.4),
        });
    let browse = kirie_ui::widget::hint(
        browse_button(typing.browsing, palette, on_choice(Choice::Browse)),
        "Emoji",
        palette,
    );
    let offered = typing
        .offer
        .as_ref()
        .map(|offer| offers(offer, emojis, palette, on_choice.clone()));
    let picker = typing.browsing.then(|| {
        panel(
            emojis
                .view("", palette, iced::Length::Fill)
                .map(move |event| on_choice(Choice::Picker(event))),
            palette,
        )
    });
    // The editor stays the first child of the same row and column whatever shows under it, so
    // the tree keeps its state, focus included, as the offer and the picker come and go.
    column![row![editor, browse].spacing(space::XS)]
        .push(offered)
        .push(picker)
        .spacing(space::XS)
        .width(WIDTH)
        .into()
}

/// The button that opens the emoji picker, lit while it is open.
fn browse_button<'a, Message: Clone + 'a>(
    open: bool,
    palette: Palette,
    on_press: Message,
) -> Element<'a, Message> {
    let tint = if open { palette.accent } else { palette.text };
    let icon = svg(Icon::Sticker.handle())
        .width(18)
        .height(18)
        .style(move |_, _| svg::Style { color: Some(tint) });
    button(container(icon).center(iced::Length::Fill))
        .width(BROWSE)
        .height(BROWSE)
        .padding(0)
        .on_press(on_press)
        .style(move |_, status| {
            let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
            button::Style {
                background: Some(iced::Background::Color(if open {
                    palette.accent.scale_alpha(0.2)
                } else if hovered {
                    palette.surface[1]
                } else {
                    palette.surface[0].scale_alpha(0.92)
                })),
                border: iced::Border {
                    color: if open { palette.accent } else { palette.border },
                    width: 1.0,
                    radius: radius::CONTROL.into(),
                },
                ..button::Style::default()
            }
        })
        .into()
}

/// `content` on the opaque card the emoji offered and the picker sit on.
fn panel<'a, Message: 'a>(
    content: impl Into<Element<'a, Message>>,
    palette: Palette,
) -> Element<'a, Message> {
    container(content)
        .padding(space::XS)
        .width(iced::Length::Fill)
        .style(move |_| container::Style {
            background: Some(iced::Background::Color(
                palette.surface[0].scale_alpha(0.96),
            )),
            border: iced::Border {
                color: palette.border,
                width: 1.0,
                radius: radius::CONTROL.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Puts the emoji typed in the box in the bundled emoji face: left to its fallback, the text
/// engine may take a system emoji face it cannot draw.
#[derive(Debug, Default)]
struct EmojiFace {
    line: usize,
}

impl Highlighter for EmojiFace {
    type Settings = ();
    type Highlight = ();
    type Iterator<'a> = std::vec::IntoIter<(Range<usize>, ())>;

    fn new((): &()) -> Self {
        Self::default()
    }

    fn update(&mut self, (): &()) {
        self.line = 0;
    }

    fn change_line(&mut self, line: usize) {
        self.line = self.line.min(line);
    }

    fn highlight_line(&mut self, line: &str) -> Self::Iterator<'_> {
        self.line += 1;
        emoji_spans(line)
            .into_iter()
            .map(|span| (span, ()))
            .collect::<Vec<_>>()
            .into_iter()
    }

    fn current_line(&self) -> usize {
        self.line
    }
}

/// The byte ranges of `line` that are emoji: pictographs with their joiners, variation
/// selectors, skin tones and flag letters.
fn emoji_spans(line: &str) -> Vec<Range<usize>> {
    let mut spans: Vec<Range<usize>> = Vec::new();
    for (at, c) in line.char_indices() {
        if !is_emoji_part(c) {
            continue;
        }
        let end = at + c.len_utf8();
        match spans.last_mut() {
            Some(last) if last.end == at => last.end = end,
            _ => spans.push(at..end),
        }
    }
    spans
}

fn is_emoji_part(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1f000..=0x1faff
            | 0x2300..=0x23ff
            | 0x2600..=0x27bf
            | 0x2b00..=0x2bff
            | 0x2190..=0x21ff
            | 0x25a0..=0x25ff
            | 0xe0020..=0xe007f
            | 0x200d
            | 0x20e3
            | 0xfe0f
            | 0x00a9
            | 0x00ae
            | 0x203c
            | 0x2049
            | 0x2122
            | 0x2139
            | 0x3030
            | 0x303d
            | 0x3297
            | 0x3299
    )
}

/// The emoji offered for a name, in the skin tone chosen, one per row with its name, the
/// selected one highlighted.
fn offers<'a, Message: Clone + 'a>(
    offer: &Offer,
    emojis: &EmojiPicker,
    palette: Palette,
    on_choice: impl Fn(Choice) -> Message + 'a,
) -> Element<'a, Message> {
    let rows = offer.emoji.iter().enumerate().map(|(index, found)| {
        let selected = index == offer.selected;
        let name = emoji::shortcode_for(found, &offer.name);
        button(
            row![
                text(emojis.toned(found).as_str())
                    .font(emoji_picker::FONT)
                    .size(18)
                    .line_height(1.0),
                container(
                    text(format!(":{name}:"))
                        .size(13)
                        .wrapping(text::Wrapping::None)
                )
                .width(iced::Length::Fill)
                .clip(true),
            ]
            .spacing(space::S)
            .align_y(iced::Alignment::Center),
        )
        .width(iced::Length::Fill)
        .padding([2.0, space::S])
        .on_press(on_choice(Choice::Pick(index)))
        .style(move |_, status| button::Style {
            background: (selected || status == button::Status::Hovered).then(|| {
                iced::Background::Color(if selected {
                    palette.accent.scale_alpha(0.35)
                } else {
                    palette.surface[1]
                })
            }),
            text_color: palette.text,
            border: iced::Border {
                radius: radius::CONTROL.into(),
                ..iced::Border::default()
            },
            ..button::Style::default()
        })
        .into()
    });
    panel(column(rows).spacing(2), palette)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kurbo::Point;

    fn with_text() -> (Document, ShapeId) {
        let document = Document::new(300, 200, 1.0);
        let id = document.next_id();
        let document = document.apply(Op::Add {
            kind: Kind::Text {
                at: Point::new(20.0, 20.0),
                text: "Hello".into(),
            },
            paint: Paint::default(),
        });
        (document, id)
    }

    #[test]
    fn new_text_is_added_unless_blank() {
        assert_eq!(Typing::new(Point::ORIGIN).finish(Paint::default()), None);
        let mut typing = Typing::new(Point::new(5.0, 5.0));
        typing.content = Content::with_text("two\nlines");
        let Some(Op::Add {
            kind: Kind::Text { text, .. },
            ..
        }) = typing.finish(Paint::default())
        else {
            panic!()
        };
        assert_eq!(text, "two\nlines");
    }

    #[test]
    fn editing_replaces_the_words_or_removes_the_mark() {
        let (document, id) = with_text();
        let shape = document.shape(id).unwrap();
        let typing = Typing::edit(shape).unwrap();
        assert_eq!(typing.text(), "Hello");
        assert_eq!(typing.preview(&document, Paint::default()).id, id);
        let mut changed = Typing::edit(shape).unwrap();
        changed.content = Content::with_text("Hi\nthere");
        assert!(matches!(
            changed.finish(Paint::default()),
            Some(Op::Replace { id: replaced, kind: Kind::Text { ref text, .. } })
                if replaced == id && text == "Hi\nthere"
        ));
        let mut cleared = Typing::edit(shape).unwrap();
        cleared.content = Content::with_text("  ");
        assert_eq!(
            cleared.finish(Paint::default()),
            Some(Op::Remove { ids: vec![id] })
        );
    }

    #[test]
    fn only_text_marks_are_edited() {
        let (document, id) = with_text();
        assert_eq!(words_at(&document, Point::new(30.0, 30.0), 4.0), Some(id));
        assert_eq!(words_at(&document, Point::new(250.0, 150.0), 4.0), None);
        let boxed = document.apply(Op::Add {
            kind: Kind::Rectangle {
                rect: kurbo::Rect::new(100.0, 100.0, 200.0, 150.0),
            },
            paint: Paint::default(),
        });
        assert_eq!(words_at(&boxed, Point::new(150.0, 120.0), 4.0), None);
        assert!(Typing::edit(boxed.shapes().last().unwrap()).is_none());
    }

    #[test]
    fn bubbles_keep_their_tail_through_typing() {
        let (center, tail) = (Point::new(100.0, 40.0), Point::new(20.0, 120.0));
        let mut typing = Typing::bubble(center, tail);
        assert!(matches!(
            typing.preview(&Document::new(300, 200, 1.0), Paint::default()).kind,
            Kind::Bubble { text, .. } if text == "|"
        ));
        typing.content = Content::with_text("Look here");
        let Some(Op::Add { kind, .. }) = typing.finish(Paint::default()) else {
            panic!()
        };
        assert_eq!(
            kind,
            Kind::Bubble {
                center,
                tail,
                text: "Look here".into()
            }
        );
        let document = Document::new(300, 200, 1.0).apply(Op::Add {
            kind,
            paint: Paint::default(),
        });
        let id = words_at(&document, center, 4.0).unwrap();
        let mut editing = Typing::edit(document.shape(id).unwrap()).unwrap();
        assert_eq!(
            (editing.text(), editing.tail),
            ("Look here".into(), Some(tail))
        );
        editing.content = Content::with_text("Moved on");
        assert!(matches!(
            editing.finish(Paint::default()),
            Some(Op::Replace { kind: Kind::Bubble { tail: kept, .. }, .. }) if kept == tail
        ));
    }

    fn typed(typing: &mut Typing, words: &str) {
        typed_in(typing, words, SkinTone::Default);
    }

    fn typed_in(typing: &mut Typing, words: &str, tone: SkinTone) {
        for c in words.chars() {
            typing.act(Action::Edit(Edit::Insert(c)), tone);
        }
    }

    #[test]
    fn a_colon_and_letters_offer_emoji() {
        let mut typing = Typing::new(Point::ORIGIN);
        typed(&mut typing, "Ship it :s");
        assert_eq!(typing.offer, None, "one letter is not enough");
        typed(&mut typing, "mile");
        let offer = typing.offer.clone().unwrap();
        assert_eq!(offer.name, "smile");
        assert_eq!(offer.emoji[0].as_str(), "\u{1f604}");
        let mut emojis = EmojiPicker::default();
        typing.choose(Choice::Next, &mut emojis);
        typing.choose(Choice::Previous, &mut emojis);
        typing.choose(Choice::Previous, &mut emojis);
        assert_eq!(
            typing.offer.as_ref().unwrap().selected,
            offer.emoji.len() - 1
        );
        assert!(typing.choose(Choice::Pick(0), &mut emojis));
        assert_eq!(
            emojis.chosen(),
            "\u{1f604}",
            "the emoji picked is remembered"
        );
        assert_eq!(typing.text(), "Ship it \u{1f604}");
        assert_eq!(typing.offer, None);
        typed(&mut typing, "!");
        assert_eq!(
            typing.text(),
            "Ship it \u{1f604}!",
            "the cursor stays after the emoji"
        );
    }

    #[test]
    fn a_closed_name_becomes_its_emoji() {
        let mut typing = Typing::new(Point::ORIGIN);
        typed(&mut typing, "Go :rocket: now");
        assert_eq!(typing.text(), "Go \u{1f680} now");
        let mut unknown = Typing::new(Point::ORIGIN);
        typed(&mut unknown, ":nothing_like_it: ");
        assert_eq!(unknown.text(), ":nothing_like_it: ");
    }

    #[test]
    fn colons_inside_words_offer_nothing() {
        let mut typing = Typing::new(Point::ORIGIN);
        typed(&mut typing, "at 12:30 or http:sm");
        assert_eq!(typing.offer, None);
        typed(&mut typing, " :thu");
        assert!(typing.offer.is_some());
        typed(&mut typing, " ");
        assert_eq!(typing.offer, None, "a space ends the name");
        typing.choose(Choice::Pick(0), &mut EmojiPicker::default());
        assert_eq!(typing.text(), "at 12:30 or http:sm :thu ");
    }

    #[test]
    fn emoji_come_in_the_skin_tone_chosen() {
        let mut emojis = EmojiPicker::default();
        emojis.update(emoji_picker::Event::Tone(SkinTone::MediumDark));
        let mut typing = Typing::new(Point::ORIGIN);
        typed_in(&mut typing, ":+1: :wave", emojis.tone());
        typing.choose(Choice::Pick(0), &mut emojis);
        assert_eq!(typing.text(), "\u{1f44d}\u{1f3fe} \u{1f44b}\u{1f3fe}");
        assert_eq!(emojis.chosen(), "\u{1f44b}\u{1f3fe}");
    }

    #[test]
    fn the_picker_types_at_the_cursor_and_escape_closes_it() {
        let mut emojis = EmojiPicker::default();
        let mut typing = Typing::new(Point::ORIGIN);
        typed(&mut typing, "Done !");
        typing.act(Action::Move(Motion::Left), SkinTone::Default);
        assert!(typing.choose(Choice::Browse, &mut emojis));
        assert!(typing.browsing);
        let search = Choice::Picker(emoji_picker::Event::Search("rocket".into()));
        assert!(
            !typing.choose(search, &mut emojis),
            "the search keeps the keyboard"
        );
        let rocket = Choice::Picker(emoji_picker::Event::Choose("\u{1f680}".into()));
        assert!(typing.choose(rocket, &mut emojis));
        assert_eq!(typing.text(), "Done \u{1f680}!");
        assert!(typing.browsing, "the picker stays open for more");
        typed(&mut typing, " :sm");
        assert!(typing.dismiss(), "Escape closes the offer first");
        assert!(typing.browsing);
        assert!(typing.dismiss(), "then the picker");
        assert!(!typing.dismiss());
    }

    #[test]
    fn the_editor_keeps_its_place_as_emoji_are_offered() {
        use iced::advanced::widget::Tree;

        fn editor_tag(typing: &Typing) -> (iced::advanced::widget::tree::Tag, usize) {
            let emojis = EmojiPicker::default();
            let element: Element<'_, ()> = text_box(
                typing,
                "typing-test",
                &emojis,
                Palette::of(kirie_ui::Scheme::Dark),
                |_| (),
                (),
                None,
                |_| (),
            );
            let tree = Tree::new(&element);
            (tree.children[0].children[0].tag, tree.children.len())
        }

        let mut typing = Typing::new(Point::ORIGIN);
        let (plain, _) = editor_tag(&typing);
        typed(&mut typing, ":smi");
        assert!(typing.offer.is_some());
        let (offering, children) = editor_tag(&typing);
        assert_eq!(plain, offering, "the editor stays the first child");
        assert_eq!(children, 2, "the offer sits under the editor");
        typing.browsing = true;
        let (browsing, children) = editor_tag(&typing);
        assert_eq!(plain, browsing);
        assert_eq!(children, 3, "the picker sits under the offer");
    }

    #[test]
    fn emoji_spans_cover_whole_sequences() {
        let family = "\u{1f469}\u{200d}\u{1f469}\u{200d}\u{1f467}";
        let line = format!("Hi {family}! \u{2702}\u{fe0f} ok \u{1f1e8}\u{1f1ed}");
        let spans: Vec<&str> = emoji_spans(&line)
            .into_iter()
            .map(|span| &line[span])
            .collect();
        assert_eq!(spans, [family, "\u{2702}\u{fe0f}", "\u{1f1e8}\u{1f1ed}"]);
        assert!(emoji_spans("plain words, 12:30").is_empty());
    }

    #[test]
    fn a_second_quick_press_is_a_double_click() {
        let mut last = None;
        assert!(!double_click(&mut last, iced::Point::new(10.0, 10.0)));
        assert!(double_click(&mut last, iced::Point::new(10.0, 10.0)));
    }
}
