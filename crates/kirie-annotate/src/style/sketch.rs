//! Sketch: every line drawn twice by an unsteady hand, closed shapes hatched like a marker
//! sketch, text in handwriting.

use kurbo::{BezPath, Point, Rect, Vec2};

use super::{Context, Rng, arrow_wings, ellipse_points, halo_for, smooth};
use crate::primitive::{FillRule, Font, Primitive};
use crate::shape::{Kind, Rgba};

/// The hatching angle, as in rough.js.
const HACHURE_ANGLE: f64 = -41.0;

pub(super) fn primitives(kind: &Kind, context: &Context) -> Vec<Primitive> {
    let mut rng = Rng::new(context.seed);
    let width = context.width() * 0.7;
    let color = context.color();
    let stroke = |path: BezPath| Primitive::Stroke { path, color, width };
    match kind {
        Kind::Line { from, to } => double_line(&mut rng, *from, *to, context.unit)
            .into_iter()
            .map(stroke)
            .collect(),
        Kind::Arrow { from, to } => {
            let mut paths = double_line(&mut rng, *from, *to, context.unit);
            let head = (context.width() * 4.0).max(16.0 * context.unit);
            if let Some((left, right)) = arrow_wings(*from, *to, head, 0.5) {
                paths.extend(double_line(&mut rng, *to, left, context.unit));
                paths.extend(double_line(&mut rng, *to, right, context.unit));
            }
            paths.into_iter().map(stroke).collect()
        }
        Kind::Rectangle { rect } => {
            let corners = corners(rect.abs());
            let mut drawn = hachure(&mut rng, &corners, context);
            for index in 0..4 {
                let (a, b) = (corners[index], corners[(index + 1) % 4]);
                let overshoot = (b - a).normalize() * 3.0 * context.unit;
                drawn.extend(
                    double_line(&mut rng, a - overshoot * 0.5, b + overshoot, context.unit)
                        .into_iter()
                        .map(stroke),
                );
            }
            drawn
        }
        Kind::Ellipse { rect } => {
            let rect = rect.abs();
            let outline = ellipse_points(rect, 48, 0.0, std::f64::consts::TAU);
            let mut drawn = hachure(&mut rng, &outline, context);
            for _ in 0..2 {
                drawn.push(stroke(rough_ellipse(&mut rng, rect, context.unit)));
            }
            drawn
        }
        Kind::Pen { points } => vec![stroke(smooth(points))],
        Kind::Highlighter { points } => vec![Primitive::Stroke {
            path: smooth(points),
            color: color.with_alpha(0x66),
            width: context.width() * 4.0,
        }],
        Kind::Text { at, text } => vec![Primitive::Text {
            at: *at,
            text: text.clone(),
            size: context.text_size() * 1.35,
            color,
            font: Font::Hand,
            halo: Some(halo_for(color)),
            centered: false,
        }],
        Kind::Counter { at, number } => {
            let radius = context.counter_radius();
            let rect = Rect::from_center_size(*at, (radius * 2.0, radius * 2.0));
            vec![
                Primitive::Fill {
                    path: kurbo::Shape::to_path(&kurbo::Circle::new(*at, radius), 0.1),
                    color: Rgba::rgb(0xff, 0xff, 0xff).with_alpha(0xe6),
                    rule: FillRule::NonZero,
                },
                stroke(rough_ellipse(&mut rng, rect, context.unit)),
                stroke(rough_ellipse(&mut rng, rect, context.unit)),
                Primitive::Text {
                    at: *at,
                    text: number.to_string(),
                    size: radius * 1.6,
                    color,
                    font: Font::Hand,
                    halo: None,
                    centered: true,
                },
            ]
        }
        Kind::Redact { rect } => {
            let corners = corners(rect.abs());
            let mut path = BezPath::new();
            for (index, corner) in corners.iter().enumerate() {
                let point = *corner + jitter(&mut rng, 1.5 * context.unit);
                if index == 0 {
                    path.move_to(point);
                } else {
                    path.line_to(point);
                }
            }
            path.close_path();
            vec![Primitive::Fill {
                path,
                color: Rgba::rgb(0x1e, 0x1e, 0x1e),
                rule: FillRule::NonZero,
            }]
        }
        Kind::Blur { .. } | Kind::Pixelate { .. } | Kind::Spotlight { .. } => Vec::new(),
    }
}

fn corners(rect: Rect) -> [Point; 4] {
    [
        Point::new(rect.x0, rect.y0),
        Point::new(rect.x1, rect.y0),
        Point::new(rect.x1, rect.y1),
        Point::new(rect.x0, rect.y1),
    ]
}

fn jitter(rng: &mut Rng, amount: f64) -> Vec2 {
    Vec2::new(rng.signed() * amount, rng.signed() * amount)
}

