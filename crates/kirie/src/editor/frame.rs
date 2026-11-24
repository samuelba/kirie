//! The frame around the picture: room, what fills it, a shadow, and the marks beyond it.

use iced::widget::{column, container, row, text};
use iced::{Alignment, Element};
use kirie_annotate::{Frame, Rgba};
use kirie_ui::icon::Icon;
use kirie_ui::widget::{icon_button, swatch, value_slider};
use kirie_ui::{Palette, space};

use crate::annotate::paint::color_of;

/// What the room around the picture can be filled with; none leaves it transparent.
const BACKGROUNDS: [Option<Rgba>; 5] = [
    None,
    Some(Rgba::rgb(0xff, 0xff, 0xff)),
    Some(Rgba::rgb(0xe9, 0xec, 0xf1)),
    Some(Rgba::rgb(0x2a, 0x2e, 0x37)),
    Some(Rgba::rgb(0x00, 0x00, 0x00)),
];

/// A change to the frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Event {
    /// The margin, in logical pixels.
    Margin(f64),
    /// The margin slider was let go.
    MarginDone,
    Background(Option<Rgba>),
    ToggleShadow,
    ToggleOverflow,
}

impl Event {
    /// `frame` as this event changes it; `unit` is image pixels per logical pixel.
    pub fn apply(self, frame: Frame, unit: f64) -> Frame {
        match self {
            Event::Margin(margin) => Frame {
                margin: margin * unit,
                ..frame
            },
            Event::MarginDone => frame,
            Event::Background(background) => Frame {
                background,
                ..frame
            },
            Event::ToggleShadow => Frame {
                shadow: !frame.shadow,
                ..frame
            },
            Event::ToggleOverflow => Frame {
                overflow: !frame.overflow,
                ..frame
            },
        }
    }
}

/// The margin, the background and the toggles for the shadow and the marks beyond the picture.
pub fn view<'a, Message: Clone + 'a>(
    frame: Frame,
    unit: f64,
    palette: Palette,
    on: impl Fn(Event) -> Message + Clone + 'a,
) -> Element<'a, Message> {
    let margin = frame.margin / unit;
    let slide = on.clone();
    let margin = row![
        container(value_slider(
            0.0..=Frame::MAX_MARGIN,
            margin,
            palette,
            move |margin| slide(Event::Margin(margin.round())),
            on(Event::MarginDone),
        ))
        .width(150),
        text(format!("{margin:.0} px"))
            .size(12)
            .color(palette.muted),
    ]
    .spacing(space::S)
    .align_y(Alignment::Center);
    let backgrounds = row(BACKGROUNDS.into_iter().map(|background| {
        swatch(
            background.map_or(iced::Color::TRANSPARENT, color_of),
            frame.background == background,
            palette,
            on(Event::Background(background)),
        )
    }))
    .spacing(2);
    let toggle = |icon: Icon, label: &'static str, active: bool, event: Event| {
        row![
            icon_button(icon, label, palette, active, Some(on(event))),
            text(label).size(13),
        ]
        .spacing(space::S)
        .align_y(Alignment::Center)
    };
    column![
        margin,
        backgrounds,
        toggle(
            Icon::Shadow,
            "Shadow under the picture",
            frame.shadow,
            Event::ToggleShadow
        ),
        toggle(
            Icon::Expand,
            "Take in marks beyond it",
            frame.overflow,
            Event::ToggleOverflow
        ),
    ]
    .spacing(space::S)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_change_the_frame() {
        let frame = Frame::default();
        let wide = Event::Margin(20.0).apply(frame, 2.0);
        assert!((wide.margin - 40.0).abs() < f64::EPSILON);
        assert_eq!(Event::MarginDone.apply(wide, 2.0), wide);
        let white = Some(Rgba::rgb(0xff, 0xff, 0xff));
        assert_eq!(Event::Background(white).apply(frame, 1.0).background, white);
        assert!(Event::ToggleShadow.apply(frame, 1.0).shadow);
        assert!(Event::ToggleOverflow.apply(frame, 1.0).overflow);
    }
}
