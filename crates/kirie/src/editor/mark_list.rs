//! The list of marks, topmost first, floating over the canvas: it picks a mark that others
//! cover, which a click on the canvas cannot reach.

use iced::widget::{Space, column, container, mouse_area, row, scrollable, svg, text};
use iced::{Alignment, Background, Border, Element, Length, Task};
use kirie_annotate::{Kind, Shape, ShapeId, Tool};
use kirie_ui::icon::Icon;
use kirie_ui::widget::{floating, hint, small_icon_button};
use kirie_ui::{Palette, radius, space};

use super::{Editor, Message};

const WIDTH: f32 = 224.0;
const ROW: f32 = 30.0;
const MOST_ROWS: f32 = 12.0;
/// The most characters of a text mark a row shows.
const SNIPPET: usize = 24;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Event {
    /// Opens or closes the list.
    Toggle,
    /// Picks the mark alone, or adds or takes it away with Ctrl or Shift held.
    Pick(ShapeId),
    /// Opens the words of a text mark or speech bubble for typing.
    Edit(ShapeId),
    /// The pointer is over the row of a mark, or over none.
    Hover(Option<ShapeId>),
}

/// Whether the list is open, and the mark whose row is under the pointer, outlined on the
/// canvas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Shown {
    Closed,
    Open { hovered: Option<ShapeId> },
}

impl Shown {
    pub fn hovered(self) -> Option<ShapeId> {
        match self {
            Shown::Closed => None,
            Shown::Open { hovered } => hovered,
        }
    }
}

/// What a row shows before its name.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Face {
    Icon(Icon),
    Emoji(String),
}

impl Editor {
    pub(super) fn mark_list_event(&mut self, event: Event) -> Task<Message> {
        match event {
            Event::Toggle => {
                self.mark_list = match self.mark_list {
                    Shown::Closed => Shown::Open { hovered: None },
                    Shown::Open { .. } => Shown::Closed,
                };
            }
            Event::Hover(hovered) => {
                if let Shown::Open { .. } = self.mark_list {
                    self.mark_list = Shown::Open { hovered };
                }
            }
            Event::Pick(id) if self.cropping.is_none() => {
                self.pick_tool(Tool::Select);
                let adding = self.modifiers.command() || self.modifiers.shift();
                if !adding {
                    self.picked = vec![id];
                } else if let Some(at) = self.picked.iter().position(|picked| *picked == id) {
                    self.picked.remove(at);
                } else {
                    self.picked.push(id);
                }
            }
            Event::Edit(id) if self.cropping.is_none() => {
                self.pick_tool(Tool::Select);
                return self.edit_text(id);
            }
            Event::Pick(_) | Event::Edit(_) => {}
        }
        Task::none()
    }

