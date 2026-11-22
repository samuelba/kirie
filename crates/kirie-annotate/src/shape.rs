//! The marks a user puts on an image.

use kurbo::{Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::primitive::{Align, Font};
use crate::style::StyleId;

/// An 8-bit sRGB color with straight alpha.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rgba {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl Rgba {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b, a: 255 }
    }

    #[must_use]
    pub const fn with_alpha(self, a: u8) -> Self {
        Self { a, ..self }
    }

    /// `#rrggbb` or `#rrggbbaa`.
    pub fn from_hex(hex: &str) -> Option<Self> {
        let hex = hex.strip_prefix('#')?;
        if !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return None;
        }
        let byte = |at: usize| u8::from_str_radix(hex.get(at..at + 2)?, 16).ok();
        match hex.len() {
            6 => Some(Self::rgb(byte(0)?, byte(2)?, byte(4)?)),
            8 => Some(Self::rgb(byte(0)?, byte(2)?, byte(4)?).with_alpha(byte(6)?)),
            _ => None,
        }
    }

    pub fn to_hex(self) -> String {
        if self.a == 255 {
            format!("#{:02x}{:02x}{:02x}", self.r, self.g, self.b)
        } else {
            format!("#{:02x}{:02x}{:02x}{:02x}", self.r, self.g, self.b, self.a)
        }
    }
}

/// How heavy a mark is: its stroke width in logical pixels before the style and the image
/// scale apply.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(transparent)]
pub struct Weight(f64);

impl Weight {
    pub const THIN: Weight = Weight(2.0);
    pub const MEDIUM: Weight = Weight(4.0);
    pub const THICK: Weight = Weight(7.0);
    pub const MIN: f64 = 1.0;
    pub const MAX: f64 = 24.0;

    pub fn new(width: f64) -> Self {
        if width.is_finite() {
            Self(width.clamp(Self::MIN, Self::MAX))
        } else {
            Self::MEDIUM
        }
    }

    pub const fn base(self) -> f64 {
        self.0
    }

    /// The text size a weight gave before text had a size of its own: 16, 22 and 32 for the
    /// thin, medium and thick weights there were then.
    fn legacy_text_size(self) -> f64 {
        if self.0 <= 4.0 {
            16.0 + (self.0 - 2.0) * 3.0
        } else {
            22.0 + (self.0 - 4.0) * 10.0 / 3.0
        }
    }
}

impl Default for Weight {
    fn default() -> Self {
        Self::MEDIUM
    }
}

impl<'de> Deserialize<'de> for Weight {
    /// A width, or the name of one of the three weights older projects knew.
    fn deserialize<D: serde::Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Stored {
            Width(f64),
            Named(Named),
        }
        #[derive(Deserialize)]
        enum Named {
            Thin,
            Medium,
            Thick,
        }
        Ok(match Stored::deserialize(from)? {
            Stored::Width(width) => Self::new(width),
            Stored::Named(Named::Thin) => Self::THIN,
            Stored::Named(Named::Medium) => Self::MEDIUM,
            Stored::Named(Named::Thick) => Self::THICK,
        })
    }
}

/// The standard colors as the clean style shades them, the first being the default; the other
/// styles shade them their own way, see [`StyleId::palette`].
pub const PALETTE: [Rgba; 6] = [
    Rgba::rgb(0xe4, 0x57, 0x2e),
    Rgba::rgb(0xf2, 0xb1, 0x34),
    Rgba::rgb(0x2f, 0xb3, 0x72),
    Rgba::rgb(0x3b, 0x82, 0xf6),
    Rgba::rgb(0xff, 0xff, 0xff),
    Rgba::rgb(0x16, 0x18, 0x1d),
];

/// The color the highlighter starts with.
pub const HIGHLIGHT: Rgba = Rgba::rgb(0xf2, 0xd1, 0x3a);

