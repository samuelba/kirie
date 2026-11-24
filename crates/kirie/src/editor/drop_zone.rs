//! The hint shown while something that can be opened is dragged over the window.

use iced::widget::canvas::{self, Frame, Geometry, LineDash, Path, Stroke};
use iced::widget::{column, container, stack, svg, text};
use iced::{Alignment, Background, Border, Element, Length, Point, Rectangle, Renderer, Theme};
use kirie_ui::icon::Icon;
use kirie_ui::{Palette, radius, space};

use super::Message;

const BADGE_ICON: f32 = 44.0;

pub(super) fn view<'a>(palette: Palette) -> Element<'a, Message> {
    let badge = container(
        svg(Icon::ImageDown.handle())
            .width(BADGE_ICON)
            .height(BADGE_ICON)
            .style(move |_, _| svg::Style {
                color: Some(palette.accent),
            }),
    )
    .padding(space::XL)
    .style(move |_| container::Style {
        background: Some(Background::Color(palette.accent.scale_alpha(0.14))),
        border: Border {
            radius: radius::PILL.into(),
            ..Border::default()
        },
        ..container::Style::default()
    });
    let words = column![
        badge,
        text("Drop to open").size(22).color(palette.text),
        text("An image or a Kirie project")
            .size(14)
            .color(palette.muted),
    ]
    .spacing(space::M)
    .align_x(Alignment::Center);
    stack![
        canvas::Canvas::new(Zone { palette })
            .width(Length::Fill)
            .height(Length::Fill),
        container(words).center(Length::Fill),
    ]
    .into()
}

/// The dimmed window and the dashed frame drawn around where the drop lands.
struct Zone {
    palette: Palette,
}

impl canvas::Program<Message> for Zone {
    type State = ();

    fn draw(
        &self,
        _state: &(),
        renderer: &Renderer,
        _theme: &Theme,
        bounds: Rectangle,
        _cursor: iced::mouse::Cursor,
    ) -> Vec<Geometry> {
        let palette = self.palette;
        let mut frame = Frame::new(renderer, bounds.size());
        frame.fill_rectangle(
            Point::ORIGIN,
            bounds.size(),
            palette.background.scale_alpha(0.86),
        );
        let inset = space::L;
        let size = iced::Size::new(
            (bounds.width - 2.0 * inset).max(0.0),
            (bounds.height - 2.0 * inset).max(0.0),
        );
        let outline =
            Path::rounded_rectangle(Point::new(inset, inset), size, radius::FLOATING.into());
        frame.fill(&outline, palette.accent.scale_alpha(0.06));
        frame.stroke(
            &outline,
            Stroke {
                line_dash: LineDash {
                    segments: &[12.0, 8.0],
                    offset: 0,
                },
                ..Stroke::default().with_color(palette.accent).with_width(2.0)
            },
        );
        vec![frame.into_geometry()]
    }
}

#[cfg(test)]
mod tests {
    use super::super::Editor;
    use super::Message;
    use crate::desktop::drops::Dropped;

    #[test]
    fn the_hint_shows_while_something_is_dragged_over() {
        let mut editor = Editor::boot(None).0;
        assert!(!editor.drops.hovered);
        let _ = editor.update(Message::Dropped(Dropped::Hovering(true)));
        assert!(editor.drops.hovered);
        let _ = editor.update(Message::Dropped(Dropped::Hovering(false)));
        assert!(!editor.drops.hovered);
    }
}