    /// The list when open, else a button that opens it; nothing without an image or while
    /// cropping.
    pub(super) fn mark_list(&self) -> Option<Element<'_, Message>> {
        if self.image.is_none() || self.cropping.is_some() {
            return None;
        }
        let palette = self.palette;
        let count = self.document().shapes().len();
        if self.mark_list == Shown::Closed {
            let opener = iced::widget::button(heading(count, palette))
                .padding([space::XS, space::M])
                .on_press(Message::MarkList(Event::Toggle))
                .style(move |_, status| iced::widget::button::Style {
                    background: Some(Background::Color(match status {
                        iced::widget::button::Status::Hovered => palette.surface[1],
                        iced::widget::button::Status::Pressed => palette.surface[2],
                        _ => palette.surface[0].scale_alpha(0.96),
                    })),
                    text_color: palette.text,
                    border: Border {
                        color: palette.border,
                        width: 1.0,
                        radius: radius::PILL.into(),
                    },
                    ..iced::widget::button::Style::default()
                });
            return Some(hint(opener.into(), "Show the marks (M)", palette));
        }
        let header = row![
            heading(count, palette),
            Space::new().width(Length::Fill),
            small_icon_button(
                Icon::Close,
                "Hide the marks (M)",
                palette,
                Some(Message::MarkList(Event::Toggle)),
            ),
        ]
        .align_y(Alignment::Center);
        let body: Element<'_, Message> = if count == 0 {
            text("No marks yet").size(12).color(palette.muted).into()
        } else {
            let rows = self
                .document()
                .shapes()
                .rev()
                .map(|shape| self.mark_row(shape));
            #[allow(clippy::cast_precision_loss)]
            let height = ROW * (count as f32).min(MOST_ROWS);
            scrollable(column(rows))
                .spacing(space::XS)
                .height(height)
                .into()
        };
        Some(
            container(column![header, body].spacing(space::S))
                .width(WIDTH)
                .padding(space::S)
                .style(floating(palette))
                .into(),
        )
    }

    fn mark_row(&self, shape: &Shape) -> Element<'_, Message> {
        let palette = self.palette;
        let picked = self.picked.contains(&shape.id);
        let hovered = self.mark_list.hovered() == Some(shape.id);
        let (face, name) = describe(&shape.kind);
        let face: Element<'_, Message> = match face {
            Face::Icon(icon) => svg(icon.handle())
                .width(16)
                .height(16)
                .style(move |_, _| svg::Style {
                    color: Some(palette.muted),
                })
                .into(),
            Face::Emoji(emoji) => text(emoji)
                .font(crate::annotate::emoji_picker::FONT)
                .size(14)
                .line_height(1.0)
                .into(),
        };
        let content = container(
            row![
                container(face).center_x(18),
                text(name)
                    .size(13)
                    .wrapping(text::Wrapping::None)
                    .width(Length::Fill),
            ]
            .spacing(space::S)
            .align_y(Alignment::Center),
        )
        .height(ROW)
        .width(Length::Fill)
        .padding([0.0, space::S])
        .align_y(Alignment::Center)
        .clip(true)
        .style(move |_| container::Style {
            background: if picked {
                Some(Background::Color(palette.accent.scale_alpha(0.22)))
            } else if hovered {
                Some(Background::Color(palette.surface[1]))
            } else {
                None
            },
            text_color: Some(palette.text),
            border: Border {
                radius: radius::CONTROL.into(),
                ..Border::default()
            },
            ..container::Style::default()
        });
        let mut area = mouse_area(content)
            .on_press(Message::MarkList(Event::Pick(shape.id)))
            .on_enter(Message::MarkList(Event::Hover(Some(shape.id))))
            .on_exit(Message::MarkList(Event::Hover(None)))
            .interaction(iced::mouse::Interaction::Pointer);
        if matches!(shape.kind, Kind::Text { .. } | Kind::Bubble { .. }) {
            area = area.on_double_click(Message::MarkList(Event::Edit(shape.id)));
        }
        area.into()
    }
}

/// The icon, title and count at the top of the list.
fn heading<'a>(count: usize, palette: Palette) -> Element<'a, Message> {
    row![
        svg(Icon::Layers.handle())
            .width(16)
            .height(16)
            .style(move |_, _| svg::Style {
                color: Some(palette.text),
            }),
        text("Marks").size(13),
        text(count.to_string()).size(12).color(palette.muted),
    ]
    .spacing(space::S)
    .align_y(Alignment::Center)
    .into()
}

/// What a row shows for a mark of `kind`: its face and its name.
fn describe(kind: &Kind) -> (Face, String) {
    let named = |icon, name: &str| (Face::Icon(icon), name.to_owned());
    match kind {
        Kind::Arrow { .. } => named(Icon::Arrow, "Arrow"),
        Kind::Line { .. } => named(Icon::Line, "Line"),
        Kind::Rectangle { .. } => named(Icon::Rectangle, "Rectangle"),
        Kind::Ellipse { .. } => named(Icon::Ellipse, "Ellipse"),
        Kind::Pen { .. } => named(Icon::Pen, "Pen"),
        Kind::Highlighter { .. } => named(Icon::Highlighter, "Highlighter"),
        Kind::Text { text, .. } => (Face::Icon(Icon::Text), snippet(text, "Text")),
        Kind::Bubble { text, .. } => (Face::Icon(Icon::Bubble), snippet(text, "Speech bubble")),
        Kind::Counter { number, .. } => (Face::Icon(Icon::Counter), format!("Step {number}")),
        Kind::Blur { .. } => named(Icon::Blur, "Blur"),
        Kind::Pixelate { .. } => named(Icon::Pixelate, "Pixelate"),
        Kind::Redact { .. } => named(Icon::Redact, "Redact"),
        Kind::Spotlight { .. } => named(Icon::Spotlight, "Spotlight"),
        Kind::Sticker { emoji, .. } => (Face::Emoji(emoji.clone()), "Sticker".to_owned()),
        Kind::Image { .. } => named(Icon::Image, "Picture"),
    }
}

