//! Annotation styles: the same marks drawn crisp and clean, sketched by hand, or brushed in ink.

mod clean;
mod ink;
mod sketch;

use kurbo::{BezPath, Point, Rect, Shape as _, Vec2};
use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::primitive::{Effect, FillRule, Primitive};
use crate::shape::{Kind, Rgba, Shape};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StyleId {
    /// Crisp vectors with a soft shadow.
    #[default]
    Clean,
    /// Hand-drawn lines and hatching, with handwritten text.
    Sketch,
    /// Brush strokes that swell and taper, like sumi ink.
    Ink,
}

impl StyleId {
    pub const ALL: [StyleId; 3] = [StyleId::Clean, StyleId::Sketch, StyleId::Ink];

    pub const fn name(self) -> &'static str {
        match self {
            StyleId::Clean => "Clean",
            StyleId::Sketch => "Sketch",
            StyleId::Ink => "Ink",
        }
    }
}

/// Everything the document's marks draw, bottom to top, in image coordinates.
pub fn render_list(document: &Document) -> Vec<Primitive> {
    document
        .shapes()
        .flat_map(|shape| primitives(shape, document.style, document.unit, document.visible()))
        .collect()
}

/// What one mark draws. `visible` is the image area shown, which a spotlight dims around.
pub fn primitives(shape: &Shape, style: StyleId, unit: f64, visible: Rect) -> Vec<Primitive> {
    let drawn = unturned(shape, style, unit, visible);
    if shape.rotation == 0.0 {
        return drawn;
    }
    let turn = shape.turn();
    drawn
        .into_iter()
        .map(|primitive| match primitive {
            Primitive::Fill { path, color, rule } => Primitive::Fill {
                path: turn * path,
                color,
                rule,
            },
            Primitive::Stroke { path, color, width } => Primitive::Stroke {
                path: turn * path,
                color,
                width,
            },
            other => other,
        })
        .collect()
}

fn unturned(shape: &Shape, style: StyleId, unit: f64, visible: Rect) -> Vec<Primitive> {
    let context = Context {
        paint: shape.paint,
        unit,
        seed: shape.id.0,
    };
    match &shape.kind {
        Kind::Blur { rect } => vec![Primitive::Effect {
            rect: rect.abs(),
            effect: Effect::Blur {
                radius: 10.0 * unit,
            },
        }],
        Kind::Pixelate { rect } => vec![Primitive::Effect {
            rect: rect.abs(),
            effect: Effect::Pixelate { cell: 12.0 * unit },
        }],
        Kind::Spotlight { rect } => vec![spotlight(visible, rect.abs(), 6.0 * unit)],
        kind => match shape.style.unwrap_or(style) {
            StyleId::Clean => clean::primitives(kind, &context),
            StyleId::Sketch => sketch::primitives(kind, &context),
            StyleId::Ink => ink::primitives(kind, &context),
        },
    }
}

/// What a style needs to know about the mark it draws.
#[derive(Debug, Clone, Copy)]
struct Context {
    paint: crate::shape::Paint,
    unit: f64,
    /// Makes hand-drawn wobble the same on every redraw of the same mark.
    seed: u64,
}

impl Context {
    fn width(&self) -> f64 {
        self.paint.weight.base() * self.unit
    }

    fn color(&self) -> Rgba {
        self.paint.color
    }

    fn tint(&self) -> Rgba {
        self.paint.color.with_alpha(0x40)
    }

    fn text_size(&self) -> f64 {
        let size = match self.paint.weight {
            crate::shape::Weight::Thin => 16.0,
            crate::shape::Weight::Medium => 22.0,
            crate::shape::Weight::Thick => 32.0,
        };
        size * self.unit
    }

    fn counter_radius(&self) -> f64 {
        (10.0 + self.paint.weight.base() * 1.5) * self.unit
    }
}

/// A highlighter stroke: the pointer's path smoothed out, drawn as one band of even width with
/// flat ends, filled once so it never darkens where it crosses itself.
fn marker(points: &[Point], context: &Context) -> Primitive {
    let smoothed = chaikin(&thin_out(points, 2.0 * context.unit), 2);
    Primitive::Fill {
        path: ink::brush(
            &smoothed,
            context.width() * 4.5,
            ink::Taper {
                start: 0.0,
                end: 0.0,
            },
        ),
        color: context.color().with_alpha(0x70),
        rule: FillRule::NonZero,
    }
}

/// Rounds the corners of a polyline `rounds` times, keeping its ends.
fn chaikin(points: &[Point], rounds: usize) -> Vec<Point> {
    let mut points = points.to_vec();
    for _ in 0..rounds {
        if points.len() < 3 {
            break;
        }
        let mut next = Vec::with_capacity(points.len() * 2);
        next.push(points[0]);
        for pair in points.windows(2) {
            next.push(pair[0].lerp(pair[1], 0.25));
            next.push(pair[0].lerp(pair[1], 0.75));
        }
        next.push(points[points.len() - 1]);
        points = next;
    }
    points
}

