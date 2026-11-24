//! The marks picked with the select tool, which the overlay and the editor treat alike: several
//! at once, by Shift+click or by a box dragged around them; and copies of them, duplicated or
//! passed through the clipboard.

use crate::{Document, Draft, Kind, Op, Paint, Shape, ShapeId, TextLook};
use kurbo::Vec2;
use serde::{Deserialize, Serialize};

/// The clipboard type copied marks are offered as.
pub const MIME: &str = "application/x-kirie-marks";
/// How far a duplicate, or a paste over its own original, lands from it, in logical pixels.
const STEP: f64 = 12.0;
/// The most marks one paste takes.
const MOST: usize = 5000;
/// How far out a pasted mark may reach, in image pixels; anything beyond is not a real mark.
const FARTHEST: f64 = 1.0e6;
/// The most characters a sticker's emoji takes: the longest emoji sequences have about ten.
const LONGEST_EMOJI: usize = 16;
/// The most bytes of marks read from the clipboard.
pub const MAX_BYTES: usize = 256 * 1024 * 1024;
/// The most pixels the image marks of one paste hold together.
const MAX_PIXELS: u64 = 2 * crate::pixels::MAX_PIXELS;
/// The longest pasted text, in characters, and the most points of a pasted stroke.
const LONGEST: usize = 100_000;
/// The highest step number pasted.
const HIGHEST: u32 = 9999;

/// Marks on the clipboard.
#[derive(Debug, Serialize, Deserialize)]
struct Clip {
    marks: Vec<Shape>,
}

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

/// Whether the sticker picker applies: to the sticker tool or picked stickers.
pub fn stickering(tool: Option<crate::Tool>, document: &Document, picked: &[ShapeId]) -> bool {
    tool == Some(crate::Tool::Sticker)
        || all(document, picked, |kind| {
            matches!(kind, Kind::Sticker { .. })
        })
}

/// The emoji the first picked sticker shows.
pub fn sticker_shown<'a>(document: &'a Document, picked: &[ShapeId]) -> Option<&'a str> {
    picked
        .iter()
        .filter_map(|id| document.shape(*id))
        .find_map(|shape| match &shape.kind {
            Kind::Sticker { emoji, .. } => Some(emoji.as_str()),
            _ => None,
        })
}

/// The edit that makes the picked stickers show `emoji`, if any of them shows another.
pub fn restick(document: &Document, picked: &[ShapeId], emoji: &str) -> Option<Op> {
    let ops: Vec<Op> = picked
        .iter()
        .filter_map(|id| document.shape(*id))
        .filter_map(|shape| match &shape.kind {
            Kind::Sticker { rect, emoji: shown } if shown != emoji => Some(Op::Replace {
                id: shape.id,
                kind: Kind::Sticker {
                    rect: *rect,
                    emoji: emoji.to_owned(),
                },
            }),
            _ => None,
        })
        .collect();
    (!ops.is_empty()).then_some(Op::Batch(ops))
}

/// Where the picked marks go among the others.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Restack {
    Front,
    Back,
    /// One place up.
    Forward,
    /// One place down.
    Backward,
}

/// The edit that moves the picked marks `to` their new place among the others, keeping their
/// order among themselves.
pub fn restack(document: &Document, picked: &[ShapeId], to: Restack) -> Option<Op> {
    let ids: Vec<ShapeId> = document
        .shapes()
        .map(|shape| shape.id)
        .filter(|id| picked.contains(id))
        .collect();
    if ids.is_empty() {
        return None;
    }
    Some(match to {
        Restack::Front => Op::Raise { ids },
        Restack::Back => Op::Lower { ids },
        Restack::Forward => Op::Shift { ids, up: true },
        Restack::Backward => Op::Shift { ids, up: false },
    })
}

/// The restacking `key` with Ctrl asks for: ] and [ one place, and } and { (Shift+] and
/// Shift+[ where those are) all the way. By the character alone, so layouts that need Shift for
/// [ and ] still reach every one.
pub fn restack_key(key: &str) -> Option<Restack> {
    match key {
        "}" => Some(Restack::Front),
        "{" => Some(Restack::Back),
        "]" => Some(Restack::Forward),
        "[" => Some(Restack::Backward),
        _ => None,
    }
}

/// Forgets picked marks that are gone, after an undo or a redo.
pub fn prune(document: &Document, picked: &mut Vec<ShapeId>) {
    picked.retain(|id| document.shape(*id).is_some());
}

/// The picked marks, bottom first.
pub fn of(document: &Document, picked: &[ShapeId]) -> Vec<Shape> {
    document
        .shapes()
        .filter(|shape| picked.contains(&shape.id))
        .cloned()
        .collect()
}

