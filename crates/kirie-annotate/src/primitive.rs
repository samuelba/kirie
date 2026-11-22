//! What a style turns a mark into: paths, text and pixel effects that every painter draws the
//! same way, so the editor shows exactly what gets saved.

use kurbo::{BezPath, Point, Rect};

use crate::shape::Rgba;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

/// The typefaces Kirie ships, so text looks the same on every machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Font {
    /// Inter.
    Sans,
    /// Caveat, a handwriting face.
    Hand,
}

impl Font {
    pub const fn family(self) -> &'static str {
        match self {
            Font::Sans => "Inter",
            Font::Hand => "Caveat",
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
        /// An outline around the glyphs, so text reads on any background.
        halo: Option<Rgba>,
        /// Center the text on `at` instead of hanging it from there.
        centered: bool,
    },
    Effect {
        rect: Rect,
        effect: Effect,
    },
}
