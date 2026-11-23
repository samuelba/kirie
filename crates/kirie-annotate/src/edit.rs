//! Changing a mark after it is drawn: grips on its corners, edges and ends resize it, and a grip
//! above boxes and ellipses turns them.

use kurbo::{Affine, Point, Rect, Vec2};

use crate::shape::{Kind, Shape};

/// A handle on a picked mark.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Grip {
    /// A corner of the box, clockwise from the top left.
    Corner(u8),
    /// The middle of an edge: top, right, bottom, left.
    Edge(u8),
    Start,
    End,
    Rotate,
}

/// How far above a box its rotate grip sits, in screen pixels.
pub const ROTATE_OFFSET: f64 = 28.0;

/// The grips of `shape` and where they are. `pixel` is one screen pixel in mark coordinates,
/// so the rotate grip keeps its distance at any zoom.
pub fn grips(shape: &Shape, pixel: f64) -> Vec<(Grip, Point)> {
    let turn = shape.turn();
    match &shape.kind {
        Kind::Arrow { from, to } | Kind::Line { from, to } => {
            vec![(Grip::Start, *from), (Grip::End, *to)]
        }
        Kind::Rectangle { rect }
        | Kind::Ellipse { rect }
        | Kind::Redact { rect }
        | Kind::Blur { rect }
        | Kind::Pixelate { rect }
        | Kind::Spotlight { rect }
        | Kind::Image { rect, .. } => {
            let rect = rect.abs();
            let mut grips: Vec<(Grip, Point)> = corners(rect)
                .into_iter()
                .enumerate()
                .map(|(index, point)| (Grip::Corner(index_u8(index)), turn * point))
                .chain(
                    edges(rect)
                        .into_iter()
                        .enumerate()
                        .map(|(index, point)| (Grip::Edge(index_u8(index)), turn * point)),
                )
                .collect();
            if turns(&shape.kind) {
                let above = Point::new(rect.center().x, rect.y0 - ROTATE_OFFSET * pixel);
                grips.push((Grip::Rotate, turn * above));
            }
            grips
        }
        Kind::Pen { .. } | Kind::Highlighter { .. } => corners(shape.kind.bounds())
            .into_iter()
            .enumerate()
            .map(|(index, point)| (Grip::Corner(index_u8(index)), point))
            .collect(),
        Kind::Bubble { center, tail, .. } => vec![(Grip::Start, *center), (Grip::End, *tail)],
        Kind::Text { .. } | Kind::Counter { .. } => Vec::new(),
    }
}

/// The grip within `reach` of `point`, if any.
pub fn grip_at(shape: &Shape, point: Point, reach: f64, pixel: f64) -> Option<Grip> {
    grips(shape, pixel)
        .into_iter()
        .find(|(_, at)| at.distance(point) <= reach)
        .map(|(grip, _)| grip)
}

/// Which way a corner or edge grip of `shape` pulls, as an angle in mark coordinates (0 is
/// right, y grows down); `None` for grips that do not resize along an axis.
pub fn pull_angle(shape: &Shape, grip: Grip) -> Option<f64> {
    use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
    let base = match grip {
        Grip::Corner(index) => -3.0 * FRAC_PI_4 + f64::from(index) * FRAC_PI_2,
        Grip::Edge(index) => -FRAC_PI_2 + f64::from(index) * FRAC_PI_2,
        Grip::Start | Grip::End | Grip::Rotate => return None,
    };
    let turn = match shape.kind {
        Kind::Pen { .. } | Kind::Highlighter { .. } => 0.0,
        _ => shape.rotation,
    };
    Some(base + turn)
}

/// Whether marks of this kind can be turned; pixel effects stay square to the screen.
pub const fn turns(kind: &Kind) -> bool {
    matches!(
        kind,
        Kind::Rectangle { .. } | Kind::Ellipse { .. } | Kind::Redact { .. }
    )
}

