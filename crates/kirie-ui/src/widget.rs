//! Kirie's controls: flat icon buttons, colour swatches, segmented pills and the floating panel
//! they sit on.

use iced::widget::{button, container, row, svg, text, tooltip};
use iced::{Background, Border, Color, Element, Length, Shadow, Vector};

use crate::icon::Icon;
use crate::{Palette, radius, space};

const ICON: f32 = 18.0;
const CONTROL: f32 = 34.0;

/// The panel of a floating toolbar.
pub fn floating(palette: Palette) -> impl Fn(&iced::Theme) -> container::Style {
    move |_| container::Style {
        text_color: Some(palette.text),
        background: Some(Background::Color(palette.surface[0].scale_alpha(0.96))),
        border: Border {
            color: palette.border,
            width: 1.0,
            radius: radius::FLOATING.into(),
        },
        shadow: Shadow {
            color: Color::BLACK.scale_alpha(0.35),
            offset: Vector::new(0.0, 8.0),
            blur_radius: 24.0,
        },
        snap: true,
    }
}

/// A square button with an icon, highlighted while `active`.
pub fn icon_button<'a, Message: Clone + 'a>(
    icon: Icon,
    label: &'a str,
    palette: Palette,
    active: bool,
    on_press: Option<Message>,
) -> Element<'a, Message> {
    let tint = if active {
        palette.on_accent
    } else {
        palette.text
    };
    let glyph = svg(icon.handle())
        .width(ICON)
        .height(ICON)
        .style(move |_, _| svg::Style { color: Some(tint) });
    let control = button(container(glyph).center(Length::Fill))
        .width(CONTROL)
        .height(CONTROL)
        .padding(0)
        .on_press_maybe(on_press)
        .style(move |_, status| {
            let background = match (active, status) {
                (true, _) => Some(palette.accent),
                (false, button::Status::Hovered) => Some(palette.surface[1]),
                (false, button::Status::Pressed) => Some(palette.surface[2]),
                _ => None,
            };
            button::Style {
                background: background.map(Background::Color),
                text_color: tint,
                border: Border {
                    radius: radius::CONTROL.into(),
                    ..Border::default()
                },
                shadow: Shadow::default(),
                snap: true,
            }
        });
    hint(control.into(), label, palette)
}

/// A labelled button for the main action.
pub fn primary_button<'a, Message: Clone + 'a>(
    icon: Icon,
    label: &'a str,
    palette: Palette,
    on_press: Message,
) -> Element<'a, Message> {
    let glyph = svg(icon.handle())
        .width(ICON)
        .height(ICON)
        .style(move |_, _| svg::Style {
            color: Some(palette.on_accent),
        });
    button(
        container(
            row![glyph, text(label).size(14)]
                .spacing(space::S)
                .align_y(iced::Alignment::Center),
        )
        .center_y(Length::Fill),
    )
    .height(CONTROL)
    .padding([0.0, space::M])
    .on_press(on_press)
    .style(move |_, status| {
        let background = match status {
            button::Status::Hovered => mix(palette.accent, Color::WHITE, 0.12),
            button::Status::Pressed => mix(palette.accent, Color::BLACK, 0.12),
            _ => palette.accent,
        };
        button::Style {
            background: Some(Background::Color(background)),
            text_color: palette.on_accent,
            border: Border {
                radius: radius::CONTROL.into(),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: true,
        }
    })
    .into()
}

/// A round colour choice, ringed when selected.
pub fn swatch<'a, Message: Clone + 'a>(
    color: Color,
    selected: bool,
    palette: Palette,
    on_press: Message,
) -> Element<'a, Message> {
    let dot = container(text(""))
        .width(18)
        .height(18)
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                color: Color::WHITE.scale_alpha(0.35),
                width: 1.0,
                radius: radius::PILL.into(),
            },
            ..container::Style::default()
        });
    button(container(dot).center(Length::Fill))
        .width(28)
        .height(28)
        .padding(0)
        .on_press(on_press)
        .style(move |_, status| button::Style {
            background: None,
            text_color: palette.text,
            border: Border {
                color: if selected {
                    palette.text
                } else if status == button::Status::Hovered {
                    palette.muted
                } else {
                    Color::TRANSPARENT
                },
                width: 2.0,
                radius: radius::PILL.into(),
            },
            shadow: Shadow::default(),
            snap: true,
        })
        .into()
}

/// One option of a segmented control.
pub fn pill<'a, Message: Clone + 'a>(
    label: &'a str,
    selected: bool,
    palette: Palette,
    on_press: Message,
) -> Element<'a, Message> {
    button(text(label).size(13))
        .padding([space::XS + 2.0, space::M])
        .on_press(on_press)
        .style(move |_, status| button::Style {
            background: match (selected, status) {
                (true, _) => Some(Background::Color(palette.surface[2])),
                (false, button::Status::Hovered) => Some(Background::Color(palette.surface[1])),
                _ => None,
            },
            text_color: if selected {
                palette.text
            } else {
                palette.muted
            },
            border: Border {
                radius: radius::PILL.into(),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: true,
        })
        .into()
}

/// A thin vertical divider between groups of controls.
pub fn divider<'a, Message: 'a>(palette: Palette) -> Element<'a, Message> {
    container(text(""))
        .width(1)
        .height(20)
        .style(move |_| container::Style {
            background: Some(Background::Color(palette.border)),
            ..container::Style::default()
        })
        .into()
}

fn hint<'a, Message: 'a>(
    content: Element<'a, Message>,
    label: &'a str,
    palette: Palette,
) -> Element<'a, Message> {
    tooltip(
        content,
        container(text(label).size(12))
            .padding([space::XS, space::S])
            .style(move |_| container::Style {
                text_color: Some(palette.text),
                background: Some(Background::Color(palette.surface[2])),
                border: Border {
                    radius: radius::CONTROL.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            }),
        tooltip::Position::Top,
    )
    .gap(space::S)
    .into()
}

/// `a` moved `amount` of the way towards `b`.
pub fn mix(a: Color, b: Color, amount: f32) -> Color {
    Color {
        r: a.r + (b.r - a.r) * amount,
        g: a.g + (b.g - a.g) * amount,
        b: a.b + (b.b - a.b) * amount,
        a: a.a + (b.a - a.a) * amount,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mixing_moves_between_colours() {
        assert_eq!(mix(Color::BLACK, Color::WHITE, 0.0), Color::BLACK);
        assert_eq!(mix(Color::BLACK, Color::WHITE, 1.0), Color::WHITE);
        let half = mix(Color::BLACK, Color::WHITE, 0.5);
        assert!((half.r - 0.5).abs() < f32::EPSILON);
    }
}
