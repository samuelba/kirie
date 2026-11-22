//! The marks a user puts on an image.

use kurbo::{Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::style::StyleId;

/// An 8-bit sRGB colour with straight alpha.
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

/// How heavy a mark is; each style turns it into a width.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum Weight {
    Thin,
    #[default]
    Medium,
    Thick,
}

impl Weight {
    /// The stroke width in logical pixels before the style and the image scale apply.
    pub const fn base(self) -> f64 {
        match self {
            Weight::Thin => 2.0,
            Weight::Medium => 4.0,
            Weight::Thick => 7.0,
        }
    }
}

/// The paint a mark was made with.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Paint {
    pub color: Rgba,
    pub weight: Weight,
    /// Closed shapes are filled with a translucent tint of the colour.
    pub filled: bool,
}

impl Default for Paint {
    fn default() -> Self {
        Self {
            color: Rgba::rgb(0xe4, 0x57, 0x2e),
            weight: Weight::Medium,
            filled: false,
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
}

impl Kind {
    /// The area the mark covers, before stroke width.
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
            Kind::Text { at, text } => {
                let longest = text.lines().map(|line| line.chars().count()).max();
                let lines = text.lines().count().max(1);
                #[allow(clippy::cast_precision_loss)]
                let size = (longest.unwrap_or(0) as f64 * 10.0, lines as f64 * 20.0);
                Rect::from_origin_size(*at, size)
            }
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
    }

    #[test]
    fn weights_grow() {
        assert!(Weight::Thin.base() < Weight::Medium.base());
        assert!(Weight::Medium.base() < Weight::Thick.base());
        assert_eq!(Weight::default(), Weight::Medium);
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
            at: Point::ORIGIN,
            text: "ab\nc".into(),
        };
        assert_eq!(text.bounds(), Rect::new(0.0, 0.0, 20.0, 40.0));
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