/// How text marks look, besides their color.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct TextLook {
    /// The size in logical pixels.
    pub size: f64,
    /// `None` sets text in the style's own face.
    pub font: Option<Font>,
    pub align: Align,
    /// An outline in a contrasting color, so the text reads on any background.
    pub outlined: bool,
}

impl TextLook {
    pub const MIN_SIZE: f64 = 8.0;
    pub const MAX_SIZE: f64 = 120.0;
}

impl Default for TextLook {
    fn default() -> Self {
        Self {
            size: 22.0,
            font: None,
            align: Align::Left,
            outlined: true,
        }
    }
}

/// The outline a spotlight lights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum SpotShape {
    #[default]
    Rectangle,
    Ellipse,
}

/// The paint a mark was made with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(from = "StoredPaint")]
pub struct Paint {
    pub color: Rgba,
    pub weight: Weight,
    /// Closed shapes are filled with a translucent tint of the color.
    pub filled: bool,
    pub text: TextLook,
    pub spot: SpotShape,
}

impl Default for Paint {
    fn default() -> Self {
        Self {
            color: PALETTE[0],
            weight: Weight::MEDIUM,
            filled: false,
            text: TextLook::default(),
            spot: SpotShape::default(),
        }
    }
}

/// A paint as projects keep it; older ones sized text by the weight.
#[derive(Deserialize)]
struct StoredPaint {
    color: Rgba,
    weight: Weight,
    filled: bool,
    #[serde(default, deserialize_with = "present")]
    text: Option<TextLook>,
    #[serde(default)]
    spot: SpotShape,
}

fn present<'de, D: serde::Deserializer<'de>>(from: D) -> Result<Option<TextLook>, D::Error> {
    TextLook::deserialize(from).map(Some)
}