/// The first line of `words` with something in it, cut short; `empty` when there is none.
fn snippet(words: &str, empty: &str) -> String {
    let Some(line) = words.lines().map(str::trim).find(|line| !line.is_empty()) else {
        return empty.to_owned();
    };
    if line.chars().count() <= SNIPPET {
        return line.to_owned();
    }
    let cut: String = line.chars().take(SNIPPET - 1).collect();
    format!("{}…", cut.trim_end())
}

#[cfg(test)]
mod tests {
    use kirie_annotate::{Op, Paint};
    use kurbo::Point;

    use super::*;
    use crate::editor::Loaded;

    /// An editor with two marks, the second over the first; their ids, bottom first.
    fn stacked() -> (Editor, [ShapeId; 2]) {
        let mut editor = Editor::boot(Some(Loaded::image(::image::RgbaImage::new(40, 30), None))).0;
        let rect = kurbo::Rect::new(1.0, 1.0, 20.0, 20.0);
        for kind in [Kind::Rectangle { rect }, Kind::Redact { rect }] {
            editor.history.apply(Op::Add {
                kind,
                paint: Paint::default(),
            });
        }
        let ids: Vec<_> = editor.document().shapes().map(|shape| shape.id).collect();
        (editor, [ids[0], ids[1]])
    }

    #[test]
    fn the_list_picks_a_covered_mark_and_ctrl_adds_to_the_pick() {
        let (mut editor, [below, above]) = stacked();
        editor.tool = Tool::Arrow;
        let _ = editor.update(Message::MarkList(Event::Pick(below)));
        assert_eq!(editor.tool, Tool::Select);
        assert_eq!(editor.picked, [below]);
        editor.modifiers = iced::keyboard::Modifiers::CTRL;
        let _ = editor.update(Message::MarkList(Event::Pick(above)));
        assert_eq!(editor.picked, [below, above]);
        let _ = editor.update(Message::MarkList(Event::Pick(below)));
        assert_eq!(editor.picked, [above]);
    }

    #[test]
    fn rows_are_hovered_only_while_the_list_is_open() {
        let (mut editor, [below, _]) = stacked();
        assert!(editor.mark_list().is_some(), "a closed list is a button");
        let _ = editor.update(Message::MarkList(Event::Hover(Some(below))));
        assert_eq!(editor.mark_list.hovered(), None);
        let _ = editor.update(Message::MarkList(Event::Toggle));
        let _ = editor.update(Message::MarkList(Event::Hover(Some(below))));
        assert_eq!(editor.mark_list.hovered(), Some(below));
        let _ = editor.update(Message::MarkList(Event::Toggle));
        assert_eq!(editor.mark_list, Shown::Closed);
        assert!(Editor::boot(None).0.mark_list().is_none());
    }

    #[test]
    fn text_marks_show_their_first_line() {
        assert_eq!(snippet("\n  Hello there \nsecond", "Text"), "Hello there");
        assert_eq!(snippet(" \n ", "Text"), "Text");
        let long = "a".repeat(40);
        let cut = snippet(&long, "Text");
        assert_eq!(cut.chars().count(), SNIPPET);
        assert!(cut.ends_with('…'));
    }

    #[test]
    fn rows_name_what_tells_marks_apart() {
        let at = Point::ORIGIN;
        assert_eq!(
            describe(&Kind::Counter { at, number: 3 }),
            (Face::Icon(Icon::Counter), "Step 3".to_owned())
        );
        let sticker = Kind::Sticker {
            rect: kurbo::Rect::ZERO,
            emoji: "🎉".to_owned(),
        };
        assert_eq!(
            describe(&sticker),
            (Face::Emoji("🎉".to_owned()), "Sticker".to_owned())
        );
        let words = Kind::Text {
            at,
            text: "Click here".to_owned(),
        };
        assert_eq!(describe(&words).1, "Click here");
    }
}
