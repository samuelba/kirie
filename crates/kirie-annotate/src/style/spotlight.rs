//! The shade spotlights put over the rest of the image. A fill rule cannot cut out holes that
//! overlap, so the shade is built from pieces that never overlap: the image is cut into bands
//! within which no two hole edges cross, and each band is shaded between the holes in it.

use std::f64::consts::PI;

use kurbo::{Arc, BezPath, Point, Rect, Vec2};

use crate::primitive::{FillRule, Primitive};
use crate::shape::Rgba;

/// An area a spotlight lights: a box with rounded corners, or an ellipse.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hole {
    pub rect: Rect,
    pub oval: bool,
}

impl Hole {
    fn corner(self, radius: f64) -> f64 {
        radius
            .min(self.rect.width() / 2.0)
            .min(self.rect.height() / 2.0)
            .max(0.0)
    }

    /// Heights where a side of the hole changes from one curve to the next.
    fn breaks(self, radius: f64) -> Vec<f64> {
        let Rect { y0, y1, .. } = self.rect;
        let corner = self.corner(radius);
        if self.oval || corner == 0.0 {
            vec![y0, y1]
        } else {
            vec![y0, y0 + corner, y1 - corner, y1]
        }
    }

    /// The left and right sides around height `y`.
    fn sides(self, y: f64, radius: f64) -> (Side, Side) {
        let Rect { x0, y0, x1, y1 } = self.rect;
        if self.oval {
            let center = self.rect.center();
            let radii = Vec2::new(self.rect.width() / 2.0, self.rect.height() / 2.0);
            return (
                Side::Arc {
                    center,
                    radii,
                    right: false,
                },
                Side::Arc {
                    center,
                    radii,
                    right: true,
                },
            );
        }
        let corner = self.corner(radius);
        let row = if y < y0 + corner {
            y0 + corner
        } else if y > y1 - corner {
            y1 - corner
        } else {
            return (Side::Line(x0), Side::Line(x1));
        };
        let radii = Vec2::new(corner, corner);
        (
            Side::Arc {
                center: Point::new(x0 + corner, row),
                radii,
                right: false,
            },
            Side::Arc {
                center: Point::new(x1 - corner, row),
                radii,
                right: true,
            },
        )
    }
}

/// One side of a hole within a band: a straight edge, or the left or right half of an ellipse.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Side {
    Line(f64),
    Arc {
        center: Point,
        radii: Vec2,
        right: bool,
    },
}

impl Side {
    fn x(self, y: f64) -> f64 {
        match self {
            Side::Line(x) => x,
            Side::Arc {
                center,
                radii,
                right,
            } => {
                let t = ((y - center.y) / radii.y).clamp(-1.0, 1.0);
                let reach = radii.x * (1.0 - t * t).max(0.0).sqrt();
                if right {
                    center.x + reach
                } else {
                    center.x - reach
                }
            }
        }
    }

    /// Continues `path` along this side from height `from` to height `to`.
    fn trace(self, path: &mut BezPath, from: f64, to: f64) {
        match self {
            Side::Line(x) => path.line_to(Point::new(x, to)),
            Side::Arc {
                center,
                radii,
                right,
            } => {
                let angle = |y: f64| {
                    let turn = ((y - center.y) / radii.y).clamp(-1.0, 1.0).asin();
                    if right { turn } else { PI - turn }
                };
                let start = angle(from);
                let arc = Arc::new(center, radii, start, angle(to) - start, 0.0);
                path.extend(arc.append_iter(0.1));
            }
        }
    }
}

/// The part of `visible` outside every hole, holes' corners rounded by `radius`, shaded `dim`
/// dark.
pub fn shade(visible: Rect, holes: &[Hole], radius: f64, dim: f64) -> Primitive {
    let holes: Vec<Hole> = holes
        .iter()
        .map(|hole| Hole {
            rect: hole.rect.abs(),
            ..*hole
        })
        .filter(|hole| hole.rect.intersect(visible).area() > 0.0)
        .collect();
    let mut cuts: Vec<f64> = holes
        .iter()
        .flat_map(|hole| hole.breaks(radius))
        .chain([visible.y0, visible.y1])
        .filter(|y| (visible.y0..=visible.y1).contains(y))
        .collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut heights = cuts.clone();
    for band in cuts.windows(2) {
        let sides = sides_in(&holes, band[0], band[1], radius, visible);
        heights.extend(crossings(&sides, band[0], band[1]));
    }
    heights.sort_by(f64::total_cmp);
    heights.dedup();
    let mut path = BezPath::new();
    for band in heights.windows(2) {
        if band[1] - band[0] > 1e-9 {
            shade_band(&mut path, &holes, band[0], band[1], radius, visible);
        }
    }
    Primitive::Fill {
        path,
        color: Rgba::rgb(0, 0, 0).with_alpha(opacity(dim)),
        rule: FillRule::NonZero,
    }
}

