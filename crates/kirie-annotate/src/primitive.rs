//! What a style turns a mark into: paths, text and pixel effects that every painter draws the
//! same way, so the editor shows exactly what gets saved.

use kurbo::{BezPath, Point, Rect};
use serde::{Deserialize, Serialize};

use crate::shape::Rgba;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// The typefaces Kirie ships, so text looks the same on every machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Font {
    /// Inter.
    Sans,
    /// Caveat, a handwriting face.
    Hand,
    /// Source Code Pro.
    Mono,
}

impl Font {
    pub const ALL: [Font; 3] = [Font::Sans, Font::Hand, Font::Mono];

    pub const fn family(self) -> &'static str {
        match self {
            Font::Sans => "Inter",
            Font::Hand => "Caveat",
            Font::Mono => "Source Code Pro",
        }
    }

    pub const fn name(self) -> &'static str {
        match self {
            Font::Sans => "Sans",
            Font::Hand => "Hand",
            Font::Mono => "Mono",
        }
    }

    /// How much larger to set this face for its text to look as big as the others.
    pub const fn scale(self) -> f64 {
        match self {
            Font::Hand => 1.35,
            Font::Sans | Font::Mono => 1.0,
        }
    }
}

/// How the lines of a text line up with each other.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Align {
    #[default]
    Left,
    Center,
    Right,
}

impl Align {
    pub const ALL: [Align; 3] = [Align::Left, Align::Center, Align::Right];

    /// How far into the room beside a short line it moves: none, half, all.
    pub const fn share(self) -> f64 {
        match self {
            Align::Left => 0.0,
            Align::Center => 0.5,
            Align::Right => 1.0,
        }
    }
}

/// Operations on the pixels under a region.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Effect {
    /// A Gaussian-like blur with this radius in image pixels.
    Blur { radius: f64 },
    /// Blocks of this size in image pixels.
    Pixelate { cell: f64 },
}

#[derive(Debug, Clone, PartialEq)]
pub enum Primitive {
    Fill {
        path: BezPath,
        color: Rgba,
        rule: FillRule,
    },
    /// Always with round caps and joins.
    Stroke {
        path: BezPath,
        color: Rgba,
        width: f64,
    },
    Text {
        /// The top left of the first line.
        at: Point,
        text: String,
        size: f64,
        color: Rgba,
        font: Font,
        align: Align,
        /// An outline around the glyphs, so text reads on any background.
        halo: Option<Rgba>,
        /// Center the text on `at` instead of hanging it from there.
        centered: bool,
    },
    Effect {
        rect: Rect,
        effect: Effect,
    },
    /// A picture stretched over `rect`.
    Image {
        rect: Rect,
        pixels: crate::pixels::Pixels,
    },
    /// An opaque fill with hard edges over every pixel the path reaches: what must stay hidden,
    /// with no soft edge or corner letting any of it through.
    Cover {
        path: BezPath,
        color: Rgba,
    },
}
