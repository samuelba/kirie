//! `KIRIE_EDITOR_DEMO` marks every new image with one of each mark, for screenshots of the
//! editor.

use kirie_annotate::{
    Document, Frame, HIGHLIGHT, Kind, Op, PALETTE, Paint, Rgba, ShapeId, SpotShape, TextLook,
    Weight,
};
use kurbo::{Point, Rect};

const RED: Rgba = PALETTE[0];
const YELLOW: Rgba = PALETTE[1];
const GREEN: Rgba = PALETTE[2];
const BLUE: Rgba = PALETTE[3];
const WHITE: Rgba = PALETTE[4];
const BLACK: Rgba = PALETTE[5];

pub fn wanted() -> bool {
    std::env::var_os("KIRIE_EDITOR_DEMO").is_some()
}

/// `document` with one mark of each kind on it, laid out over the whole image.
#[must_use]
pub fn marked(document: Document) -> Document {
    let image = Image {
        width: f64::from(document.width),
        height: f64::from(document.height),
        scale: (f64::from(document.width) / document.unit / 1280.0).clamp(1.0, 3.0),
    };
    let unit = document.unit;
    let document = strokes(image)
        .into_iter()
        .chain(effects(image))
        .fold(document, |document, (kind, paint)| {
            document.apply(Op::Add { kind, paint })
        })
        .apply(Op::SetSpotlightBrightness(0.7))
        .apply(Op::SetFrame(Frame {
            margin: 40.0 * unit,
            background: Some(Rgba::rgb(0x3b, 0x4a, 0x6b)),
            shadow: true,
            overflow: false,
        }));
    let tilted = document
        .shapes()
        .find(|shape| matches!(shape.kind, Kind::Rectangle { .. }))
        .cloned();
    match tilted {
        Some(shape) => document.apply(Op::Reshape {
            id: shape.id,
            kind: shape.kind,
            rotation: -0.06,
        }),
        None => document,
    }
}

/// The mark the demo shows as picked, with its handles.
pub fn picked(document: &Document) -> Option<ShapeId> {
    document
        .shapes()
        .find(|shape| matches!(shape.kind, Kind::Ellipse { .. }))
        .map(|shape| shape.id)
}

/// Places marks by fractions of the image size.
#[derive(Clone, Copy)]
struct Image {
    width: f64,
    height: f64,
    /// How much heavier marks are than usual, so they still read on large images.
    scale: f64,
}

impl Image {
    fn at(self, x: f64, y: f64) -> Point {
        Point::new(x * self.width, y * self.height)
    }

    fn rect(self, x0: f64, y0: f64, x1: f64, y1: f64) -> Rect {
        Rect::from_points(self.at(x0, y0), self.at(x1, y1))
    }

    fn paint(self, color: Rgba, weight: f64) -> Paint {
        Paint {
            color,
            weight: Weight::new(weight * self.scale),
            ..Paint::default()
        }
    }
}

fn strokes(image: Image) -> Vec<(Kind, Paint)> {
    let squiggle = (0..=32)
        .map(|step| {
            let t = f64::from(step) / 32.0;
            image.at(
                0.06 + t * 0.34,
                0.68 + 0.03 * (t * std::f64::consts::TAU * 2.0).sin(),
            )
        })
        .collect();
    let swipe = (0..=8)
        .map(|step| image.at(0.06 + f64::from(step) * 0.0425, 0.57))
        .collect();
    let label = Paint {
        text: TextLook {
            size: 32.0 * image.scale,
            ..TextLook::default()
        },
        ..image.paint(WHITE, 4.0)
    };
    vec![
        (
            Kind::Rectangle {
                rect: image.rect(0.06, 0.10, 0.34, 0.42),
            },
            image.paint(RED, 4.0),
        ),
        (
            Kind::Counter {
                at: image.at(0.06, 0.10),
                number: 1,
            },
            image.paint(RED, 7.0),
        ),
        (
            Kind::Arrow {
                from: image.at(0.54, 0.40),
                to: image.at(0.37, 0.28),
            },
            image.paint(YELLOW, 7.0),
        ),
        (
            Kind::Text {
                at: image.at(0.40, 0.43),
                text: "Cut it out, mark it up".into(),
            },
            label,
        ),
        (
            Kind::Ellipse {
                rect: image.rect(0.66, 0.08, 0.94, 0.36),
            },
            Paint {
                filled: true,
                ..image.paint(GREEN, 4.0)
            },
        ),
        (
            Kind::Counter {
                at: image.at(0.68, 0.10),
                number: 2,
            },
            image.paint(GREEN, 7.0),
        ),
        (
            Kind::Highlighter { points: swipe },
            image.paint(HIGHLIGHT, 18.0),
        ),
        (Kind::Pen { points: squiggle }, image.paint(BLUE, 4.0)),
        (
            Kind::Line {
                from: image.at(0.06, 0.80),
                to: image.at(0.40, 0.80),
            },
            image.paint(YELLOW, 2.0),
        ),
    ]
}

fn effects(image: Image) -> Vec<(Kind, Paint)> {
    vec![
        (
            Kind::Spotlight {
                rect: image.rect(0.70, 0.74, 0.96, 0.95),
            },
            Paint {
                spot: SpotShape::Ellipse,
                ..image.paint(WHITE, 4.0)
            },
        ),
        (
            Kind::Blur {
                rect: image.rect(0.48, 0.54, 0.66, 0.68),
            },
            image.paint(WHITE, 4.0),
        ),
        (
            Kind::Pixelate {
                rect: image.rect(0.70, 0.54, 0.94, 0.68),
            },
            image.paint(WHITE, 4.0),
        ),
        (
            Kind::Counter {
                at: image.at(0.48, 0.54),
                number: 3,
            },
            image.paint(BLUE, 7.0),
        ),
        (
            Kind::Redact {
                rect: image.rect(0.48, 0.76, 0.66, 0.82),
            },
            image.paint(BLACK, 4.0),
        ),
    ]
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;

    #[test]
    fn the_demo_has_one_of_each_mark_on_the_image() {
        let document = marked(Document::new(1600, 900, 2.0));
        let kinds: HashSet<_> = document
            .shapes()
            .map(|shape| std::mem::discriminant(&shape.kind))
            .collect();
        assert_eq!(kinds.len(), 12);
        let image = Rect::new(0.0, 0.0, 1600.0, 900.0);
        for shape in document.shapes() {
            let bounds = shape.kind.bounds();
            assert_eq!(
                image.union(bounds),
                image,
                "{:?} is off the image",
                shape.kind
            );
        }
        assert!(document.frame.margin > 0.0);
        assert!(picked(&document).is_some());
    }
}
