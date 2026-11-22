//! Kirie's design language: a calm, flat dark or light surface with one vermilion accent, on a
//! 4 px grid with soft radii. The overlay and the editor both draw from these tokens.

pub mod icon;
pub mod widget;

use iced::Color;

/// Spacing steps on the 4 px grid.
pub mod space {
    pub const XS: f32 = 4.0;
    pub const S: f32 = 8.0;
    pub const M: f32 = 12.0;
    pub const L: f32 = 16.0;
    pub const XL: f32 = 24.0;
}

/// Corner radii.
pub mod radius {
    pub const CONTROL: f32 = 8.0;
    pub const CARD: f32 = 12.0;
    pub const FLOATING: f32 = 16.0;
    pub const PILL: f32 = 999.0;
}

/// Shu, the vermilion of a hanko seal.
pub const SHU: Color = rgb(0xe4, 0x57, 0x2e);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Scheme {
    #[default]
    Dark,
    Light,
}

/// The colors of one scheme.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Palette {
    pub scheme: Scheme,
    pub background: Color,
    /// Raised surfaces, from the lowest (cards) to the highest (pressed controls).
    pub surface: [Color; 3],
    pub border: Color,
    pub text: Color,
    pub muted: Color,
    pub accent: Color,
    /// Text and icons on the accent.
    pub on_accent: Color,
    pub success: Color,
    pub danger: Color,
    /// Dims what lies outside a selection on the frozen screen.
    pub scrim: Color,
}

impl Palette {
    pub const DARK: Palette = Palette {
        scheme: Scheme::Dark,
        background: rgb(0x16, 0x18, 0x1d),
        surface: [
            rgb(0x1d, 0x20, 0x27),
            rgb(0x24, 0x28, 0x32),
            rgb(0x2c, 0x31, 0x3c),
        ],
        border: rgba(0xff, 0xff, 0xff, 0.08),
        text: rgb(0xe6, 0xe8, 0xee),
        muted: rgb(0x9a, 0xa1, 0xae),
        accent: SHU,
        on_accent: rgb(0xff, 0xf8, 0xf2),
        success: rgb(0x4c, 0xc3, 0x8a),
        danger: rgb(0xef, 0x5b, 0x5b),
        scrim: rgba(0x0b, 0x0c, 0x10, 0.55),
    };

    pub const LIGHT: Palette = Palette {
        scheme: Scheme::Light,
        background: rgb(0xf4, 0xef, 0xe6),
        surface: [
            rgb(0xfb, 0xf8, 0xf2),
            rgb(0xef, 0xe8, 0xdc),
            rgb(0xe6, 0xdd, 0xce),
        ],
        border: rgba(0x1f, 0x1d, 0x1a, 0.12),
        text: rgb(0x1f, 0x1d, 0x1a),
        muted: rgb(0x6b, 0x65, 0x5c),
        accent: SHU,
        on_accent: rgb(0xff, 0xf8, 0xf2),
        success: rgb(0x2f, 0x9e, 0x6b),
        danger: rgb(0xd4, 0x3f, 0x3f),
        scrim: rgba(0x0b, 0x0c, 0x10, 0.45),
    };

    pub const fn of(scheme: Scheme) -> Palette {
        match scheme {
            Scheme::Dark => Self::DARK,
            Scheme::Light => Self::LIGHT,
        }
    }
}

const fn rgb(r: u8, g: u8, b: u8) -> Color {
    rgba(r, g, b, 1.0)
}

#[allow(clippy::cast_lossless)]
const fn rgba(r: u8, g: u8, b: u8, a: f32) -> Color {
    Color {
        r: r as f32 / 255.0,
        g: g as f32 / 255.0,
        b: b as f32 / 255.0,
        a,
    }
}

/// Black or white text, whichever reads better on `background` (WCAG relative luminance).
pub fn readable_on(background: Color) -> Color {
    let channel = |c: f32| {
        if c <= 0.039_28 {
            c / 12.92
        } else {
            ((c + 0.055) / 1.055).powf(2.4)
        }
    };
    let luminance = 0.2126 * channel(background.r)
        + 0.7152 * channel(background.g)
        + 0.0722 * channel(background.b);
    if luminance > 0.4 {
        Palette::LIGHT.text
    } else {
        Palette::DARK.text
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemes_share_the_accent() {
        assert_eq!(
            Palette::of(Scheme::Dark).accent,
            Palette::of(Scheme::Light).accent
        );
        assert_eq!(Palette::of(Scheme::Light).scheme, Scheme::Light);
    }

    #[test]
    fn text_contrasts_with_its_background() {
        assert_eq!(readable_on(Color::WHITE), Palette::LIGHT.text);
        assert_eq!(readable_on(Color::BLACK), Palette::DARK.text);
        assert_eq!(readable_on(Palette::DARK.background), Palette::DARK.text);
        assert_eq!(readable_on(Palette::LIGHT.background), Palette::LIGHT.text);
    }

    #[test]
    fn hex_conversion() {
        assert!((SHU.r - 228.0 / 255.0).abs() < f32::EPSILON);
        assert!((Palette::DARK.scrim.a - 0.55).abs() < f32::EPSILON);
    }
}