/// Every side within the band from `top` to `bottom`, the edges of `visible` among them.
fn sides_in(holes: &[Hole], top: f64, bottom: f64, radius: f64, visible: Rect) -> Vec<Side> {
    let middle = f64::midpoint(top, bottom);
    holes
        .iter()
        .filter(|hole| hole.rect.y0 < middle && middle < hole.rect.y1)
        .flat_map(|hole| {
            let (left, right) = hole.sides(middle, radius);
            [left, right]
        })
        .chain([Side::Line(visible.x0), Side::Line(visible.x1)])
        .collect()
}

/// Heights between `top` and `bottom` where two of `sides` cross.
fn crossings(sides: &[Side], top: f64, bottom: f64) -> Vec<f64> {
    const SAMPLES: u32 = 24;
    let at = |step: u32| top + (bottom - top) * f64::from(step) / f64::from(SAMPLES);
    let mut found = Vec::new();
    for (index, a) in sides.iter().enumerate() {
        for b in &sides[index + 1..] {
            if matches!((a, b), (Side::Line(_), Side::Line(_))) {
                continue;
            }
            let apart = |y: f64| a.x(y) - b.x(y);
            for step in 0..SAMPLES {
                let (mut low, mut high) = (at(step), at(step + 1));
                let starts = apart(low);
                if starts * apart(high) >= 0.0 {
                    continue;
                }
                for _ in 0..48 {
                    let middle = f64::midpoint(low, high);
                    if apart(middle) * starts > 0.0 {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                found.push(f64::midpoint(low, high));
            }
        }
    }
    found
}

/// Shades the band from `top` to `bottom` everywhere outside the holes; no two sides cross
/// within it, so which side bounds each piece holds from top to bottom.
fn shade_band(
    path: &mut BezPath,
    holes: &[Hole],
    top: f64,
    bottom: f64,
    radius: f64,
    visible: Rect,
) {
    let middle = f64::midpoint(top, bottom);
    let mut spans: Vec<(Side, Side)> = holes
        .iter()
        .filter(|hole| hole.rect.y0 < middle && middle < hole.rect.y1)
        .map(|hole| hole.sides(middle, radius))
        .collect();
    spans.sort_by(|a, b| a.0.x(middle).total_cmp(&b.0.x(middle)));
    let end = Side::Line(visible.x1);
    let mut from = Side::Line(visible.x0);
    for (left, right) in spans {
        if left.x(middle) > from.x(middle) && from.x(middle) < visible.x1 {
            let to = if left.x(middle) < visible.x1 {
                left
            } else {
                end
            };
            piece(path, from, to, top, bottom);
        }
        if right.x(middle) > from.x(middle) {
            from = right;
        }
    }
    if from.x(middle) < visible.x1 {
        piece(path, from, end, top, bottom);
    }
}

/// The area between `left` and `right` from `top` to `bottom`, always wound the same way.
fn piece(path: &mut BezPath, left: Side, right: Side, top: f64, bottom: f64) {
    path.move_to(Point::new(left.x(top), top));
    path.line_to(Point::new(right.x(top), top));
    right.trace(path, top, bottom);
    path.line_to(Point::new(left.x(bottom), bottom));
    left.trace(path, bottom, top);
    path.close_path();
}

#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
fn opacity(share: f64) -> u8 {
    (share.clamp(0.0, 1.0) * 255.0).round() as u8
}

#[cfg(test)]
mod tests {
    use kurbo::Shape as _;

    use super::*;

    const VISIBLE: Rect = Rect::new(0.0, 0.0, 100.0, 100.0);

    fn shaded(holes: &[Hole], radius: f64) -> BezPath {
        let Primitive::Fill { path, .. } = shade(VISIBLE, holes, radius, 0.5) else {
            panic!()
        };
        path
    }

    /// How far `point` is inside `hole` (negative) or outside it (positive), roughly.
    fn depth(hole: Hole, point: Point, radius: f64) -> f64 {
        let center = hole.rect.center();
        let half = Vec2::new(hole.rect.width() / 2.0, hole.rect.height() / 2.0);
        if hole.oval {
            let reach = ((point.x - center.x) / half.x).hypot((point.y - center.y) / half.y);
            return (reach - 1.0) * half.x.min(half.y);
        }
        let corner = hole.corner(radius);
        let q = Vec2::new(
            (point.x - center.x).abs() - (half.x - corner),
            (point.y - center.y).abs() - (half.y - corner),
        );
        Vec2::new(q.x.max(0.0), q.y.max(0.0)).hypot() + q.x.max(q.y).min(0.0) - corner
    }

    /// Every point away from an edge is shaded once outside the holes, and not at all in them.
    fn assert_shades_once(holes: &[Hole], radius: f64) {
        let path = shaded(holes, radius);
        for row in 0..50 {
            for column in 0..50 {
                let point = Point::new(f64::from(column) * 2.0 + 0.7, f64::from(row) * 2.0 + 0.3);
                let depths: Vec<f64> = holes
                    .iter()
                    .map(|hole| depth(*hole, point, radius))
                    .collect();
                if depths.iter().any(|depth| depth.abs() < 0.5) {
                    continue;
                }
                let inside = depths.iter().any(|depth| *depth < 0.0);
                let winding = path.winding(point);
                assert_eq!(
                    winding.abs(),
                    i32::from(!inside),
                    "at {point:?} among {holes:?}"
                );
            }
        }
    }

    #[test]
    fn overlapping_boxes_are_shaded_around_once() {
        let boxes = |rect| Hole { rect, oval: false };
        assert_shades_once(
            &[
                boxes(Rect::new(10.0, 20.0, 60.0, 80.0)),
                boxes(Rect::new(40.0, 10.0, 90.0, 50.0)),
            ],
            6.0,
        );
    }

    #[test]
    fn overlapping_ovals_are_shaded_around_once() {
        let oval = |rect| Hole { rect, oval: true };
        assert_shades_once(
            &[
                oval(Rect::new(10.0, 20.0, 60.0, 80.0)),
                oval(Rect::new(40.0, 10.0, 95.0, 60.0)),
                oval(Rect::new(20.0, 55.0, 50.0, 95.0)),
            ],
            6.0,
        );
    }

    #[test]
    fn ovals_and_boxes_mix() {
        assert_shades_once(
            &[
                Hole {
                    rect: Rect::new(10.0, 10.0, 70.0, 50.0),
                    oval: true,
                },
                Hole {
                    rect: Rect::new(50.0, 30.0, 90.0, 90.0),
                    oval: false,
                },
            ],
            8.0,
        );
    }

    #[test]
    fn an_oval_leaves_its_box_corners_shaded() {
        let path = shaded(
            &[Hole {
                rect: Rect::new(20.0, 20.0, 80.0, 60.0),
                oval: true,
            }],
            6.0,
        );
        assert_eq!(path.winding(Point::new(50.0, 40.0)), 0);
        assert_eq!(path.winding(Point::new(22.0, 22.0)).abs(), 1);
        assert_eq!(path.winding(Point::new(78.0, 58.0)).abs(), 1);
    }

    #[test]
    fn a_spotlight_past_the_edge_shades_only_what_is_shown() {
        let path = shaded(
            &[Hole {
                rect: Rect::new(-20.0, 30.0, 50.0, 70.0),
                oval: false,
            }],
            6.0,
        );
        assert_eq!(
            path.winding(Point::new(0.5, 30.5)),
            0,
            "no rounding at the edge"
        );
        assert_eq!(path.winding(Point::new(49.5, 30.5)).abs(), 1);
        assert_eq!(path.winding(Point::new(-10.0, 10.0)), 0, "nothing outside");
        assert!(path.bounding_box().x0 >= 0.0);
        assert_shades_once(
            &[Hole {
                rect: Rect::new(-20.0, -10.0, 40.0, 50.0),
                oval: true,
            }],
            6.0,
        );
    }

    #[test]
    fn without_holes_everything_is_shaded() {
        let path = shaded(&[], 6.0);
        assert_eq!(path.winding(Point::new(50.0, 50.0)).abs(), 1);
        assert!((path.area().abs() - VISIBLE.area()).abs() < 1e-6);
    }

    #[test]
    fn the_shade_is_as_dark_as_asked() {
        let Primitive::Fill { color, .. } = shade(VISIBLE, &[], 6.0, 0.55) else {
            panic!()
        };
        assert_eq!(color.a, 0x8c);
    }
}