/// `origin` with `grip` dragged to `to`. `constrain` (Shift) keeps squares, 45° lines and 15°
/// turns.
pub fn drag(origin: &Shape, grip: Grip, to: Point, constrain: bool) -> Shape {
    match (&origin.kind, grip) {
        (_, Grip::Rotate) => {
            let center = origin.kind.bounds().center();
            let mut angle = (to - center).atan2() + std::f64::consts::FRAC_PI_2;
            if constrain {
                let step = 15f64.to_radians();
                angle = (angle / step).round() * step;
            }
            Shape {
                rotation: angle,
                ..origin.clone()
            }
        }
        (Kind::Arrow { from, to: end } | Kind::Line { from, to: end }, Grip::Start | Grip::End) => {
            let fixed = if grip == Grip::Start { *end } else { *from };
            let moved = if constrain { snap(fixed, to) } else { to };
            let (from, end) = if grip == Grip::Start {
                (moved, fixed)
            } else {
                (fixed, moved)
            };
            let kind = match origin.kind {
                Kind::Arrow { .. } => Kind::Arrow { from, to: end },
                _ => Kind::Line { from, to: end },
            };
            Shape {
                kind,
                ..origin.clone()
            }
        }
        (Kind::Pen { points } | Kind::Highlighter { points }, Grip::Corner(corner)) => {
            let bounds = origin.kind.bounds();
            let resized = resize(bounds, Grip::Corner(corner), to, constrain);
            let map = fit(bounds, resized);
            let points: Vec<Point> = points.iter().map(|point| map * *point).collect();
            let kind = match origin.kind {
                Kind::Pen { .. } => Kind::Pen { points },
                _ => Kind::Highlighter { points },
            };
            Shape {
                kind,
                ..origin.clone()
            }
        }
        (Kind::Bubble { center, tail, text }, Grip::Start | Grip::End) => {
            let (center, tail) = if grip == Grip::Start {
                (to, *tail)
            } else {
                (*center, to)
            };
            Shape {
                kind: Kind::Bubble {
                    center,
                    tail,
                    text: text.clone(),
                },
                ..origin.clone()
            }
        }
        (kind, Grip::Corner(_) | Grip::Edge(_)) => {
            let Some(rect) = rect_of(kind) else {
                return origin.clone();
            };
            // Resize in the mark's own frame, then put it back so the far side stays put.
            let turn = origin.turn();
            // A picture keeps its proportions from its corners, unless Shift frees them.
            let keep = matches!(kind, Kind::Image { .. }) && matches!(grip, Grip::Corner(_));
            let local = if keep {
                proportional(rect, grip, turn.inverse() * to, constrain)
            } else {
                resize(rect, grip, turn.inverse() * to, constrain)
            };
            let center = turn * local.center();
            let placed = local + (center - local.center());
            Shape {
                kind: with_rect(kind, placed),
                ..origin.clone()
            }
        }
        _ => origin.clone(),
    }
}

fn rect_of(kind: &Kind) -> Option<Rect> {
    match kind {
        Kind::Rectangle { rect }
        | Kind::Ellipse { rect }
        | Kind::Redact { rect }
        | Kind::Blur { rect }
        | Kind::Pixelate { rect }
        | Kind::Spotlight { rect }
        | Kind::Image { rect, .. } => Some(rect.abs()),
        _ => None,
    }
}

fn with_rect(kind: &Kind, rect: Rect) -> Kind {
    match kind {
        Kind::Rectangle { .. } => Kind::Rectangle { rect },
        Kind::Ellipse { .. } => Kind::Ellipse { rect },
        Kind::Redact { .. } => Kind::Redact { rect },
        Kind::Blur { .. } => Kind::Blur { rect },
        Kind::Pixelate { .. } => Kind::Pixelate { rect },
        Kind::Spotlight { .. } => Kind::Spotlight { rect },
        Kind::Image { pixels, .. } => Kind::Image {
            rect,
            pixels: pixels.clone(),
        },
        other => other.clone(),
    }
}

