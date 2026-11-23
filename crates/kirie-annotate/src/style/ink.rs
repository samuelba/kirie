//! Ink: brush strokes that swell and taper like sumi on paper; circles become an open ensō.

use kurbo::{BezPath, Point, Rect, RoundedRect, Shape as _, Vec2};

use super::{Context, Rng, arrow_wings, ellipse_points, text_on, thin_out};
use crate::primitive::{Align, FillRule, Font, Primitive};
use crate::shape::{Kind, Rgba};

/// What redactions are filled with.
pub(super) const REDACT: Rgba = Rgba::rgb(0x1f, 0x1d, 0x1a);

/// How the brush presses along a stroke: light at the start, full in the middle, lifting off.
#[derive(Debug, Clone, Copy)]
pub(super) struct Taper {
    pub start: f64,
    pub end: f64,
}

const STROKE: Taper = Taper {
    start: 0.18,
    end: 0.35,
};

/// The sides of a box: pressed almost all the way, lifting only at the very ends.
const EDGE: Taper = Taper {
    start: 0.06,
    end: 0.12,
};

pub(super) fn primitives(kind: &Kind, context: &Context) -> Vec<Primitive> {
    let mut rng = Rng::new(context.seed);
    let width = context.width() * 1.6;
    let color = context.color();
    let ink = |path: BezPath| Primitive::Fill {
        path,
        color,
        rule: FillRule::NonZero,
    };
    match kind {
        Kind::Line { from, to } => vec![ink(brush(&[*from, *to], width, STROKE))],
        Kind::Arrow { from, to } => {
            let mut drawn = vec![ink(brush(&[*from, *to], width, STROKE))];
            let head = (width * 3.0).max(14.0 * context.unit);
            if let Some((left, right)) = arrow_wings(*from, *to, head, 0.5) {
                let flick = Taper {
                    start: 0.05,
                    end: 0.7,
                };
                drawn.push(ink(brush(&[*to, left], width * 0.9, flick)));
                drawn.push(ink(brush(&[*to, right], width * 0.9, flick)));
            }
            drawn
        }
        Kind::Rectangle { rect } => {
            let rect = rect.abs();
            let mut drawn: Vec<Primitive> =
                tint(context, RoundedRect::from_rect(rect, 0.0).to_path(0.1))
                    .into_iter()
                    .collect();
            let corners = [
                Point::new(rect.x0, rect.y0),
                Point::new(rect.x1, rect.y0),
                Point::new(rect.x1, rect.y1),
                Point::new(rect.x0, rect.y1),
            ];
            for index in 0..4 {
                let (a, b) = (corners[index], corners[(index + 1) % 4]);
                let along = (b - a).normalize();
                let overshoot = along * width * (0.6 + 0.4 * rng.signed().abs());
                drawn.push(ink(brush(
                    &[a - overshoot * 0.3, b + overshoot],
                    width * 0.85,
                    EDGE,
                )));
            }
            drawn
        }
        Kind::Ellipse { rect } => {
            let rect = rect.abs();
            let mut drawn: Vec<Primitive> =
                tint(context, kurbo::Ellipse::from_rect(rect).to_path(0.1))
                    .into_iter()
                    .collect();
            drawn.push(ink(enso(&mut rng, rect, width)));
            drawn
        }
        Kind::Pen { points } => vec![ink(brush(points, width, STROKE))],
        Kind::Highlighter { points } => vec![super::marker(points, context)],
        Kind::Counter { at, number } => {
            let radius = context.counter_radius();
            vec![
                Primitive::Fill {
                    path: kurbo::Circle::new(*at, radius).to_path(0.1),
                    color,
                    rule: FillRule::NonZero,
                },
                Primitive::Text {
                    at: *at,
                    text: number.to_string(),
                    size: radius * 1.6,
                    color: text_on(color),
                    font: Font::Hand,
                    align: Align::Left,
                    halo: None,
                    centered: true,
                },
            ]
        }
        Kind::Redact { rect } => vec![Primitive::Fill {
            path: RoundedRect::from_rect(rect.abs(), 1.0 * context.unit).to_path(0.1),
            color: REDACT,
            rule: FillRule::NonZero,
        }],
        Kind::Text { .. }
        | Kind::Bubble { .. }
        | Kind::Blur { .. }
        | Kind::Pixelate { .. }
        | Kind::Spotlight { .. }
        | Kind::Image { .. } => Vec::new(),
    }
}

fn tint(context: &Context, path: BezPath) -> Option<Primitive> {
    context.paint.filled.then(|| Primitive::Fill {
        path,
        color: context.tint(),
        rule: FillRule::NonZero,
    })
}

/// An open circle in one sweep of the brush, leaving a gap where it lifts.
fn enso(rng: &mut Rng, rect: Rect, width: f64) -> BezPath {
    let start = -2.2 + rng.signed() * 0.3;
    let sweep = std::f64::consts::TAU * 0.93;
    brush(
        &ellipse_points(rect, 64, start, sweep),
        width,
        Taper {
            start: 0.08,
            end: 0.45,
        },
    )
}

