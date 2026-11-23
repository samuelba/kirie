//! The select tool's pointer over marks: it shows what a press would do and what a drag holds,
//! a resize arrow on corners and edges, turned with the mark, a hand on the rotate knob and a
//! move cross on the mark itself.

use std::f64::consts::FRAC_PI_4;

use iced::mouse::Interaction;
use kirie_annotate::edit::{self, Grip};
use kirie_annotate::{Document, Draft, Shape, ShapeId};

/// The cursor at `at` with the select tool. `reach` is how near a mark or grip must be, in mark
/// coordinates, as given to [`kirie_annotate::tools::press`]; only a lone picked mark has grips.
pub fn select(
    document: &Document,
    picked: &[ShapeId],
    draft: Option<&Draft>,
    at: kurbo::Point,
    reach: f64,
) -> Interaction {
    match draft {
        Some(Draft::Editing { grip, current, .. }) => return holding(current, *grip),
        Some(Draft::Moving { .. }) => return Interaction::Move,
        Some(Draft::Marquee { .. }) => return Interaction::Crosshair,
        _ => {}
    }
    let lone = match picked {
        [id] => Some(*id),
        _ => None,
    };
    let on_grip = lone
        .and_then(|id| document.shape(id))
        .and_then(|shape| Some((shape, edit::grip_at(shape, at, reach, reach / 6.0)?)));
    if let Some((shape, grip)) = on_grip {
        over(shape, grip)
    } else if document.hit(at, reach).is_some() {
        Interaction::Move
    } else {
        Interaction::default()
    }
}

fn over(shape: &Shape, grip: Grip) -> Interaction {
    match edit::pull_angle(shape, grip) {
        Some(angle) => resize(angle),
        None if grip == Grip::Rotate => Interaction::Grab,
        None => Interaction::Crosshair,
    }
}

fn holding(shape: &Shape, grip: Grip) -> Interaction {
    if grip == Grip::Rotate {
        Interaction::Grabbing
    } else {
        over(shape, grip)
    }
}

/// The resize arrow nearest to `angle` (0 is right, y grows down).
fn resize(angle: f64) -> Interaction {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    match (angle / FRAC_PI_4).round().rem_euclid(4.0) as u8 {
        0 => Interaction::ResizingHorizontally,
        1 => Interaction::ResizingDiagonallyDown,
        2 => Interaction::ResizingVertically,
        _ => Interaction::ResizingDiagonallyUp,
    }
}

#[cfg(test)]
mod tests {
    use std::f64::consts::FRAC_PI_2;

    use kirie_annotate::{Kind, Op, Paint};
    use kurbo::{Point, Rect};

    use super::*;

    fn picked_box() -> (Document, ShapeId) {
        let document = Document::new(200, 200, 1.0);
        let id = document.next_id();
        let document = document.apply(Op::Add {
            kind: Kind::Rectangle {
                rect: Rect::new(20.0, 40.0, 120.0, 90.0),
            },
            paint: Paint::default(),
        });
        (document, id)
    }

    #[test]
    fn arrows_point_the_way_a_grip_pulls() {
        assert_eq!(resize(0.0), Interaction::ResizingHorizontally);
        assert_eq!(resize(FRAC_PI_2), Interaction::ResizingVertically);
        assert_eq!(resize(FRAC_PI_4), Interaction::ResizingDiagonallyDown);
        assert_eq!(
            resize(-3.0 * FRAC_PI_4),
            Interaction::ResizingDiagonallyDown
        );
        assert_eq!(resize(-FRAC_PI_4), Interaction::ResizingDiagonallyUp);
        assert_eq!(resize(0.3), Interaction::ResizingHorizontally);
    }

    #[test]
    fn hovering_tells_grips_from_the_mark() {
        let (document, id) = picked_box();
        let at = |x, y| select(&document, &[id], None, Point::new(x, y), 6.0);
        assert_eq!(at(120.0, 90.0), Interaction::ResizingDiagonallyDown);
        assert_eq!(at(120.0, 40.0), Interaction::ResizingDiagonallyUp);
        assert_eq!(at(70.0, 40.0), Interaction::ResizingVertically);
        assert_eq!(at(20.0, 65.0), Interaction::ResizingHorizontally);
        assert_eq!(at(70.0, 40.0 - edit::ROTATE_OFFSET), Interaction::Grab);
        assert_eq!(at(70.0, 90.5), Interaction::ResizingVertically);
        assert_eq!(at(50.0, 90.0), Interaction::Move);
        assert_eq!(at(180.0, 180.0), Interaction::default());
        let unpicked = select(&document, &[], None, Point::new(120.0, 90.0), 6.0);
        assert_eq!(unpicked, Interaction::Move, "no grips without a pick");
        let several = select(&document, &[id, id], None, Point::new(120.0, 90.0), 6.0);
        assert_eq!(several, Interaction::Move, "no grips on several picks");
    }

    #[test]
    fn a_turned_box_turns_its_arrows() {
        let (document, id) = picked_box();
        let shape = document.shape(id).unwrap().clone();
        let turned = Shape {
            rotation: FRAC_PI_2,
            ..shape
        };
        assert_eq!(
            over(&turned, Grip::Edge(0)),
            Interaction::ResizingHorizontally
        );
        assert_eq!(
            over(&turned, Grip::Corner(0)),
            Interaction::ResizingDiagonallyUp
        );
    }

    #[test]
    fn a_drag_keeps_its_cursor() {
        let (document, id) = picked_box();
        let shape = Box::new(document.shape(id).unwrap().clone());
        let editing = |grip| Draft::Editing {
            grip,
            origin: shape.clone(),
            current: shape.clone(),
        };
        let far = Point::new(180.0, 180.0);
        let held = |draft: &Draft| select(&document, &[id], Some(draft), far, 6.0);
        assert_eq!(held(&editing(Grip::Rotate)), Interaction::Grabbing);
        assert_eq!(
            held(&editing(Grip::Edge(1))),
            Interaction::ResizingHorizontally
        );
        assert_eq!(held(&editing(Grip::End)), Interaction::Crosshair);
        let moving = Draft::Moving {
            ids: vec![id],
            held: id,
            start: far,
            by: kurbo::Vec2::ZERO,
        };
        assert_eq!(held(&moving), Interaction::Move);
    }
}
