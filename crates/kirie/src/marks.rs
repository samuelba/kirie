//! The marks picked with the select tool, which the overlay and the editor treat alike: several
//! at once, by Shift+click or by a box dragged around them.

use kirie_annotate::{Document, Draft, Kind, Op, Paint, ShapeId};

/// Adds `id` to the picked marks, or takes it out.
pub fn toggle(picked: &mut Vec<ShapeId>, id: ShapeId) {
    if let Some(at) = picked.iter().position(|one| *one == id) {
        picked.remove(at);
    } else {
        picked.push(id);
    }
}

/// What a press with the select tool does to the picked marks: a move takes the marks it
/// moves, and a marquee lets go of them unless it adds to them.
pub fn pressed(draft: &Draft, picked: &mut Vec<ShapeId>) {
    match draft {
        Draft::Moving { ids, .. } => picked.clone_from(ids),
        Draft::Marquee { extend: false, .. } => picked.clear(),
        _ => {}
    }
}

/// Lets go of `draft`: a marquee picks the marks inside it, a click on a picked mark without a
/// move picks it alone, and anything else gives the edit it makes.
pub fn release(
    draft: Draft,
    document: &Document,
    picked: &mut Vec<ShapeId>,
    paint: Paint,
) -> Option<Op> {
    if let Draft::Moving { held, by, .. } = &draft
        && by.hypot() < f64::EPSILON
    {
        *picked = vec![*held];
    }
    let Some(rect) = draft.marquee() else {
        return draft.finish(paint);
    };
    let found = document.within(rect);
    if !matches!(draft, Draft::Marquee { extend: true, .. }) {
        picked.clear();
    }
    for id in found {
        if !picked.contains(&id) {
            picked.push(id);
        }
    }
    None
}

/// The edit that changes every picked mark's paint with `change`, in one step.
pub fn repaint(document: &Document, picked: &[ShapeId], change: impl Fn(&mut Paint)) -> Option<Op> {
    let mut ops: Vec<Op> = picked
        .iter()
        .filter_map(|id| document.shape(*id))
        .map(|shape| {
            let mut paint = shape.paint;
            change(&mut paint);
            Op::Repaint {
                ids: vec![shape.id],
                paint,
            }
        })
        .collect();
    match ops.len() {
        0 => None,
        1 => ops.pop(),
        _ => Some(Op::Batch(ops)),
    }
}

/// The paint the controls show: the first picked mark's, else `fallback`, the next mark's.
pub fn shown_paint(document: &Document, picked: &[ShapeId], fallback: Paint) -> Paint {
    picked
        .iter()
        .find_map(|id| document.shape(*id))
        .map_or(fallback, |shape| shape.paint)
}

/// Whether marks are picked and each is of a kind that `is` accepts.
pub fn all(document: &Document, picked: &[ShapeId], is: impl Fn(&Kind) -> bool) -> bool {
    let mut shapes = picked
        .iter()
        .filter_map(|id| document.shape(*id))
        .peekable();
    shapes.peek().is_some() && shapes.all(|shape| is(&shape.kind))
}

/// Forgets picked marks that are gone, after an undo or a redo.
pub fn prune(document: &Document, picked: &mut Vec<ShapeId>) {
    picked.retain(|id| document.shape(*id).is_some());
}

#[cfg(test)]
mod tests {
    use kirie_annotate::Op;
    use kurbo::{Point, Rect};

    use super::*;

    fn boxes() -> Document {
        let rectangle = |x: f64| Op::Add {
            kind: Kind::Rectangle {
                rect: Rect::new(x, 10.0, x + 20.0, 30.0),
            },
            paint: Paint::default(),
        };
        Document::new(200, 100, 1.0)
            .apply(rectangle(10.0))
            .apply(rectangle(50.0))
            .apply(rectangle(90.0))
    }

    #[test]
    fn toggling_adds_and_removes() {
        let mut picked = vec![ShapeId(1)];
        toggle(&mut picked, ShapeId(2));
        assert_eq!(picked, [ShapeId(1), ShapeId(2)]);
        toggle(&mut picked, ShapeId(1));
        assert_eq!(picked, [ShapeId(2)]);
    }