/// The outline of a brush stroke along `points`, `width` wide where pressed fully.
pub(super) fn brush(points: &[Point], width: f64, taper: Taper) -> BezPath {
    let samples = resample(&thin_out(points, 0.5), (width * 0.3).max(0.5));
    let mut path = BezPath::new();
    match samples.as_slice() {
        [] => return path,
        [only] => return kurbo::Circle::new(*only, width * 0.3).to_path(0.1),
        _ => {}
    }
    let count = samples.len();
    let mut left = Vec::with_capacity(count);
    let mut right = Vec::with_capacity(count);
    for (index, point) in samples.iter().enumerate() {
        let before = samples[index.saturating_sub(1)];
        let after = samples[(index + 1).min(count - 1)];
        let tangent = after - before;
        let normal = if tangent.hypot() > 0.0 {
            Vec2::new(-tangent.y, tangent.x).normalize()
        } else {
            Vec2::ZERO
        };
        #[allow(clippy::cast_precision_loss)]
        let t = index as f64 / (count - 1) as f64;
        let radius = width / 2.0 * pressure(t, taper);
        left.push(*point + normal * radius);
        right.push(*point - normal * radius);
    }
    path.move_to(left[0]);
    for point in &left[1..] {
        path.line_to(*point);
    }
    for point in right.iter().rev() {
        path.line_to(*point);
    }
    path.close_path();
    path
}

/// The brush pressure at `t` along the stroke, from 0 to 1.
fn pressure(t: f64, taper: Taper) -> f64 {
    let ease = |x: f64| x * x * (3.0 - 2.0 * x);
    let rise = if taper.start > 0.0 {
        ease((t / taper.start).min(1.0))
    } else {
        1.0
    };
    let fall = if taper.end > 0.0 {
        ease(((1.0 - t) / taper.end).min(1.0))
    } else {
        1.0
    };
    (0.2 + 0.8 * rise).min(0.12 + 0.88 * fall)
}

/// Points every `step` along the polyline.
fn resample(points: &[Point], step: f64) -> Vec<Point> {
    let Some(first) = points.first() else {
        return Vec::new();
    };
    let mut out = vec![*first];
    let mut carried = 0.0;
    for pair in points.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let length = a.distance(b);
        let mut at = step - carried;
        while at <= length {
            out.push(a.lerp(b, at / length));
            at += step;
        }
        carried = length - (at - step);
    }
    if let Some(last) = points.last()
        && out.last() != Some(last)
    {
        out.push(*last);
    }
    out
}

/// A speech bubble: its outline filled flat with ink.
pub(super) fn bubble(outline: BezPath, context: &Context) -> Vec<Primitive> {
    vec![Primitive::Fill {
        path: outline,
        color: context.color(),
        rule: FillRule::NonZero,
    }]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pressure_rises_and_lifts() {
        assert!(pressure(0.0, STROKE) < 0.3);
        assert!((pressure(0.5, STROKE) - 1.0).abs() < 1e-9);
        assert!(pressure(1.0, STROKE) < 0.2);
        let flat = Taper {
            start: 0.0,
            end: 0.0,
        };
        assert!((pressure(0.0, flat) - 1.0).abs() < 1e-9);
    }

    #[test]
    fn resampling_spaces_points_evenly() {
        let points = resample(&[Point::ORIGIN, Point::new(10.0, 0.0)], 2.0);
        assert_eq!(points.len(), 6);
        assert!((points[1].x - 2.0).abs() < 1e-9);
        let bent = resample(
            &[Point::ORIGIN, Point::new(3.0, 0.0), Point::new(3.0, 3.0)],
            2.0,
        );
        assert_eq!(bent.last(), Some(&Point::new(3.0, 3.0)));
        assert!(resample(&[], 1.0).is_empty());
    }

    #[test]
    fn brushes_are_closed_outlines_as_wide_as_asked() {
        let path = brush(&[Point::ORIGIN, Point::new(100.0, 0.0)], 10.0, STROKE);
        let bounds = path.bounding_box();
        assert!(bounds.height() <= 10.0 + 1e-6 && bounds.height() > 8.0);
        assert!(path.elements().last() == Some(&kurbo::PathEl::ClosePath));
        assert!(brush(&[], 4.0, STROKE).elements().is_empty());
        assert!(brush(&[Point::ORIGIN], 4.0, STROKE).bounding_box().width() > 0.0);
    }

    #[test]
    fn enso_leaves_a_gap() {
        let mut rng = Rng::new(1);
        let ring = enso(&mut rng, Rect::new(0.0, 0.0, 100.0, 100.0), 8.0);
        assert!(ring.bounding_box().width() > 90.0);
    }
}