/// `marks` as the clipboard holds them. Image marks make this slow: run it off the UI thread.
pub fn encode(marks: Vec<Shape>) -> Result<Vec<u8>, String> {
    ron::to_string(&Clip { marks })
        .map(String::into_bytes)
        .map_err(|error| format!("cannot copy the marks: {error}"))
}

/// Marks read from the clipboard, which any program may have put there: marks out of all
/// reason are left out.
pub fn decode(bytes: &[u8]) -> Result<Vec<Shape>, String> {
    if bytes.len() > MAX_BYTES {
        return Err("too many marks are copied to paste them".to_owned());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| "the copied marks are not text")?;
    let clip: Clip = crate::pixels::budget(MAX_PIXELS, || ron::from_str(text))
        .map_err(|error| format!("the copied marks cannot be read: {error}"))?;
    if clip.marks.len() > MOST {
        return Err(format!("more than {MOST} marks cannot be pasted at once"));
    }
    Ok(clip.marks.into_iter().filter_map(sane).collect())
}

/// `shape` if its numbers are finite and near and its text and strokes not endless, with its
/// text size and step number kept in range, and turned only if its kind turns.
fn sane(shape: Shape) -> Option<Shape> {
    let bounds = shape.kind.bounds();
    let near = [bounds.x0, bounds.y0, bounds.x1, bounds.y1]
        .iter()
        .all(|value| value.is_finite() && value.abs() <= FARTHEST);
    let bounded = match &shape.kind {
        Kind::Text { text, .. } | Kind::Bubble { text, .. } => text.chars().count() <= LONGEST,
        Kind::Pen { points } | Kind::Highlighter { points } => points.len() <= LONGEST,
        Kind::Image { .. } => bounds.width() > 0.0 && bounds.height() > 0.0,
        Kind::Sticker { emoji, .. } => emoji.chars().count() <= LONGEST_EMOJI,
        _ => true,
    };
    if !near || !bounded || !shape.rotation.is_finite() {
        return None;
    }
    let kind = match shape.kind {
        Kind::Counter { at, number } => Kind::Counter {
            at,
            number: number.min(HIGHEST),
        },
        kind => kind,
    };
    let rotation = if crate::edit::turns(&kind) {
        shape.rotation
    } else {
        0.0
    };
    let text = shape.paint.text;
    let size = if text.size.is_finite() {
        text.size.clamp(TextLook::MIN_SIZE, TextLook::MAX_SIZE)
    } else {
        TextLook::default().size
    };
    Some(Shape {
        kind,
        rotation,
        paint: Paint {
            text: TextLook { size, ..text },
            ..shape.paint
        },
        ..shape
    })
}

/// The edit that pastes `clip` into `document`, and the ids the copies get. They land where
/// they were copied from, stepped aside from copies already there, and on the middle of the
/// picture when they would all fall beside it.
pub fn paste(document: &Document, clip: &[Shape]) -> Option<(Op, Vec<ShapeId>)> {
    let bounds = clip
        .iter()
        .map(|shape| shape.kind.bounds())
        .reduce(|all, one| all.union(one))?;
    let visible = document.visible();
    let onto = if bounds.intersect(visible).area() > 0.0 || visible.contains(bounds.center()) {
        Vec2::ZERO
    } else {
        visible.center() - bounds.center()
    };
    let step = Vec2::new(STEP, STEP) * document.unit;
    let key = |rect: kurbo::Rect| [rect.x0, rect.y0, rect.x1, rect.y1].map(f64::to_bits);
    let mut placed: std::collections::HashMap<_, Vec<&Kind>> = std::collections::HashMap::new();
    for shape in document.shapes() {
        placed
            .entry(key(shape.kind.bounds()))
            .or_default()
            .push(&shape.kind);
    }
    let taken = |by: Vec2| {
        clip.iter().any(|copy| {
            let kind = copy.kind.translated(by);
            placed
                .get(&key(kind.bounds()))
                .is_some_and(|kinds| kinds.contains(&&kind))
        })
    };
    let by = (0..100u32)
        .map(|n| onto + step * f64::from(n))
        .find(|by| !taken(*by))
        .unwrap_or(onto);
    Some(insert(document, clip, by))
}

/// The edit that copies the picked marks a step aside, and the ids the copies get.
pub fn duplicate(document: &Document, picked: &[ShapeId]) -> Option<(Op, Vec<ShapeId>)> {
    let marks: Vec<Shape> = document
        .shapes()
        .filter(|shape| picked.contains(&shape.id))
        .cloned()
        .collect();
    (!marks.is_empty()).then(|| insert(document, &marks, Vec2::new(STEP, STEP) * document.unit))
}