    #[test]
    fn a_marquee_picks_what_it_surrounds() {
        let document = boxes();
        let marquee = |from, to, extend| Draft::Marquee {
            start: Point::new(from, 0.0),
            end: Point::new(to, 50.0),
            extend,
        };
        let paint = Paint::default();
        let mut picked = vec![ShapeId(3)];
        let edit = release(marquee(0.0, 75.0, false), &document, &mut picked, paint);
        assert_eq!(edit, None);
        assert_eq!(picked, [ShapeId(1), ShapeId(2)]);
        release(marquee(85.0, 120.0, true), &document, &mut picked, paint);
        assert_eq!(picked, [ShapeId(1), ShapeId(2), ShapeId(3)]);
        release(marquee(150.0, 150.0, false), &document, &mut picked, paint);
        assert!(
            picked.is_empty(),
            "a click on nothing lets go of everything"
        );
    }

    #[test]
    fn a_click_on_one_of_several_picks_it_alone_and_a_drag_moves_them_all() {
        let document = boxes();
        let paint = Paint::default();
        let moving = |by| Draft::Moving {
            ids: vec![ShapeId(1), ShapeId(2)],
            held: ShapeId(2),
            start: Point::ORIGIN,
            by,
        };
        let mut picked = Vec::new();
        pressed(&moving(kurbo::Vec2::ZERO), &mut picked);
        assert_eq!(picked, [ShapeId(1), ShapeId(2)]);
        let moved = release(
            moving(kurbo::Vec2::new(4.0, 0.0)),
            &document,
            &mut picked,
            paint,
        );
        assert!(moved.is_some());
        assert_eq!(picked, [ShapeId(1), ShapeId(2)]);
        assert!(release(moving(kurbo::Vec2::ZERO), &document, &mut picked, paint).is_none());
        assert_eq!(picked, [ShapeId(2)]);
        let extending = Draft::Marquee {
            start: Point::ORIGIN,
            end: Point::ORIGIN,
            extend: true,
        };
        pressed(&extending, &mut picked);
        assert_eq!(picked, [ShapeId(2)], "Shift keeps the picked marks");
    }

    #[test]
    fn repainting_keeps_each_marks_own_paint() {
        let document = boxes().apply(Op::Repaint {
            ids: vec![ShapeId(2)],
            paint: Paint {
                filled: true,
                ..Paint::default()
            },
        });
        let picked = [ShapeId(1), ShapeId(2)];
        let red = kirie_annotate::Rgba::rgb(255, 0, 0);
        let op = repaint(&document, &picked, |paint| paint.color = red).unwrap();
        let painted = document.apply(op);
        let paint = |id| painted.shape(id).unwrap().paint;
        assert_eq!(paint(ShapeId(1)).color, red);
        assert_eq!(paint(ShapeId(2)).color, red);
        assert!(paint(ShapeId(2)).filled && !paint(ShapeId(1)).filled);
        assert_ne!(paint(ShapeId(3)).color, red);
        assert_eq!(repaint(&document, &[], |_| {}), None);
        assert!(matches!(
            repaint(&document, &[ShapeId(3)], |_| {}),
            Some(Op::Repaint { .. })
        ));
    }

    #[test]
    fn controls_follow_the_picked_marks() {
        let document = boxes();
        let rectangles = |kind: &Kind| matches!(kind, Kind::Rectangle { .. });
        assert!(all(&document, &[ShapeId(1), ShapeId(3)], rectangles));
        assert!(!all(&document, &[], rectangles));
        assert!(!all(&document, &[ShapeId(1)], |kind| matches!(
            kind,
            Kind::Text { .. }
        )));
        let fallback = Paint {
            filled: true,
            ..Paint::default()
        };
        assert!(shown_paint(&document, &[], fallback).filled);
        assert!(!shown_paint(&document, &[ShapeId(9), ShapeId(1)], fallback).filled);
        let mut picked = vec![ShapeId(1), ShapeId(9)];
        prune(&document, &mut picked);
        assert_eq!(picked, [ShapeId(1)]);
    }
}