fn spotlight(visible: Rect, hole: Rect, radius: f64) -> Primitive {
    let mut path = visible.to_path(0.1);
    path.extend(kurbo::RoundedRect::from_rect(hole, radius).path_elements(0.1));
    Primitive::Fill {
        path,
        color: Rgba::rgb(0, 0, 0).with_alpha(0x8c),
        rule: FillRule::EvenOdd,
    }
}

/// A dark or light outline colour that sets `color` apart from any background.
fn halo_for(color: Rgba) -> Rgba {
    let luminance =
        0.2126 * f64::from(color.r) + 0.7152 * f64::from(color.g) + 0.0722 * f64::from(color.b);
    if luminance > 150.0 {
        Rgba::rgb(0x16, 0x18, 0x1d).with_alpha(0xd0)
    } else {
        Rgba::rgb(0xff, 0xff, 0xff).with_alpha(0xe6)
    }
}

/// Text that reads on `fill`.
fn text_on(fill: Rgba) -> Rgba {
    if halo_for(fill).r == 0xff {
        Rgba::rgb(0xff, 0xff, 0xff)
    } else {
        Rgba::rgb(0x16, 0x18, 0x1d)
    }
}

/// The two back corners of an arrow head of `length` pointing at `tip` from `from`.
fn arrow_wings(from: Point, tip: Point, length: f64, spread: f64) -> Option<(Point, Point)> {
    let direction = tip - from;
    let distance = direction.hypot();
    if distance < f64::EPSILON {
        return None;
    }
    let back = direction / distance * -length;
    let (sin, cos) = spread.sin_cos();
    let rotate = |v: Vec2, sin: f64| Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
    Some((tip + rotate(back, sin), tip + rotate(back, -sin)))
}

/// A smooth curve through `points` (Catmull-Rom turned into cubic Béziers).
fn smooth(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    let Some(first) = points.first() else {
        return path;
    };
    path.move_to(*first);
    if points.len() == 1 {
        path.line_to(*first);
        return path;
    }
    for index in 0..points.len() - 1 {
        let p0 = points[index.saturating_sub(1)];
        let p1 = points[index];
        let p2 = points[index + 1];
        let p3 = points[(index + 2).min(points.len() - 1)];
        let c1 = p1 + (p2 - p0) / 6.0;
        let c2 = p2 - (p3 - p1) / 6.0;
        path.curve_to(c1, c2, p2);
    }
    path
}

/// `count + 1` points along an ellipse from `start` radians over `sweep` radians.
fn ellipse_points(rect: Rect, count: usize, start: f64, sweep: f64) -> Vec<Point> {
    let center = rect.center();
    let (rx, ry) = (rect.width() / 2.0, rect.height() / 2.0);
    #[allow(clippy::cast_precision_loss)]
    (0..=count)
        .map(|index| {
            let angle = start + sweep * index as f64 / count as f64;
            Point::new(center.x + rx * angle.cos(), center.y + ry * angle.sin())
        })
        .collect()
}

/// Drops points closer than `min` to the previous kept one; the last point always stays.
fn thin_out(points: &[Point], min: f64) -> Vec<Point> {
    let mut kept: Vec<Point> = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let last = index == points.len() - 1;
        if kept
            .last()
            .is_none_or(|previous| previous.distance(*point) >= min)
            || last
        {
            kept.push(*point);
        }
    }
    kept
}