impl From<StoredPaint> for Paint {
    fn from(stored: StoredPaint) -> Self {
        Self {
            color: stored.color,
            weight: stored.weight,
            filled: stored.filled,
            text: stored.text.unwrap_or(TextLook {
                size: stored.weight.legacy_text_size(),
                ..TextLook::default()
            }),
            spot: stored.spot,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub struct ShapeId(pub u64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Kind {
    Arrow {
        from: Point,
        to: Point,
    },
    Line {
        from: Point,
        to: Point,
    },
    Rectangle {
        rect: Rect,
    },
    Ellipse {
        rect: Rect,
    },
    Pen {
        points: Vec<Point>,
    },
    Highlighter {
        points: Vec<Point>,
    },
    Text {
        at: Point,
        text: String,
    },
    /// A numbered badge for step-by-step instructions.
    Counter {
        at: Point,
        number: u32,
    },
    Blur {
        rect: Rect,
    },
    Pixelate {
        rect: Rect,
    },
    /// A solid block over something secret.
    Redact {
        rect: Rect,
    },
    /// Dims everything but this region.
    Spotlight {
        rect: Rect,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Shape {
    pub id: ShapeId,
    pub kind: Kind,
    pub paint: Paint,
    /// Overrides the document's style for this mark.
    pub style: Option<StyleId>,
    /// Turn in radians around the center of the mark's bounds; only boxes and ellipses turn.
    #[serde(default)]
    pub rotation: f64,
}

impl Shape {
    /// The turn about the mark's center, as an affine map.
    pub fn turn(&self) -> kurbo::Affine {
        if self.rotation == 0.0 {
            kurbo::Affine::IDENTITY
        } else {
            kurbo::Affine::rotate_about(self.rotation, self.kind.bounds().center())
        }
    }

    /// How far `point` is from the mark, taking its turn into account.
    pub fn distance(&self, point: Point) -> f64 {
        let point = self.turn().inverse() * point;
        match self.kind {
            Kind::Spotlight { rect } if self.paint.spot == SpotShape::Ellipse => {
                Kind::Ellipse { rect }.distance(point)
            }
            _ => self.kind.distance(point),
        }
    }
}

impl Kind {
    /// The area the mark covers, before stroke width. Text covers only its anchor here: how
    /// far it reaches depends on its face and size, which [`Document::bounds`] measures.
    ///
    /// [`Document::bounds`]: crate::Document::bounds
    pub fn bounds(&self) -> Rect {
        match self {
            Kind::Arrow { from, to } | Kind::Line { from, to } => Rect::from_points(*from, *to),
            Kind::Rectangle { rect }
            | Kind::Ellipse { rect }
            | Kind::Blur { rect }
            | Kind::Pixelate { rect }
            | Kind::Redact { rect }
            | Kind::Spotlight { rect } => rect.abs(),
            Kind::Pen { points } | Kind::Highlighter { points } => points_bounds(points),
            Kind::Text { at, .. } => Rect::from_origin_size(*at, (0.0, 0.0)),
            Kind::Counter { at, .. } => Rect::from_center_size(*at, (28.0, 28.0)),
        }
    }

    /// The same mark moved by `by`.
    #[must_use]
    pub fn translated(&self, by: Vec2) -> Kind {
        let points = |points: &[Point]| points.iter().map(|point| *point + by).collect();
        match self {
            Kind::Arrow { from, to } => Kind::Arrow {
                from: *from + by,
                to: *to + by,
            },
            Kind::Line { from, to } => Kind::Line {
                from: *from + by,
                to: *to + by,
            },
            Kind::Rectangle { rect } => Kind::Rectangle { rect: *rect + by },
            Kind::Ellipse { rect } => Kind::Ellipse { rect: *rect + by },
            Kind::Blur { rect } => Kind::Blur { rect: *rect + by },
            Kind::Pixelate { rect } => Kind::Pixelate { rect: *rect + by },
            Kind::Redact { rect } => Kind::Redact { rect: *rect + by },
            Kind::Spotlight { rect } => Kind::Spotlight { rect: *rect + by },
            Kind::Pen { points: p } => Kind::Pen { points: points(p) },
            Kind::Highlighter { points: p } => Kind::Highlighter { points: points(p) },
            Kind::Text { at, text } => Kind::Text {
                at: *at + by,
                text: text.clone(),
            },
            Kind::Counter { at, number } => Kind::Counter {
                at: *at + by,
                number: *number,
            },
        }
    }

    /// The same mark under a scale and translation, as when desktop coordinates become image
    /// pixels.
    #[must_use]
    pub fn transformed(&self, transform: kurbo::Affine) -> Kind {
        let point = |point: &Point| transform * *point;
        let rect = |rect: &Rect| transform.transform_rect_bbox(rect.abs());
        let points = |points: &[Point]| points.iter().map(point).collect();
        match self {
            Kind::Arrow { from, to } => Kind::Arrow {
                from: point(from),
                to: point(to),
            },
            Kind::Line { from, to } => Kind::Line {
                from: point(from),
                to: point(to),
            },
            Kind::Rectangle { rect: r } => Kind::Rectangle { rect: rect(r) },
            Kind::Ellipse { rect: r } => Kind::Ellipse { rect: rect(r) },
            Kind::Blur { rect: r } => Kind::Blur { rect: rect(r) },
            Kind::Pixelate { rect: r } => Kind::Pixelate { rect: rect(r) },
            Kind::Redact { rect: r } => Kind::Redact { rect: rect(r) },
            Kind::Spotlight { rect: r } => Kind::Spotlight { rect: rect(r) },
            Kind::Pen { points: p } => Kind::Pen { points: points(p) },
            Kind::Highlighter { points: p } => Kind::Highlighter { points: points(p) },
            Kind::Text { at, text } => Kind::Text {
                at: point(at),
                text: text.clone(),
            },
            Kind::Counter { at, number } => Kind::Counter {
                at: point(at),
                number: *number,
            },
        }
    }

    /// Whether the mark works on the pixels under it rather than drawing on top.
    pub const fn is_effect(&self) -> bool {
        matches!(
            self,
            Kind::Blur { .. } | Kind::Pixelate { .. } | Kind::Spotlight { .. }
        )
    }

    /// How far `point` is from the mark, 0 when on or inside it.
    pub fn distance(&self, point: Point) -> f64 {
        match self {
            Kind::Arrow { from, to } | Kind::Line { from, to } => {
                segment_distance(point, *from, *to)
            }
            Kind::Ellipse { rect } => {
                let rect = rect.abs();
                let center = rect.center();
                let (rx, ry) = (rect.width() / 2.0, rect.height() / 2.0);
                if rx <= 0.0 || ry <= 0.0 {
                    return point.distance(center);
                }
                let normalized = ((point.x - center.x) / rx).hypot((point.y - center.y) / ry);
                (normalized - 1.0).max(0.0) * rx.min(ry)
            }
            Kind::Pen { points } | Kind::Highlighter { points } => match points.as_slice() {
                [] => f64::INFINITY,
                [only] => point.distance(*only),
                points => points
                    .windows(2)
                    .map(|pair| segment_distance(point, pair[0], pair[1]))
                    .fold(f64::INFINITY, f64::min),
            },
            kind => rect_distance(kind.bounds(), point),
        }
    }
}

fn points_bounds(points: &[Point]) -> Rect {
    let Some((first, rest)) = points.split_first() else {
        return Rect::ZERO;
    };
    rest.iter()
        .fold(Rect::from_points(*first, *first), |bounds, point| {
            bounds.union_pt(*point)
        })
}

fn segment_distance(point: Point, a: Point, b: Point) -> f64 {
    let ab = b - a;
    let length = ab.hypot2();
    if length == 0.0 {
        return point.distance(a);
    }
    let t = ((point - a).dot(ab) / length).clamp(0.0, 1.0);
    point.distance(a + ab * t)
}

fn rect_distance(rect: Rect, point: Point) -> f64 {
    let dx = (rect.x0 - point.x).max(point.x - rect.x1).max(0.0);
    let dy = (rect.y0 - point.y).max(point.y - rect.y1).max(0.0);
    dx.hypot(dy)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit() -> Rect {
        Rect::new(0.0, 0.0, 1.0, 1.0)
    }

    #[test]
    fn hex_round_trip() {
        let shu = Rgba::from_hex("#e4572e").unwrap();
        assert_eq!(shu, Rgba::rgb(0xe4, 0x57, 0x2e));
        assert_eq!(shu.to_hex(), "#e4572e");
        let glass = Rgba::from_hex("#00000080").unwrap();
        assert_eq!(glass.a, 0x80);
        assert_eq!(glass.to_hex(), "#00000080");
        assert_eq!(Rgba::from_hex("e4572e"), None);
        assert_eq!(Rgba::from_hex("#e457"), None);
        assert_eq!(Rgba::from_hex("#zz572e"), None);
        assert_eq!(Rgba::from_hex("#+1+2+3"), None);
    }

    #[test]
    fn older_paints_size_text_by_weight() {
        let old = "(color: (r: 1, g: 2, b: 3, a: 255), weight: Thick, filled: true)";
        let paint: Paint = ron::from_str(old).unwrap();
        assert!((paint.text.size - 32.0).abs() < f64::EPSILON);
        assert!(paint.text.outlined && paint.text.font.is_none());
        assert_eq!(paint.spot, SpotShape::Rectangle);
        let oval = Paint {
            spot: SpotShape::Ellipse,
            ..Paint::default()
        };
        let saved = ron::to_string(&oval).unwrap();
        assert_eq!(ron::from_str::<Paint>(&saved).unwrap(), oval);
    }

    #[test]
    fn oval_spotlights_are_hit_inside_the_ellipse() {
        let spotlight = |spot| Shape {
            id: ShapeId(1),
            kind: Kind::Spotlight {
                rect: Rect::new(0.0, 0.0, 20.0, 20.0),
            },
            paint: Paint {
                spot,
                ..Paint::default()
            },
            style: None,
            rotation: 0.0,
        };
        let corner = Point::new(1.0, 1.0);
        assert!(spotlight(SpotShape::Rectangle).distance(corner) < 1e-9);
        assert!(spotlight(SpotShape::Ellipse).distance(corner) > 1.0);
    }

    #[test]
    fn weights_keep_any_width_within_bounds() {
        assert!((Weight::new(5.5).base() - 5.5).abs() < f64::EPSILON);
        assert!((Weight::new(0.0).base() - Weight::MIN).abs() < f64::EPSILON);
        assert!((Weight::new(99.0).base() - Weight::MAX).abs() < f64::EPSILON);
        assert_eq!(Weight::new(f64::NAN), Weight::MEDIUM);
        for (old, weight) in [
            ("Thin", Weight::THIN),
            ("Medium", Weight::MEDIUM),
            ("Thick", Weight::THICK),
        ] {
            assert_eq!(ron::from_str::<Weight>(old).unwrap(), weight);
        }
        assert_eq!(ron::to_string(&Weight::new(5.5)).unwrap(), "5.5");
        assert_eq!(ron::from_str::<Weight>("5.5").unwrap(), Weight::new(5.5));
        assert_eq!(
            ron::from_str::<Weight>("50").unwrap(),
            Weight::new(Weight::MAX)
        );
        assert!((Weight::MEDIUM.legacy_text_size() - 22.0).abs() < 1e-9);
        assert!((Weight::THICK.legacy_text_size() - 32.0).abs() < 1e-9);
    }

    #[test]
    fn weights_grow() {
        assert!(Weight::THIN.base() < Weight::MEDIUM.base());
        assert!(Weight::MEDIUM.base() < Weight::THICK.base());
        assert_eq!(Weight::default(), Weight::MEDIUM);
        assert!(!Paint::default().filled);
    }

    #[test]
    fn bounds_of_every_kind() {
        let a = Point::new(10.0, 20.0);
        let b = Point::new(0.0, 5.0);
        assert_eq!(
            Kind::Arrow { from: a, to: b }.bounds(),
            Rect::new(0.0, 5.0, 10.0, 20.0)
        );
        let flipped = Rect::new(10.0, 10.0, 0.0, 0.0);
        assert_eq!(
            Kind::Blur { rect: flipped }.bounds(),
            Rect::new(0.0, 0.0, 10.0, 10.0)
        );
        let pen = Kind::Pen {
            points: vec![a, b, Point::new(30.0, 0.0)],
        };
        assert_eq!(pen.bounds(), Rect::new(0.0, 0.0, 30.0, 20.0));
        assert_eq!(Kind::Pen { points: vec![] }.bounds(), Rect::ZERO);
        assert_eq!(Kind::Counter { at: a, number: 1 }.bounds().center(), a);
        let text = Kind::Text {
            at: a,
            text: "ab\nc".into(),
        };
        assert_eq!(text.bounds(), Rect::from_origin_size(a, (0.0, 0.0)));
    }

    #[test]
    fn translation_moves_every_point() {
        let by = Vec2::new(5.0, -5.0);
        let (o, one) = (Point::ORIGIN, Point::new(1.0, 1.0));
        let kinds = [
            Kind::Arrow { from: o, to: one },
            Kind::Line { from: o, to: one },
            Kind::Rectangle { rect: unit() },
            Kind::Ellipse { rect: unit() },
            Kind::Pen { points: vec![o] },
            Kind::Highlighter { points: vec![o] },
            Kind::Text {
                at: o,
                text: "x".into(),
            },
            Kind::Counter { at: o, number: 3 },
            Kind::Blur { rect: unit() },
            Kind::Pixelate { rect: unit() },
            Kind::Redact { rect: unit() },
            Kind::Spotlight { rect: unit() },
        ];
        for kind in kinds {
            let moved = kind.translated(by);
            assert_eq!(moved.bounds().origin(), kind.bounds().origin() + by);
        }
    }

    #[test]
    fn transforms_scale_and_move_every_kind() {
        let transform = kurbo::Affine::scale(2.0).then_translate(Vec2::new(-10.0, 0.0));
        let kinds = [
            Kind::Arrow {
                from: Point::new(5.0, 0.0),
                to: Point::new(10.0, 5.0),
            },
            Kind::Line {
                from: Point::new(5.0, 0.0),
                to: Point::new(10.0, 5.0),
            },
            Kind::Rectangle {
                rect: Rect::new(5.0, 0.0, 10.0, 5.0),
            },
            Kind::Ellipse {
                rect: Rect::new(5.0, 0.0, 10.0, 5.0),
            },
            Kind::Pen {
                points: vec![Point::new(5.0, 0.0), Point::new(10.0, 5.0)],
            },
            Kind::Highlighter {
                points: vec![Point::new(5.0, 0.0), Point::new(10.0, 5.0)],
            },
            Kind::Blur {
                rect: Rect::new(5.0, 0.0, 10.0, 5.0),
            },
            Kind::Pixelate {
                rect: Rect::new(5.0, 0.0, 10.0, 5.0),
            },
            Kind::Redact {
                rect: Rect::new(5.0, 0.0, 10.0, 5.0),
            },
            Kind::Spotlight {
                rect: Rect::new(5.0, 0.0, 10.0, 5.0),
            },
        ];
        for kind in kinds {
            assert_eq!(
                kind.transformed(transform).bounds(),
                Rect::new(0.0, 0.0, 10.0, 10.0)
            );
        }
        let Kind::Counter { at, number } = (Kind::Counter {
            at: Point::new(5.0, 1.0),
            number: 4,
        })
        .transformed(transform) else {
            panic!()
        };
        assert_eq!((at, number), (Point::new(0.0, 2.0), 4));
        let Kind::Text { at, .. } = (Kind::Text {
            at: Point::new(5.0, 1.0),
            text: "a".into(),
        })
        .transformed(transform) else {
            panic!()
        };
        assert_eq!(at, Point::new(0.0, 2.0));
    }

    #[test]
    fn distances() {
        let line = Kind::Line {
            from: Point::new(0.0, 0.0),
            to: Point::new(10.0, 0.0),
        };
        assert!((line.distance(Point::new(5.0, 3.0)) - 3.0).abs() < 1e-9);
        assert!((line.distance(Point::new(13.0, 4.0)) - 5.0).abs() < 1e-9);
        let rect = Kind::Rectangle {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        };
        assert!(rect.distance(Point::new(5.0, 5.0)) < 1e-9);
        assert!((rect.distance(Point::new(13.0, 14.0)) - 5.0).abs() < 1e-9);
        let ellipse = Kind::Ellipse {
            rect: Rect::new(0.0, 0.0, 20.0, 20.0),
        };
        assert!(ellipse.distance(Point::new(10.0, 10.0)) < 1e-9);
        assert!((ellipse.distance(Point::new(10.0, 30.0)) - 10.0).abs() < 1e-9);
        let flat = Kind::Ellipse {
            rect: Rect::new(0.0, 0.0, 0.0, 10.0),
        };
        assert!((flat.distance(Point::new(0.0, 5.0))).abs() < 1e-9);
        let pen = Kind::Pen {
            points: vec![Point::ORIGIN, Point::new(0.0, 10.0)],
        };
        assert!((pen.distance(Point::new(2.0, 5.0)) - 2.0).abs() < 1e-9);
        let dot = Kind::Pen {
            points: vec![Point::ORIGIN],
        };
        assert!((dot.distance(Point::new(3.0, 4.0)) - 5.0).abs() < 1e-9);
        assert!(
            Kind::Pen { points: vec![] }
                .distance(Point::ORIGIN)
                .is_infinite()
        );
        let point_line = Kind::Line {
            from: Point::ORIGIN,
            to: Point::ORIGIN,
        };
        assert!((point_line.distance(Point::new(3.0, 4.0)) - 5.0).abs() < 1e-9);
        assert!(Kind::Blur { rect: unit() }.is_effect());
        assert!(!Kind::Redact { rect: unit() }.is_effect());
    }
}