/// `rect` with the corner or edge `grip` moved to `to`; the opposite side stays.
fn resize(rect: Rect, grip: Grip, to: Point, constrain: bool) -> Rect {
    let (mut x0, mut y0, mut x1, mut y1) = (rect.x0, rect.y0, rect.x1, rect.y1);
    match grip {
        Grip::Corner(0) => (x0, y0) = (to.x, to.y),
        Grip::Corner(1) => (x1, y0) = (to.x, to.y),
        Grip::Corner(2) => (x1, y1) = (to.x, to.y),
        Grip::Corner(3) => (x0, y1) = (to.x, to.y),
        Grip::Edge(0) => y0 = to.y,
        Grip::Edge(1) => x1 = to.x,
        Grip::Edge(2) => y1 = to.y,
        Grip::Edge(3) => x0 = to.x,
        _ => {}
    }
    if constrain && matches!(grip, Grip::Corner(_)) {
        let side = (x1 - x0).abs().max((y1 - y0).abs());
        match grip {
            Grip::Corner(0) => (x0, y0) = (x1 - side, y1 - side),
            Grip::Corner(1) => (x1, y0) = (x0 + side, y1 - side),
            Grip::Corner(2) => (x1, y1) = (x0 + side, y0 + side),
            _ => (x0, y1) = (x1 - side, y0 + side),
        }
    }
    Rect::new(x0, y0, x1, y1).abs()
}

/// `rect` with the corner `grip` moved toward `to`, keeping its proportions unless `free`.
fn proportional(rect: Rect, grip: Grip, to: Point, free: bool) -> Rect {
    let stretched = resize(rect, grip, to, false);
    if free || rect.width() <= 0.0 || rect.height() <= 0.0 {
        return stretched;
    }
    let scale = (stretched.width() / rect.width()).max(stretched.height() / rect.height());
    let size = kurbo::Size::new(rect.width() * scale, rect.height() * scale);
    let fixed = match grip {
        Grip::Corner(0) => Point::new(rect.x1, rect.y1),
        Grip::Corner(1) => Point::new(rect.x0, rect.y1),
        Grip::Corner(2) => Point::new(rect.x0, rect.y0),
        _ => Point::new(rect.x1, rect.y0),
    };
    let toward = Vec2::new(
        (to.x - fixed.x).signum() * size.width,
        (to.y - fixed.y).signum() * size.height,
    );
    Rect::from_points(fixed, fixed + toward)
}

/// The map that stretches `from` onto `to`.
fn fit(from: Rect, to: Rect) -> Affine {
    let sx = if from.width() > 0.0 {
        to.width() / from.width()
    } else {
        1.0
    };
    let sy = if from.height() > 0.0 {
        to.height() / from.height()
    } else {
        1.0
    };
    Affine::translate(-from.origin().to_vec2())
        .then_scale_non_uniform(sx, sy)
        .then_translate(to.origin().to_vec2())
}

fn snap(fixed: Point, moved: Point) -> Point {
    let delta = moved - fixed;
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (delta.atan2() / step).round() * step;
    fixed + Vec2::from_angle(angle) * delta.hypot()
}

fn corners(rect: Rect) -> [Point; 4] {
    [
        Point::new(rect.x0, rect.y0),
        Point::new(rect.x1, rect.y0),
        Point::new(rect.x1, rect.y1),
        Point::new(rect.x0, rect.y1),
    ]
}

fn edges(rect: Rect) -> [Point; 4] {
    let center = rect.center();
    [
        Point::new(center.x, rect.y0),
        Point::new(rect.x1, center.y),
        Point::new(center.x, rect.y1),
        Point::new(rect.x0, center.y),
    ]
}