/// A small, fast generator; seeded per mark so its wobble never changes.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `-1.0..1.0`.
    #[allow(clippy::cast_precision_loss)]
    fn signed(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Op;
    use crate::shape::{Paint, ShapeId, Weight};

    fn every_kind() -> Vec<Kind> {
        let rect = Rect::new(10.0, 10.0, 60.0, 40.0);
        let (a, b) = (Point::new(5.0, 5.0), Point::new(80.0, 50.0));
        vec![
            Kind::Arrow { from: a, to: b },
            Kind::Line { from: a, to: b },
            Kind::Rectangle { rect },
            Kind::Ellipse { rect },
            Kind::Pen {
                points: vec![a, Point::new(20.0, 30.0), b],
            },
            Kind::Highlighter { points: vec![a, b] },
            Kind::Text {
                at: a,
                text: "Hi".into(),
            },
            Kind::Counter { at: a, number: 2 },
            Kind::Blur { rect },
            Kind::Pixelate { rect },
            Kind::Redact { rect },
            Kind::Spotlight { rect },
        ]
    }

    #[test]
    fn every_style_draws_every_kind() {
        for style in StyleId::ALL {
            for filled in [false, true] {
                for kind in every_kind() {
                    let shape = Shape {
                        id: ShapeId(7),
                        kind: kind.clone(),
                        paint: Paint {
                            filled,
                            ..Paint::default()
                        },
                        style: None,
                        rotation: 0.0,
                    };
                    let visible = Rect::new(0.0, 0.0, 100.0, 100.0);
                    let drawn = primitives(&shape, style, 1.0, visible);
                    assert!(!drawn.is_empty(), "{style:?} drew nothing for {kind:?}");
                }
            }
        }
    }

    #[test]
    fn drawing_is_deterministic() {
        let document = Document::new(100, 100, 1.0)
            .apply(Op::SetStyle(StyleId::Sketch))
            .apply(Op::Add {
                kind: Kind::Rectangle {
                    rect: Rect::new(10.0, 10.0, 50.0, 50.0),
                },
                paint: Paint::default(),
            });
        assert_eq!(render_list(&document), render_list(&document));
    }

    #[test]
    fn a_marks_own_style_wins() {
        let shape = Shape {
            id: ShapeId(1),
            kind: Kind::Text {
                at: Point::ORIGIN,
                text: "x".into(),
            },
            paint: Paint::default(),
            style: Some(StyleId::Sketch),
            rotation: 0.0,
        };
        let drawn = primitives(&shape, StyleId::Clean, 1.0, Rect::ZERO);
        assert!(drawn.iter().any(|primitive| matches!(
            primitive,
            Primitive::Text {
                font: crate::primitive::Font::Hand,
                ..
            }
        )));
    }

    #[test]
    fn styles_have_names() {
        for style in StyleId::ALL {
            assert!(!style.name().is_empty());
        }
    }

    #[test]
    fn markers_are_smooth_even_bands() {
        let shaky: Vec<Point> = (0..40)
            .map(|i| Point::new(f64::from(i) * 5.0, if i % 2 == 0 { 0.0 } else { 1.5 }))
            .collect();
        let smoothed = chaikin(&shaky, 2);
        let wobble = |points: &[Point]| {
            points
                .windows(2)
                .map(|p| (p[1].y - p[0].y).abs())
                .fold(0.0, f64::max)
        };
        assert!(wobble(&smoothed) < wobble(&shaky));
        assert_eq!(smoothed.first(), shaky.first());
        assert_eq!(smoothed.last(), shaky.last());
        assert_eq!(chaikin(&shaky[..2], 3), shaky[..2].to_vec());
    }

    #[test]
    fn halos_contrast() {
        assert_eq!(halo_for(Rgba::rgb(255, 255, 255)).r, 0x16);
        assert_eq!(halo_for(Rgba::rgb(0, 0, 0)).r, 0xff);
        assert_eq!(text_on(Rgba::rgb(0xe4, 0x57, 0x2e)).r, 0xff);
        assert_eq!(text_on(Rgba::rgb(0xf4, 0xef, 0xe6)).r, 0x16);
    }

    #[test]
    fn arrow_wings_are_symmetric() {
        let (left, right) = arrow_wings(Point::ORIGIN, Point::new(10.0, 0.0), 5.0, 0.5).unwrap();
        assert!((left.x - right.x).abs() < 1e-9);
        assert!((left.y + right.y).abs() < 1e-9);
        assert!(left.x < 10.0);
        assert!(arrow_wings(Point::ORIGIN, Point::ORIGIN, 5.0, 0.5).is_none());
    }

    #[test]
    fn helpers() {
        assert!(smooth(&[]).elements().is_empty());
        assert_eq!(smooth(&[Point::ORIGIN]).elements().len(), 2);
        let curve = smooth(&[Point::ORIGIN, Point::new(1.0, 1.0), Point::new(2.0, 0.0)]);
        assert_eq!(curve.elements().len(), 3);
        let ring = ellipse_points(Rect::new(0.0, 0.0, 2.0, 2.0), 4, 0.0, std::f64::consts::TAU);
        assert_eq!(ring.len(), 5);
        assert!((ring[0].x - 2.0).abs() < 1e-9);
        let kept = thin_out(
            &[
                Point::ORIGIN,
                Point::new(0.1, 0.0),
                Point::new(5.0, 0.0),
                Point::new(5.1, 0.0),
            ],
            1.0,
        );
        assert_eq!(kept.len(), 3);
        let mut rng = Rng::new(1);
        let values: Vec<f64> = (0..100).map(|_| rng.signed()).collect();
        assert!(values.iter().all(|v| (-1.0..1.0).contains(v)));
        assert!(values.iter().any(|v| *v < 0.0) && values.iter().any(|v| *v > 0.0));
        let context = Context {
            paint: Paint {
                weight: Weight::Thick,
                ..Paint::default()
            },
            unit: 2.0,
            seed: 0,
        };
        assert!((context.width() - 14.0).abs() < 1e-9);
        assert!((context.text_size() - 64.0).abs() < 1e-9);
        assert_eq!(context.tint().a, 0x40);
    }
}
