//! A color picker: a saturation and brightness field, a hue strip and a hex field, for adding a
//! custom color or changing a swatch.

use iced::mouse::{self, Interaction};
use iced::widget::canvas::{self, Action, Frame, Geometry, Path, Stroke, gradient};
use iced::widget::{column, container, row, text, text_input};
use iced::{Alignment, Background, Border, Color, Element, Length, Point, Rectangle, Renderer};
use kirie_annotate::{Rgba, StyleId};
use kirie_ui::icon::Icon;
use kirie_ui::widget::{icon_button, pill};
use kirie_ui::{Palette, radius, space};

use crate::paint::color_of;
use crate::swatches::{Slot, Swatches};

pub const WIDTH: f32 = 216.0;
/// The room the picker takes, for placing what holds it.
pub const HEIGHT: f32 = 264.0;
const SHADE: f32 = 132.0;
const HUE: f32 = 14.0;

/// A color as hue (degrees), saturation and value, so the hue survives grays.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hsv {
    pub hue: f32,
    pub saturation: f32,
    pub value: f32,
}

impl Hsv {
    pub fn from_rgba(color: Rgba) -> Self {
        let top = color.r.max(color.g).max(color.b);
        let bottom = color.r.min(color.g).min(color.b);
        let [r, g, b] = [color.r, color.g, color.b].map(|c| f32::from(c) / 255.0);
        let max = f32::from(top) / 255.0;
        let range = f32::from(top - bottom) / 255.0;
        let hue = if top == bottom {
            0.0
        } else if top == color.r {
            60.0 * ((g - b) / range).rem_euclid(6.0)
        } else if top == color.g {
            60.0 * ((b - r) / range + 2.0)
        } else {
            60.0 * ((r - g) / range + 4.0)
        };
        let saturation = if top == 0 { 0.0 } else { range / max };
        Self {
            hue,
            saturation,
            value: max,
        }
    }

    pub fn to_rgba(self) -> Rgba {
        let chroma = self.value * self.saturation;
        let sector = (self.hue.rem_euclid(360.0)) / 60.0;
        let x = chroma * (1.0 - (sector.rem_euclid(2.0) - 1.0).abs());
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let (r, g, b) = match sector as u32 {
            0 => (chroma, x, 0.0),
            1 => (x, chroma, 0.0),
            2 => (0.0, chroma, x),
            3 => (0.0, x, chroma),
            4 => (x, 0.0, chroma),
            _ => (chroma, 0.0, x),
        };
        let lift = self.value - chroma;
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        let byte = |c: f32| ((c + lift).clamp(0.0, 1.0) * 255.0).round() as u8;
        Rgba::rgb(byte(r), byte(g), byte(b))
    }
}

#[derive(Debug, Clone)]
pub enum Event {
    Shade {
        saturation: f32,
        value: f32,
    },
    Hue(f32),
    Hex(String),
    /// Starts taking a color from the picture with the next click.
    Eyedropper,
    /// The color under the click, taken from the picture.
    Sampled(Rgba),
    Apply,
    Remove,
    ResetDefaults,
    Cancel,
}

/// What the picker is open for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Target {
    /// A new custom color.
    New,
    Change(Slot),
}

#[derive(Debug, Clone)]
pub struct Picker {
    target: Target,
    /// The style whose swatches are offered.
    style: StyleId,
    hsv: Hsv,
    alpha: u8,
    hex: String,
    /// The eyedropper waits for a click on the picture.
    sampling: bool,
}

impl Picker {
    /// Opens on the swatch's color among `style`'s, or on `current` for a new one.
    pub fn new(target: Target, style: StyleId, swatches: &Swatches, current: Rgba) -> Self {
        let color = match target {
            Target::Change(slot) => swatches.get(style, slot),
            Target::New => None,
        }
        .unwrap_or(current.with_alpha(255));
        Self {
            target,
            style,
            hsv: Hsv::from_rgba(color),
            alpha: color.a,
            hex: color.to_hex(),
            sampling: false,
        }
    }

    /// Whether the eyedropper waits for a click on the picture, to take its color.
    pub fn sampling(&self) -> bool {
        self.sampling
    }

    pub fn target(&self) -> Target {
        self.target
    }

    pub fn color(&self) -> Rgba {
        self.hsv.to_rgba().with_alpha(self.alpha)
    }

    fn set(&mut self, hsv: Hsv) {
        self.hsv = hsv;
        self.hex = self.color().to_hex();
    }
}

/// What an event did to the picker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Editing,
    Closed,
    /// The swatches changed and the picker closed; draw with `pick` if given.
    Changed {
        pick: Option<Rgba>,
    },
}