fn index_u8(index: usize) -> u8 {
    u8::try_from(index).unwrap_or(u8::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::{Paint, ShapeId};

    fn shape(kind: Kind) -> Shape {
        Shape {
            id: ShapeId(1),
            kind,
            paint: Paint::default(),
            style: None,
            rotation: 0.0,
        }
    }

    fn rectangle() -> Shape {
        shape(Kind::Rectangle {
            rect: Rect::new(0.0, 0.0, 100.0, 50.0),
        })
    }

    #[test]
    fn boxes_have_corners_edges_and_a_rotate_grip() {
        let grips = grips(&rectangle(), 1.0);
        assert_eq!(grips.len(), 9);
        assert!(grips.contains(&(Grip::Corner(2), Point::new(100.0, 50.0))));
        assert!(grips.contains(&(Grip::Rotate, Point::new(50.0, -ROTATE_OFFSET))));
        let blur = shape(Kind::Blur {
            rect: Rect::new(0.0, 0.0, 10.0, 10.0),
        });
        assert_eq!(super::grips(&blur, 1.0).len(), 8, "effects do not turn");
        let text = shape(Kind::Text {
            at: Point::ORIGIN,
            text: "a".into(),
        });
        assert!(super::grips(&text, 1.0).is_empty());
    }

    #[test]
    fn bubbles_move_their_box_and_tail_apart() {
        let (center, tip) = (Point::new(50.0, 20.0), Point::new(10.0, 80.0));
        let bubble = shape(Kind::Bubble {
            center,
            tail: tip,
            text: "a".into(),
        });
        assert_eq!(
            grips(&bubble, 1.0),
            vec![(Grip::Start, center), (Grip::End, tip)]
        );
        let aimed = drag(&bubble, Grip::End, Point::new(90.0, 90.0), false);
        let Kind::Bubble {
            center: kept, tail, ..
        } = aimed.kind
        else {
            panic!()
        };
        assert_eq!((kept, tail), (center, Point::new(90.0, 90.0)));
        let moved = drag(&bubble, Grip::Start, Point::new(70.0, 0.0), false);
        let Kind::Bubble {
            center: body, tail, ..
        } = moved.kind
        else {
            panic!()
        };
        assert_eq!((body, tail), (Point::new(70.0, 0.0), tip));
    }

    #[test]
    fn resize_directions_follow_the_turn() {
        use std::f64::consts::{FRAC_PI_2, FRAC_PI_4};
        let close = |a: Option<f64>, b: f64| assert!((a.unwrap() - b).abs() < 1e-9, "{a:?} vs {b}");
        close(pull_angle(&rectangle(), Grip::Corner(2)), FRAC_PI_4);
        close(pull_angle(&rectangle(), Grip::Edge(1)), 0.0);
        close(pull_angle(&rectangle(), Grip::Edge(0)), -FRAC_PI_2);
        let turned = Shape {
            rotation: FRAC_PI_2,
            ..rectangle()
        };
        close(pull_angle(&turned, Grip::Edge(1)), FRAC_PI_2);
        assert_eq!(pull_angle(&rectangle(), Grip::Rotate), None);
        let line = shape(Kind::Line {
            from: Point::ORIGIN,
            to: Point::new(10.0, 0.0),
        });
        assert_eq!(pull_angle(&line, Grip::End), None);
    }

    #[test]
    fn corners_and_edges_resize_from_the_far_side() {
        let dragged = drag(
            &rectangle(),
            Grip::Corner(2),
            Point::new(150.0, 80.0),
            false,
        );
        assert_eq!(dragged.kind.bounds(), Rect::new(0.0, 0.0, 150.0, 80.0));
        let edge = drag(&rectangle(), Grip::Edge(3), Point::new(-20.0, 999.0), false);
        assert_eq!(edge.kind.bounds(), Rect::new(-20.0, 0.0, 100.0, 50.0));
        let square = drag(&rectangle(), Grip::Corner(2), Point::new(120.0, 60.0), true);
        assert_eq!(square.kind.bounds(), Rect::new(0.0, 0.0, 120.0, 120.0));
        assert_eq!(
            grip_at(&rectangle(), Point::new(101.0, 49.0), 4.0, 1.0),
            Some(Grip::Corner(2))
        );
    }

    #[test]
    fn rotating_turns_about_the_center_and_snaps() {
        let turned = drag(&rectangle(), Grip::Rotate, Point::new(100.0, 25.0), false);
        assert!((turned.rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        let snapped = drag(&rectangle(), Grip::Rotate, Point::new(100.0, 22.0), true);
        assert!((snapped.rotation - std::f64::consts::FRAC_PI_2).abs() < 1e-9);
        assert!(
            turned.distance(Point::new(50.0, 70.0)) < 1e-9,
            "hit inside the turned box"
        );
    }

    #[test]
    fn a_turned_box_keeps_its_far_corner_when_resized() {
        let mut turned = rectangle();
        turned.rotation = std::f64::consts::FRAC_PI_2;
        let far = turned.turn() * Point::new(0.0, 0.0);
        let near = turned.turn() * Point::new(100.0, 50.0);
        let resized = drag(
            &turned,
            Grip::Corner(2),
            near + Vec2::new(-10.0, 30.0),
            false,
        );
        let far_after = resized.turn() * resized.kind.bounds().origin();
        assert!(far.distance(far_after) < 1e-6, "{far:?} vs {far_after:?}");
    }

    #[test]
    fn line_ends_move_and_snap() {
        let line = shape(Kind::Arrow {
            from: Point::ORIGIN,
            to: Point::new(10.0, 0.0),
        });
        let moved = drag(&line, Grip::End, Point::new(30.0, 4.0), true);
        let Kind::Arrow { from, to } = moved.kind else {
            panic!()
        };
        assert_eq!(from, Point::ORIGIN);
        assert!(to.y.abs() < 1e-9);
        let start = drag(&line, Grip::Start, Point::new(-5.0, -5.0), false);
        assert_eq!(start.kind.bounds(), Rect::new(-5.0, -5.0, 10.0, 0.0));
    }

    #[test]
    fn pictures_keep_their_proportions_from_the_corners() {
        let picture = shape(Kind::Image {
            rect: Rect::new(0.0, 0.0, 40.0, 20.0),
            pixels: crate::pixels::Pixels::new(image::RgbaImage::new(4, 2)),
        });
        assert_eq!(grips(&picture, 1.0).len(), 8, "pictures do not turn");
        let wider = drag(&picture, Grip::Corner(2), Point::new(80.0, 25.0), false);
        assert_eq!(wider.kind.bounds(), Rect::new(0.0, 0.0, 80.0, 40.0));
        let flipped = drag(&picture, Grip::Corner(0), Point::new(50.0, 25.0), false);
        assert_eq!(flipped.kind.bounds(), Rect::new(40.0, 20.0, 50.0, 25.0));
        let free = drag(&picture, Grip::Corner(2), Point::new(80.0, 25.0), true);
        assert_eq!(free.kind.bounds(), Rect::new(0.0, 0.0, 80.0, 25.0));
        let edge = drag(&picture, Grip::Edge(1), Point::new(60.0, 99.0), false);
        assert_eq!(edge.kind.bounds(), Rect::new(0.0, 0.0, 60.0, 20.0));
    }

    #[test]
    fn strokes_stretch_with_their_box() {
        let pen = shape(Kind::Pen {
            points: vec![Point::ORIGIN, Point::new(10.0, 10.0)],
        });
        let stretched = drag(&pen, Grip::Corner(2), Point::new(20.0, 40.0), false);
        assert_eq!(stretched.kind.bounds(), Rect::new(0.0, 0.0, 20.0, 40.0));
        assert_eq!(grips(&pen, 1.0).len(), 4);
        let still = drag(&pen, Grip::Edge(0), Point::ORIGIN, false);
        assert_eq!(still, pen);
    }
}