/// The edit that puts `image` over the picture as a mark, and its id: in the middle of what is
/// kept, shrunk to fit four fifths of it.
pub fn place_image(document: &Document, image: image::RgbaImage) -> Result<(Op, ShapeId), String> {
    let most = crate::pixels::MAX_SIDE;
    if image.width() == 0 || image.height() == 0 {
        return Err("the pasted image is empty".to_owned());
    }
    if !crate::pixels::fits(image.width(), image.height()) {
        return Err(format!(
            "images over {most} pixels wide or tall, or of over {} megapixels, cannot be pasted",
            crate::pixels::MAX_PIXELS / 1_000_000
        ));
    }
    let visible = document.visible();
    let (width, height) = (f64::from(image.width()), f64::from(image.height()));
    let shrink = (0.8 * visible.width() / width)
        .min(0.8 * visible.height() / height)
        .min(1.0);
    let size = kurbo::Size::new(width * shrink, height * shrink);
    let rect = kurbo::Rect::from_center_size(visible.center(), size);
    let op = Op::Add {
        kind: Kind::Image {
            rect,
            pixels: crate::Pixels::new(image),
        },
        paint: Paint::default(),
    };
    Ok((op, document.next_id()))
}

fn insert(document: &Document, marks: &[Shape], by: Vec2) -> (Op, Vec<ShapeId>) {
    let first = document.next_id().0;
    let ids = (first..).take(marks.len()).map(ShapeId).collect();
    let shapes = marks
        .iter()
        .map(|shape| Shape {
            kind: shape.kind.translated(by),
            ..shape.clone()
        })
        .collect();
    (Op::Insert { shapes }, ids)
}

#[cfg(test)]
mod tests {
    use crate::Op;
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
    fn copied_marks_survive_the_clipboard() {
        let document = boxes();
        assert!(of(&document, &[]).is_empty());
        let bytes = encode(of(&document, &[ShapeId(3), ShapeId(1)])).unwrap();
        let marks = decode(&bytes).unwrap();
        let ids: Vec<ShapeId> = marks.iter().map(|shape| shape.id).collect();
        assert_eq!(ids, [ShapeId(1), ShapeId(3)], "bottom first, as drawn");
        assert_eq!(marks[1], *document.shape(ShapeId(3)).unwrap());
        assert!(decode(b"not marks").is_err());
        assert!(decode(&[0xff, 0xfe]).is_err());
    }

    #[test]
    fn unreasonable_marks_are_left_out() {
        let mut far = boxes().shape(ShapeId(1)).unwrap().clone();
        far.kind = Kind::Rectangle {
            rect: Rect::new(0.0, 0.0, 1.0e9, 5.0),
        };
        let mut huge = boxes().shape(ShapeId(2)).unwrap().clone();
        huge.paint.text.size = 1.0e6;
        let mut counter = huge.clone();
        counter.kind = Kind::Counter {
            at: Point::ORIGIN,
            number: u32::MAX,
        };
        counter.rotation = 1.0;
        let mut endless = huge.clone();
        endless.kind = Kind::Pen {
            points: vec![Point::ORIGIN; LONGEST + 1],
        };
        let bytes = ron::to_string(&Clip {
            marks: vec![far, huge, counter, endless],
        })
        .unwrap();
        let marks = decode(bytes.as_bytes()).unwrap();
        assert_eq!(marks.len(), 2);
        assert!((marks[0].paint.text.size - TextLook::MAX_SIZE).abs() < f64::EPSILON);
        assert!(matches!(
            marks[1].kind,
            Kind::Counter {
                number: HIGHEST,
                ..
            }
        ));
        assert!(
            marks[1].rotation.abs() < f64::EPSILON,
            "step numbers do not turn"
        );
    }

    #[test]
    fn pastes_land_in_place_then_step_aside() {
        let document = Document::new(200, 100, 1.0);
        let clip: Vec<Shape> = boxes().shapes().take(1).cloned().collect();
        let (op, ids) = paste(&document, &clip).unwrap();
        assert_eq!(ids, [ShapeId(1)]);
        let once = document.apply(op);
        assert_eq!(once.shape(ShapeId(1)).unwrap().kind, clip[0].kind);
        let (op, ids) = paste(&once, &clip).unwrap();
        assert_eq!(ids, [ShapeId(2)]);
        let twice = once.apply(op);
        let stepped = twice.shape(ShapeId(2)).unwrap().kind.bounds();
        assert_eq!(stepped.origin(), Point::new(10.0 + STEP, 10.0 + STEP));
        let (op, _) = paste(&twice, &clip).unwrap();
        let thrice = twice.apply(op);
        let again = thrice.shape(ShapeId(3)).unwrap().kind.bounds();
        assert_eq!(
            again.origin(),
            Point::new(10.0 + 2.0 * STEP, 10.0 + 2.0 * STEP)
        );
        assert!(paste(&document, &[]).is_none());
    }