/// Applies `event` to the open picker, closing it once done.
pub fn update(picker: &mut Option<Picker>, swatches: &mut Swatches, event: Event) -> Outcome {
    let Some(open) = picker.as_mut() else {
        return Outcome::Closed;
    };
    let pick = match event {
        Event::Shade { saturation, value } => {
            open.set(Hsv {
                saturation,
                value,
                ..open.hsv
            });
            return Outcome::Editing;
        }
        Event::Hue(hue) => {
            open.set(Hsv { hue, ..open.hsv });
            return Outcome::Editing;
        }
        Event::Hex(hex) => {
            let written = hex.trim().trim_start_matches('#');
            if let Some(color) = Rgba::from_hex(&format!("#{written}")) {
                open.hsv = Hsv::from_rgba(color);
                open.alpha = color.a;
            }
            open.hex = hex;
            return Outcome::Editing;
        }
        Event::Sampled(color) => {
            open.set(Hsv::from_rgba(color));
            open.sampling = false;
            return Outcome::Editing;
        }
        Event::Eyedropper => {
            open.sampling = !open.sampling;
            return Outcome::Editing;
        }
        Event::Cancel => {
            *picker = None;
            return Outcome::Closed;
        }
        Event::Apply => {
            let color = open.color();
            match open.target {
                Target::Change(slot) => swatches.replace(open.style, slot, color),
                Target::New => swatches.add(open.style, color),
            }
            Some(color)
        }
        Event::Remove => {
            if let Target::Change(slot) = open.target {
                swatches.remove(slot);
            }
            None
        }
        Event::ResetDefaults => {
            swatches.reset_defaults();
            None
        }
    };
    *picker = None;
    Outcome::Changed { pick }
}

pub fn view(picker: &Picker, palette: Palette) -> Element<'_, Event> {
    let title = match picker.target {
        Target::New => "New color",
        Target::Change(Slot::Default(_)) => "Change this color",
        Target::Change(Slot::Custom(_)) => "Change your color",
    };
    let preview = container(text("")).width(26).height(26).style({
        let color = color_of(picker.color());
        move |_| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                color: Color::WHITE.scale_alpha(0.35),
                width: 1.0,
                radius: radius::PILL.into(),
            },
            ..container::Style::default()
        }
    });
    let hex = text_input("#rrggbb", &picker.hex)
        .on_input(Event::Hex)
        .on_submit(Event::Apply)
        .size(13)
        .padding([space::XS, space::S])
        .width(Length::Fill)
        .style(move |_, _| text_input::Style {
            background: Background::Color(palette.surface[1]),
            border: Border {
                color: palette.border,
                width: 1.0,
                radius: radius::CONTROL.into(),
            },
            icon: palette.muted,
            placeholder: palette.muted,
            value: palette.text,
            selection: palette.accent.scale_alpha(0.4),
        });
    let mut buttons = row![pill(
        if picker.target == Target::New {
            "Add"
        } else {
            "Apply"
        },
        true,
        palette,
        Event::Apply
    )]
    .spacing(2);
    buttons = match picker.target {
        Target::Change(Slot::Custom(_)) => {
            buttons.push(pill("Remove", false, palette, Event::Remove))
        }
        Target::Change(Slot::Default(_)) => {
            buttons.push(pill("Reset all", false, palette, Event::ResetDefaults))
        }
        Target::New => buttons,
    };
    buttons = buttons.push(pill("Cancel", false, palette, Event::Cancel));
    column![
        text(title).size(12).color(palette.muted),
        canvas::Canvas::new(Field {
            hsv: picker.hsv,
            area: Area::Shade,
        })
        .width(WIDTH)
        .height(SHADE),
        canvas::Canvas::new(Field {
            hsv: picker.hsv,
            area: Area::Hue,
        })
        .width(WIDTH)
        .height(HUE),
        row![
            preview,
            hex,
            icon_button(
                Icon::Pipette,
                "Take a color from the picture",
                palette,
                picker.sampling,
                Some(Event::Eyedropper),
            )
        ]
        .spacing(space::S)
        .align_y(Alignment::Center),
        buttons,
    ]
    .spacing(space::S)
    .width(WIDTH)
    .into()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Area {
    Shade,
    Hue,
}

struct Field {
    hsv: Hsv,
    area: Area,
}

impl Field {
    fn pick(&self, at: Point, bounds: Rectangle) -> Event {
        let x = ((at.x - bounds.x) / bounds.width).clamp(0.0, 1.0);
        let y = ((at.y - bounds.y) / bounds.height).clamp(0.0, 1.0);
        match self.area {
            Area::Shade => Event::Shade {
                saturation: x,
                value: 1.0 - y,
            },
            Area::Hue => Event::Hue(x * 360.0),
        }
    }
}

