//! Typing the words of a text mark, new or already placed, in a box that takes several lines:
//! Enter places the text and Shift+Enter starts a new line.

use iced::Element;
use iced::advanced::mouse;
use iced::keyboard::{Key, key::Named};
use iced::widget::text_editor::{self, Binding, Content, Motion};
use kirie_annotate::{Document, Kind, Op, Paint, Shape, ShapeId};
use kirie_ui::{Palette, radius, space};

/// Text being typed at `at`, for a new mark or in place of the words of `editing`.
#[derive(Debug)]
pub struct Typing {
    pub at: kurbo::Point,
    pub content: Content,
    pub editing: Option<ShapeId>,
}

impl Typing {
    pub fn new(at: kurbo::Point) -> Self {
        Self {
            at,
            content: Content::new(),
            editing: None,
        }
    }

    /// Typing that changes the words of `shape`, if it is a text mark.
    pub fn edit(shape: &Shape) -> Option<Self> {
        let Kind::Text { at, text } = &shape.kind else {
            return None;
        };
        let mut content = Content::with_text(text);
        content.perform(text_editor::Action::Move(Motion::DocumentEnd));
        Some(Self {
            at: *at,
            content,
            editing: Some(shape.id),
        })
    }

    pub fn text(&self) -> String {
        self.content.text()
    }

    /// The mark as typed so far, to draw in place of the one being edited; `paint` is for a
    /// new mark.
    pub fn preview(&self, document: &Document, paint: Paint) -> Shape {
        let text = self.text();
        let kind = Kind::Text {
            at: self.at,
            text: if text.is_empty() {
                "|".to_owned()
            } else {
                text
            },
        };
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
        let kind = Kind::Text { at: self.at, text };
        match self.editing {
            Some(id) if blank => Some(Op::Remove { ids: vec![id] }),
            Some(id) => Some(Op::Replace { id, kind }),
            None if blank => None,
            None => Some(Op::Add { kind, paint }),
        }
    }
}

/// The text mark under `at`, within `reach`.
pub fn text_at(document: &Document, at: kurbo::Point, reach: f64) -> Option<ShapeId> {
    document.hit(at, reach).filter(|id| {
        matches!(
            document.shape(*id).map(|shape| &shape.kind),
            Some(Kind::Text { .. })
        )
    })
}

/// Whether a press makes a double click with the one before; keeps it for the next.
pub fn double_click(last: &mut Option<mouse::Click>, at: iced::Point) -> bool {
    let click = mouse::Click::new(at, mouse::Button::Left, *last);
    *last = Some(click);
    click.kind() == mouse::click::Kind::Double
}

/// The box the words are typed in.
pub fn text_box<'a, Message: Clone + 'a>(
    typing: &'a Typing,
    id: &'static str,
    palette: Palette,
    on_action: impl Fn(text_editor::Action) -> Message + 'a,
    on_done: Message,
) -> Element<'a, Message> {
    iced::widget::text_editor(&typing.content)
        .id(id)
        .placeholder("Type, then Enter")
        .on_action(on_action)
        .key_binding(move |press| {
            if press.key == Key::Named(Named::Enter) && !press.modifiers.shift() {
                return Some(Binding::Custom(on_done.clone()));
            }
            Binding::from_key_press(press)
        })
        .width(300)
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
        })
        .into()
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
        assert_eq!(text_at(&document, Point::new(30.0, 30.0), 4.0), Some(id));
        assert_eq!(text_at(&document, Point::new(250.0, 150.0), 4.0), None);
        let boxed = document.apply(Op::Add {
            kind: Kind::Rectangle {
                rect: kurbo::Rect::new(100.0, 100.0, 200.0, 150.0),
            },
            paint: Paint::default(),
        });
        assert_eq!(text_at(&boxed, Point::new(150.0, 120.0), 4.0), None);
        assert!(Typing::edit(boxed.shapes().last().unwrap()).is_none());
    }

    #[test]
    fn a_second_quick_press_is_a_double_click() {
        let mut last = None;
        assert!(!double_click(&mut last, iced::Point::new(10.0, 10.0)));
        assert!(double_click(&mut last, iced::Point::new(10.0, 10.0)));
    }
}