    #[test]
    fn marks_from_a_larger_picture_come_onto_this_one() {
        let small = Document::new(100, 100, 1.0);
        let mut far = boxes().shape(ShapeId(1)).unwrap().clone();
        far.kind = Kind::Rectangle {
            rect: Rect::new(900.0, 900.0, 920.0, 920.0),
        };
        let (op, _) = paste(&small, &[far]).unwrap();
        let pasted = small.apply(op);
        let bounds = pasted.shapes().next().unwrap().kind.bounds();
        assert_eq!(bounds.center(), Point::new(50.0, 50.0));
    }

    #[test]
    fn pasted_images_sit_in_the_middle_and_fit() {
        let document = Document::new(200, 100, 1.0);
        let (op, id) = place_image(&document, image::RgbaImage::new(20, 10)).unwrap();
        let placed = document.apply(op);
        assert_eq!(id, ShapeId(1));
        assert_eq!(
            placed.shape(id).unwrap().kind.bounds(),
            Rect::new(90.0, 45.0, 110.0, 55.0)
        );
        let (op, id) = place_image(&document, image::RgbaImage::new(400, 100)).unwrap();
        let shrunk = document.apply(op).shape(id).unwrap().kind.bounds();
        assert_eq!(
            shrunk,
            Rect::new(20.0, 30.0, 180.0, 70.0),
            "four fifths of the width"
        );
        assert!(place_image(&document, image::RgbaImage::new(0, 4)).is_err());
    }

    #[test]
    fn duplicates_sit_a_step_aside_on_top() {
        let document = boxes();
        assert!(duplicate(&document, &[]).is_none());
        let (op, ids) = duplicate(&document, &[ShapeId(2), ShapeId(1)]).unwrap();
        assert_eq!(ids, [ShapeId(4), ShapeId(5)]);
        let doubled = document.apply(op);
        assert_eq!(doubled.shapes().len(), 5);
        let copy = doubled.shape(ShapeId(4)).unwrap().kind.bounds();
        assert_eq!(copy.origin(), Point::new(10.0 + STEP, 10.0 + STEP));
    }

    #[test]
    fn restacking_keeps_the_picked_marks_in_their_order() {
        let document = boxes();
        let order = |op| {
            document
                .apply(op)
                .shapes()
                .map(|shape| shape.id.0)
                .collect::<Vec<_>>()
        };
        let picked = [ShapeId(3), ShapeId(1)];
        assert_eq!(
            order(restack(&document, &picked, Restack::Front).unwrap()),
            [2, 1, 3]
        );
        assert_eq!(
            order(restack(&document, &picked, Restack::Back).unwrap()),
            [1, 3, 2]
        );
        let forward = restack(&document, &[ShapeId(1)], Restack::Forward).unwrap();
        assert_eq!(order(forward), [2, 1, 3]);
        assert!(restack(&document, &[], Restack::Front).is_none());
        assert_eq!(restack_key("}"), Some(Restack::Front));
        assert_eq!(restack_key("["), Some(Restack::Backward));
        assert_eq!(restack_key("a"), None);
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
        let red = crate::Rgba::rgb(255, 0, 0);
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

    #[test]
    fn picked_stickers_change_their_emoji() {
        let sticker = |emoji: &str| Kind::Sticker {
            rect: kurbo::Rect::new(0.0, 0.0, 40.0, 40.0),
            emoji: emoji.into(),
        };
        let document = boxes()
            .apply(Op::Add {
                kind: sticker("a"),
                paint: Paint::default(),
            })
            .apply(Op::Add {
                kind: sticker("b"),
                paint: Paint::default(),
            });
        let stickers: Vec<ShapeId> = document.shapes().skip(3).map(|shape| shape.id).collect();
        assert!(stickering(None, &document, &stickers));
        assert!(!stickering(None, &document, &[ShapeId(1), stickers[0]]));
        assert!(stickering(Some(crate::Tool::Sticker), &document, &[]));
        let changed = document.apply(restick(&document, &stickers, "b").unwrap());
        for id in &stickers {
            assert_eq!(changed.shape(*id).unwrap().kind, sticker("b"));
        }
        assert_eq!(restick(&changed, &stickers, "b"), None);
        assert_eq!(restick(&document, &[ShapeId(1)], "b"), None);
        let mut pasted = changed.shape(stickers[0]).unwrap().clone();
        assert!(sane(pasted.clone()).is_some());
        pasted.kind = sticker(&"x".repeat(LONGEST_EMOJI + 1));
        assert!(sane(pasted).is_none());
    }
}