/// One wobbly stroke from `a` to `b`, bowed a little to one side.
fn rough_line(rng: &mut Rng, a: Point, b: Point, unit: f64) -> BezPath {
    let along = b - a;
    let length = along.hypot();
    let offset = (length * 0.02).clamp(0.6, 3.0) * unit;
    let normal = if length > 0.0 {
        Vec2::new(-along.y, along.x) / length
    } else {
        Vec2::ZERO
    };
    let bow = normal * rng.signed() * (length * 0.015).min(4.0 * unit);
    let mut path = BezPath::new();
    path.move_to(a + jitter(rng, offset * 0.5));
    path.curve_to(
        a + along * 0.5 + bow + jitter(rng, offset),
        a + along * 0.75 + bow + jitter(rng, offset),
        b + jitter(rng, offset * 0.5),
    );
    path
}

fn double_line(rng: &mut Rng, a: Point, b: Point, unit: f64) -> Vec<BezPath> {
    vec![rough_line(rng, a, b, unit), rough_line(rng, a, b, unit)]
}

/// An ellipse drawn in one go that overshoots where it started.
fn rough_ellipse(rng: &mut Rng, rect: Rect, unit: f64) -> BezPath {
    let start = rng.signed() * std::f64::consts::PI;
    let wobble = (rect.width().min(rect.height()) * 0.03).clamp(0.5, 3.0) * unit;
    let points: Vec<Point> = ellipse_points(rect, 14, start, std::f64::consts::TAU + 0.35)
        .into_iter()
        .map(|point| point + jitter(rng, wobble))
        .collect();
    smooth(&points)
}

/// Parallel hatching lines inside the closed `outline`, when the mark is filled.
fn hachure(rng: &mut Rng, outline: &[Point], context: &Context) -> Vec<Primitive> {
    if !context.paint.filled || outline.len() < 3 {
        return Vec::new();
    }
    let gap = (context.width() * 2.5).max(6.0 * context.unit);
    let color = context.color().with_alpha(0x99);
    let width = (context.width() * 0.4).max(context.unit);
    hachure_segments(outline, gap)
        .into_iter()
        .map(|(a, b)| Primitive::Stroke {
            path: rough_line(rng, a, b, context.unit * 0.5),
            color,
            width,
        })
        .collect()
}

/// The pieces of evenly spaced, slanted lines that lie inside a closed polygon.
fn hachure_segments(polygon: &[Point], gap: f64) -> Vec<(Point, Point)> {
    let (sin, cos) = HACHURE_ANGLE.to_radians().sin_cos();
    // Rotate so hatching lines are horizontal, scan, then rotate the pieces back.
    let rotate = |p: Point, sin: f64| Point::new(p.x * cos - p.y * sin, p.x * sin + p.y * cos);
    let turned: Vec<Point> = polygon.iter().map(|p| rotate(*p, -sin)).collect();
    let (top, bottom) = turned
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(top, bottom), p| {
            (top.min(p.y), bottom.max(p.y))
        });
    let mut segments = Vec::new();
    let mut y = top + gap / 2.0;
    while y < bottom {
        let mut crossings: Vec<f64> = turned
            .iter()
            .zip(turned.iter().cycle().skip(1))
            .filter_map(|(a, b)| {
                let (low, high) = if a.y < b.y { (a, b) } else { (b, a) };
                (y >= low.y && y < high.y)
                    .then(|| low.x + (y - low.y) / (high.y - low.y) * (high.x - low.x))
            })
            .collect();
        crossings.sort_by(f64::total_cmp);
        for pair in crossings.as_chunks::<2>().0 {
            segments.push((
                rotate(Point::new(pair[0], y), sin),
                rotate(Point::new(pair[1], y), sin),
            ));
        }
        y += gap;
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::Paint;

    #[test]
    fn hatching_stays_inside_a_square() {
        let square = corners(Rect::new(0.0, 0.0, 100.0, 100.0));
        let segments = hachure_segments(&square, 10.0);
        assert!(segments.len() > 5);
        for (a, b) in segments {
            for point in [a, b] {
                assert!((-0.01..=100.01).contains(&point.x), "{point:?}");
                assert!((-0.01..=100.01).contains(&point.y), "{point:?}");
            }
        }
    }

    #[test]
    fn only_filled_shapes_are_hatched() {
        let square = corners(Rect::new(0.0, 0.0, 100.0, 100.0));
        let mut rng = Rng::new(3);
        let plain = Context {
            paint: Paint::default(),
            unit: 1.0,
            seed: 3,
        };
        assert!(hachure(&mut rng, &square, &plain).is_empty());
        let filled = Context {
            paint: Paint {
                filled: true,
                ..Paint::default()
            },
            ..plain
        };
        assert!(!hachure(&mut rng, &square, &filled).is_empty());
        assert!(hachure(&mut rng, &square[..2], &filled).is_empty());
    }

    #[test]
    fn rough_lines_stay_near_their_ends() {
        let mut rng = Rng::new(9);
        for _ in 0..20 {
            let path = rough_line(&mut rng, Point::ORIGIN, Point::new(100.0, 0.0), 1.0);
            let bounds = kurbo::Shape::bounding_box(&path);
            assert!(bounds.y0 > -8.0 && bounds.y1 < 8.0, "{bounds:?}");
        }
        let dot = rough_line(&mut rng, Point::ORIGIN, Point::ORIGIN, 1.0);
        assert!(kurbo::Shape::bounding_box(&dot).width() < 2.0);
    }
}