impl canvas::Program<Event> for Field {
    /// Whether a drag started here.
    type State = bool;

    fn update(
        &self,
        dragging: &mut bool,
        event: &canvas::Event,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Option<Action<Event>> {
        match event {
            canvas::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => {
                let at = cursor.position_over(bounds)?;
                *dragging = true;
                Some(Action::publish(self.pick(at, bounds)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::CursorMoved { position }) if *dragging => {
                Some(Action::publish(self.pick(*position, bounds)).and_capture())
            }
            canvas::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left))
                if *dragging =>
            {
                *dragging = false;
                Some(Action::capture())
            }
            _ => None,
        }
    }

    fn draw(
        &self,
        _: &bool,
        renderer: &Renderer,
        _: &iced::Theme,
        bounds: Rectangle,
        _: mouse::Cursor,
    ) -> Vec<Geometry> {
        let mut frame = Frame::new(renderer, bounds.size());
        let size = bounds.size();
        let area = Path::rounded_rectangle(Point::ORIGIN, size, radius::CONTROL.into());
        let ring = |frame: &mut Frame, path: &Path| {
            frame.stroke(
                path,
                Stroke::default()
                    .with_color(Color::BLACK.scale_alpha(0.5))
                    .with_width(3.0),
            );
            frame.stroke(
                path,
                Stroke::default().with_color(Color::WHITE).with_width(1.5),
            );
        };
        match self.area {
            Area::Shade => {
                let pure = Hsv {
                    saturation: 1.0,
                    value: 1.0,
                    ..self.hsv
                };
                frame.fill(&area, color_of(pure.to_rgba()));
                let across = gradient::Linear::new(Point::ORIGIN, Point::new(size.width, 0.0))
                    .add_stop(0.0, Color::WHITE)
                    .add_stop(1.0, Color::WHITE.scale_alpha(0.0));
                frame.fill(&area, across);
                let down = gradient::Linear::new(Point::ORIGIN, Point::new(0.0, size.height))
                    .add_stop(0.0, Color::BLACK.scale_alpha(0.0))
                    .add_stop(1.0, Color::BLACK);
                frame.fill(&area, down);
                let at = Point::new(
                    self.hsv.saturation * size.width,
                    (1.0 - self.hsv.value) * size.height,
                );
                ring(&mut frame, &Path::circle(at, 6.0));
            }
            Area::Hue => {
                let rainbow = (0..=6u8).fold(
                    gradient::Linear::new(Point::ORIGIN, Point::new(size.width, 0.0)),
                    |strip, step| {
                        let hue = Hsv {
                            hue: f32::from(step) * 60.0,
                            saturation: 1.0,
                            value: 1.0,
                        };
                        strip.add_stop(f32::from(step) / 6.0, color_of(hue.to_rgba()))
                    },
                );
                frame.fill(&area, rainbow);
                let x = (self.hsv.hue / 360.0 * size.width).clamp(3.0, size.width - 3.0);
                ring(
                    &mut frame,
                    &Path::rounded_rectangle(
                        Point::new(x - 3.0, 0.0),
                        iced::Size::new(6.0, size.height),
                        3.0.into(),
                    ),
                );
            }
        }
        vec![frame.into_geometry()]
    }

    fn mouse_interaction(
        &self,
        dragging: &bool,
        bounds: Rectangle,
        cursor: mouse::Cursor,
    ) -> Interaction {
        if *dragging || cursor.is_over(bounds) {
            Interaction::Crosshair
        } else {
            Interaction::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kirie_annotate::PALETTE;

    #[test]
    fn colors_survive_hsv() {
        for color in PALETTE
            .into_iter()
            .chain([Rgba::rgb(0x80, 0x80, 0x80), Rgba::rgb(1, 2, 3)])
        {
            assert_eq!(Hsv::from_rgba(color).to_rgba(), color);
        }
        let red = Hsv::from_rgba(Rgba::rgb(255, 0, 0));
        assert!((red.hue, red.saturation, red.value) == (0.0, 1.0, 1.0));
        let blue = Hsv::from_rgba(Rgba::rgb(0, 0, 255));
        assert!((blue.hue - 240.0).abs() < 1e-3);
    }

    #[test]
    fn a_new_color_is_kept_and_drawn_with() {
        let mut swatches = Swatches::default();
        let mut picker = Some(Picker::new(
            Target::New,
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        update(&mut picker, &mut swatches, Event::Hue(240.0));
        let shade = Event::Shade {
            saturation: 1.0,
            value: 1.0,
        };
        assert_eq!(update(&mut picker, &mut swatches, shade), Outcome::Editing);
        assert_eq!(picker.as_ref().unwrap().hex, "#0000ff");
        let blue = Rgba::rgb(0, 0, 255);
        assert_eq!(
            update(&mut picker, &mut swatches, Event::Apply),
            Outcome::Changed { pick: Some(blue) }
        );
        assert_eq!(swatches.custom(), [blue]);
        assert!(picker.is_none());
    }

    #[test]
    fn typing_a_hex_moves_the_picker_once_it_is_whole() {
        let mut swatches = Swatches::default();
        let mut picker = Some(Picker::new(
            Target::Change(Slot::Default(1)),
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        assert_eq!(picker.as_ref().unwrap().color(), PALETTE[1]);
        update(&mut picker, &mut swatches, Event::Hex("14b8".into()));
        assert_eq!(picker.as_ref().unwrap().color(), PALETTE[1]);
        update(&mut picker, &mut swatches, Event::Hex("14b8a6".into()));
        let open = picker.as_ref().unwrap();
        assert_eq!(open.color(), Rgba::rgb(0x14, 0xb8, 0xa6));
        assert_eq!(open.hex, "14b8a6");
    }

    #[test]
    fn a_changed_default_is_drawn_with() {
        let mut swatches = Swatches::default();
        let mut picker = Some(Picker::new(
            Target::Change(Slot::Default(2)),
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        update(&mut picker, &mut swatches, Event::Hex("#ec4899".into()));
        let pink = Rgba::rgb(0xec, 0x48, 0x99);
        assert_eq!(
            update(&mut picker, &mut swatches, Event::Apply),
            Outcome::Changed { pick: Some(pink) }
        );
        assert_eq!(swatches.defaults(StyleId::Clean)[2], pink);
    }

    #[test]
    fn removing_and_resetting_change_the_swatches_only() {
        let mut swatches = Swatches::default();
        swatches.add(StyleId::Clean, Rgba::rgb(1, 2, 3));
        swatches.replace(StyleId::Clean, Slot::Default(0), Rgba::rgb(4, 5, 6));
        let mut picker = Some(Picker::new(
            Target::Change(Slot::Custom(0)),
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        assert_eq!(
            update(&mut picker, &mut swatches, Event::Remove),
            Outcome::Changed { pick: None }
        );
        assert!(swatches.custom().is_empty());
        let mut picker = Some(Picker::new(
            Target::Change(Slot::Default(0)),
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        update(&mut picker, &mut swatches, Event::ResetDefaults);
        assert_eq!(swatches, Swatches::default());
    }

    #[test]
    fn a_translucent_color_stays_translucent() {
        let mut swatches = Swatches::default();
        let glass = Rgba::rgb(0xec, 0x48, 0x99).with_alpha(0x80);
        swatches.add(StyleId::Clean, glass);
        let mut picker = Some(Picker::new(
            Target::Change(Slot::Custom(0)),
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        assert_eq!(picker.as_ref().unwrap().hex, "#ec489980");
        update(&mut picker, &mut swatches, Event::Hue(240.0));
        assert_eq!(picker.as_ref().unwrap().color().a, 0x80);
        update(&mut picker, &mut swatches, Event::Hex("#00ff00".into()));
        assert_eq!(picker.as_ref().unwrap().color(), Rgba::rgb(0, 255, 0));
    }

    #[test]
    fn a_sampled_color_keeps_the_pickers_alpha() {
        let mut swatches = Swatches::default();
        swatches.add(StyleId::Clean, Rgba::rgb(1, 2, 3).with_alpha(0x40));
        let mut picker = Some(Picker::new(
            Target::Change(Slot::Custom(0)),
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        assert_eq!(
            update(&mut picker, &mut swatches, Event::Eyedropper),
            Outcome::Editing
        );
        assert!(picker.as_ref().unwrap().sampling());
        let sampled = Event::Sampled(Rgba::rgb(0x14, 0xb8, 0xa6));
        assert_eq!(
            update(&mut picker, &mut swatches, sampled),
            Outcome::Editing
        );
        let open = picker.as_ref().unwrap();
        assert_eq!(open.color(), Rgba::rgb(0x14, 0xb8, 0xa6).with_alpha(0x40));
        assert_eq!(open.hex, "#14b8a640");
        assert!(!open.sampling(), "one click takes one color");
    }

    #[test]
    fn canceling_changes_nothing() {
        let mut swatches = Swatches::default();
        let mut picker = Some(Picker::new(
            Target::Change(Slot::Default(0)),
            StyleId::Clean,
            &swatches,
            PALETTE[0],
        ));
        update(&mut picker, &mut swatches, Event::Hue(200.0));
        assert_eq!(
            update(&mut picker, &mut swatches, Event::Cancel),
            Outcome::Closed
        );
        assert!(picker.is_none());
        assert_eq!(swatches, Swatches::default());
    }
}
