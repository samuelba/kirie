//! An image and the marks on it. Every edit makes a new document; the marks are shared between
//! versions, so keeping old versions for undo is cheap.

use std::sync::Arc;

use kurbo::{Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::shape::{Kind, Paint, Shape, ShapeId};
use crate::style::StyleId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Document {
    /// The image size in pixels; marks use the same coordinates.
    pub width: u32,
    pub height: u32,
    /// Image pixels per logical pixel, so marks look the same size on high-density captures.
    pub unit: f64,
    pub style: StyleId,
    /// Whether marks cast a shadow, in the styles that draw one.
    #[serde(default = "casts_shadows")]
    pub shadow: bool,
    /// The part of the image that is kept, if cropped.
    pub crop: Option<Rect>,
    shapes: Vec<Arc<Shape>>,
    next_id: u64,
}

/// An edit to a document.
#[derive(Debug, Clone, PartialEq)]
pub enum Op {
    Add {
        kind: Kind,
        paint: Paint,
    },
    Replace {
        id: ShapeId,
        kind: Kind,
    },
    /// A new outline and turn for a mark, as resizing or rotating it makes.
    Reshape {
        id: ShapeId,
        kind: Kind,
        rotation: f64,
    },
    Repaint {
        ids: Vec<ShapeId>,
        paint: Paint,
    },
    Move {
        ids: Vec<ShapeId>,
        by: Vec2,
    },
    Remove {
        ids: Vec<ShapeId>,
    },
    /// Brings marks to the top, in the order given.
    Raise {
        ids: Vec<ShapeId>,
    },
    Clear,
    SetStyle(StyleId),
    SetShadow(bool),
    Crop(Option<Rect>),
}

fn casts_shadows() -> bool {
    true
}

impl Document {
    pub fn new(width: u32, height: u32, unit: f64) -> Self {
        Self {
            width,
            height,
            unit: if unit.is_finite() && unit > 0.0 {
                unit
            } else {
                1.0
            },
            style: StyleId::default(),
            shadow: true,
            crop: None,
            shapes: Vec::new(),
            next_id: 1,
        }
    }

    pub fn shapes(&self) -> impl DoubleEndedIterator<Item = &Shape> + ExactSizeIterator {
        self.shapes.iter().map(AsRef::as_ref)
    }

    pub fn shape(&self, id: ShapeId) -> Option<&Shape> {
        self.shapes().find(|shape| shape.id == id)
    }

    pub fn is_empty(&self) -> bool {
        self.shapes.is_empty()
    }

    /// The id the next added mark gets.
    pub fn next_id(&self) -> ShapeId {
        ShapeId(self.next_id)
    }

    /// The topmost mark within `reach` of `point`.
    pub fn hit(&self, point: kurbo::Point, reach: f64) -> Option<ShapeId> {
        self.shapes()
            .rev()
            .find(|shape| self.distance(shape, point) <= reach)
            .map(|shape| shape.id)
    }

    /// The area `shape` covers here, before stroke width, with text measured in its face.
    pub fn bounds(&self, shape: &Shape) -> Rect {
        crate::style::bounds(shape, self.style, self.unit)
    }

    /// How far `point` is from `shape`, 0 when on or inside it.
    pub fn distance(&self, shape: &Shape, point: kurbo::Point) -> f64 {
        match shape.kind {
            Kind::Text { .. } => {
                let area = self.bounds(shape);
                let dx = (area.x0 - point.x).max(point.x - area.x1).max(0.0);
                let dy = (area.y0 - point.y).max(point.y - area.y1).max(0.0);
                dx.hypot(dy)
            }
            _ => shape.distance(point),
        }
    }

    /// The next number for a step counter.
    pub fn next_counter(&self) -> u32 {
        self.shapes()
            .filter_map(|shape| match shape.kind {
                Kind::Counter { number, .. } => Some(number),
                _ => None,
            })
            .max()
            .unwrap_or(0)
            + 1
    }

    /// The image area kept: the crop, or the whole image.
    pub fn visible(&self) -> Rect {
        self.crop
            .unwrap_or_else(|| Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height)))
    }

    /// This document's marks on another image: `transform` maps this document's coordinates
    /// onto the new image of `width`×`height` and `unit`. The crop is dropped.
    #[must_use]
    pub fn transformed(
        &self,
        transform: kurbo::Affine,
        width: u32,
        height: u32,
        unit: f64,
    ) -> Document {
        let mut next = Document::new(width, height, unit);
        next.style = self.style;
        next.shadow = self.shadow;
        next.next_id = self.next_id;
        next.shapes = self
            .shapes
            .iter()
            .map(|shape| {
                Arc::new(Shape {
                    kind: shape.kind.transformed(transform),
                    ..Shape::clone(shape)
                })
            })
            .collect();
        next
    }

    #[must_use]
    pub fn apply(&self, op: Op) -> Document {
        let mut next = self.clone();
        match op {
            Op::Add { kind, paint } => {
                next.shapes.push(Arc::new(Shape {
                    id: ShapeId(next.next_id),
                    kind,
                    paint,
                    style: None,
                    rotation: 0.0,
                }));
                next.next_id += 1;
            }
            Op::Replace { id, kind } => next.map(&[id], |shape| Shape {
                kind: kind.clone(),
                ..shape.clone()
            }),
            Op::Reshape { id, kind, rotation } => next.map(&[id], |shape| Shape {
                kind: kind.clone(),
                rotation,
                ..shape.clone()
            }),
            Op::Repaint { ids, paint } => next.map(&ids, |shape| Shape {
                paint,
                ..shape.clone()
            }),
            Op::Move { ids, by } => next.map(&ids, |shape| Shape {
                kind: shape.kind.translated(by),
                ..shape.clone()
            }),
            Op::Remove { ids } => next.shapes.retain(|shape| !ids.contains(&shape.id)),
            Op::Raise { ids } => {
                let (raised, kept): (Vec<_>, Vec<_>) = next
                    .shapes
                    .into_iter()
                    .partition(|shape| ids.contains(&shape.id));
                let mut raised = raised;
                raised.sort_by_key(|shape| ids.iter().position(|id| *id == shape.id));
                next.shapes = kept.into_iter().chain(raised).collect();
            }
            Op::Clear => next.shapes.clear(),
            Op::SetStyle(style) => next.style = style,
            Op::SetShadow(shadow) => next.shadow = shadow,
            Op::Crop(crop) => {
                next.crop = crop
                    .map(|rect| self.clamp(rect))
                    .filter(|r| r.area() >= 1.0);
            }
        }
        next
    }

    fn map(&mut self, ids: &[ShapeId], change: impl Fn(&Shape) -> Shape) {
        for shape in &mut self.shapes {
            if ids.contains(&shape.id) {
                *shape = Arc::new(change(shape));
            }
        }
    }

    fn clamp(&self, rect: Rect) -> Rect {
        let image = Rect::new(0.0, 0.0, f64::from(self.width), f64::from(self.height));
        rect.abs().intersect(image)
    }
}

/// Undo and redo over whole documents.
#[derive(Debug, Clone, PartialEq)]
pub struct History {
    current: Document,
    past: Vec<Document>,
    future: Vec<Document>,
    /// The last step was made gradually and nothing has happened since, so it can still grow.
    gradual: bool,
}

impl History {
    /// How many steps back undo can go.
    const LIMIT: usize = 200;

    pub fn new(document: Document) -> Self {
        Self {
            current: document,
            past: Vec::new(),
            future: Vec::new(),
            gradual: false,
        }
    }

    pub fn current(&self) -> &Document {
        &self.current
    }

    pub fn can_undo(&self) -> bool {
        !self.past.is_empty()
    }

    pub fn can_redo(&self) -> bool {
        !self.future.is_empty()
    }

    /// Applies an edit as one undo step. Edits that change nothing leave no step; returns
    /// whether this one did.
    pub fn apply(&mut self, op: Op) -> bool {
        self.gradual = false;
        let next = self.current.apply(op);
        if next == self.current {
            return false;
        }
        self.past.push(std::mem::replace(&mut self.current, next));
        if self.past.len() > Self::LIMIT {
            self.past.remove(0);
        }
        self.future.clear();
        true
    }

    /// Applies one move of an edit made in many, like dragging a slider, so they all undo in
    /// one step. A move `continuing` the same drag joins the step the drag began, unless
    /// something else happened since, such as an undo; then it begins a step of its own.
    pub fn apply_gradually(&mut self, op: Op, continuing: bool) {
        if continuing && self.gradual {
            self.current = self.current.apply(op);
        } else {
            self.gradual = self.apply(op);
        }
    }

    pub fn undo(&mut self) -> bool {
        self.gradual = false;
        let Some(previous) = self.past.pop() else {
            return false;
        };
        self.future
            .push(std::mem::replace(&mut self.current, previous));
        true
    }

    pub fn redo(&mut self) -> bool {
        self.gradual = false;
        let Some(next) = self.future.pop() else {
            return false;
        };
        self.past.push(std::mem::replace(&mut self.current, next));
        true
    }
}

#[cfg(test)]
mod tests {
    use kurbo::Point;

    use super::*;

    fn line(x: f64) -> Op {
        Op::Add {
            kind: Kind::Line {
                from: Point::new(x, 0.0),
                to: Point::new(x, 10.0),
            },
            paint: Paint::default(),
        }
    }

    #[test]
    fn adding_numbers_marks_and_keeps_the_original() {
        let empty = Document::new(100, 100, 1.0);
        let one = empty.apply(line(1.0));
        let two = one.apply(line(2.0));
        assert!(empty.is_empty());
        assert_eq!(one.shapes().len(), 1);
        assert_eq!(
            two.shapes().map(|shape| shape.id).collect::<Vec<_>>(),
            [ShapeId(1), ShapeId(2)]
        );
        assert_eq!(two.next_id(), ShapeId(3));
        assert!(two.shape(ShapeId(2)).is_some());
    }

    #[test]
    fn bad_units_fall_back_to_one() {
        assert!((Document::new(1, 1, 0.0).unit - 1.0).abs() < f64::EPSILON);
        assert!((Document::new(1, 1, f64::NAN).unit - 1.0).abs() < f64::EPSILON);
        assert!((Document::new(1, 1, 2.0).unit - 2.0).abs() < f64::EPSILON);
    }

    #[test]
    fn edits_touch_only_the_named_marks() {
        let doc = Document::new(100, 100, 1.0)
            .apply(line(1.0))
            .apply(line(2.0));
        let moved = doc.apply(Op::Move {
            ids: vec![ShapeId(1)],
            by: Vec2::new(5.0, 0.0),
        });
        assert_eq!(moved.shape(ShapeId(1)).unwrap().kind.bounds().x0, 6.0);
        assert_eq!(moved.shape(ShapeId(2)).unwrap().kind.bounds().x0, 2.0);

        let painted = doc.apply(Op::Repaint {
            ids: vec![ShapeId(2)],
            paint: Paint {
                filled: true,
                ..Paint::default()
            },
        });
        assert!(painted.shape(ShapeId(2)).unwrap().paint.filled);
        assert!(!painted.shape(ShapeId(1)).unwrap().paint.filled);

        let replaced = doc.apply(Op::Replace {
            id: ShapeId(1),
            kind: Kind::Counter {
                at: Point::ORIGIN,
                number: 1,
            },
        });
        assert!(matches!(
            replaced.shape(ShapeId(1)).unwrap().kind,
            Kind::Counter { .. }
        ));

        let removed = doc.apply(Op::Remove {
            ids: vec![ShapeId(1)],
        });
        assert_eq!(removed.shapes().len(), 1);
        assert!(doc.apply(Op::Clear).is_empty());
    }

    #[test]
    fn raising_puts_marks_on_top_in_order() {
        let doc = Document::new(10, 10, 1.0)
            .apply(line(1.0))
            .apply(line(2.0))
            .apply(line(3.0));
        let raised = doc.apply(Op::Raise {
            ids: vec![ShapeId(2), ShapeId(1)],
        });
        let order: Vec<_> = raised.shapes().map(|shape| shape.id.0).collect();
        assert_eq!(order, [3, 2, 1]);
    }

    #[test]
    fn hits_find_the_topmost_mark() {
        let doc = Document::new(100, 100, 1.0)
            .apply(line(10.0))
            .apply(line(12.0));
        assert_eq!(doc.hit(Point::new(11.0, 5.0), 2.0), Some(ShapeId(2)));
        assert_eq!(doc.hit(Point::new(50.0, 5.0), 2.0), None);
    }

    #[test]
    fn text_is_hit_where_its_letters_are() {
        let doc = Document::new(400, 200, 1.0).apply(Op::Add {
            kind: Kind::Text {
                at: Point::new(10.0, 10.0),
                text: "a fairly long line".into(),
            },
            paint: Paint::default(),
        });
        let text = doc.shape(ShapeId(1)).unwrap();
        let area = doc.bounds(text);
        assert!(area.width() > 150.0 && area.height() > 20.0, "{area:?}");
        assert_eq!(doc.hit(Point::new(150.0, 20.0), 2.0), Some(ShapeId(1)));
        assert_eq!(doc.hit(Point::new(10.0, 60.0), 2.0), None);
        let bigger = doc.apply(Op::Repaint {
            ids: vec![ShapeId(1)],
            paint: Paint {
                text: crate::shape::TextLook {
                    size: 44.0,
                    ..crate::shape::TextLook::default()
                },
                ..Paint::default()
            },
        });
        let grown = bigger.bounds(bigger.shape(ShapeId(1)).unwrap());
        assert!(grown.width() > area.width() * 1.8);
        let sketched = doc.apply(Op::SetStyle(StyleId::Sketch));
        assert_ne!(sketched.bounds(text), area, "measured in the style's face");
    }

    #[test]
    fn counters_count_up() {
        let doc = Document::new(10, 10, 1.0);
        assert_eq!(doc.next_counter(), 1);
        let doc = doc.apply(Op::Add {
            kind: Kind::Counter {
                at: Point::ORIGIN,
                number: 4,
            },
            paint: Paint::default(),
        });
        assert_eq!(doc.next_counter(), 5);
    }

    #[test]
    fn crops_stay_inside_the_image() {
        let doc = Document::new(100, 50, 1.0);
        assert_eq!(doc.visible(), Rect::new(0.0, 0.0, 100.0, 50.0));
        let cropped = doc.apply(Op::Crop(Some(Rect::new(80.0, 40.0, 120.0, -10.0))));
        assert_eq!(cropped.crop, Some(Rect::new(80.0, 0.0, 100.0, 40.0)));
        assert_eq!(cropped.visible(), Rect::new(80.0, 0.0, 100.0, 40.0));
        let outside = doc.apply(Op::Crop(Some(Rect::new(200.0, 200.0, 300.0, 300.0))));
        assert_eq!(outside.crop, None);
        assert_eq!(cropped.apply(Op::Crop(None)).crop, None);
        let styled = doc.apply(Op::SetStyle(StyleId::Sketch));
        assert_eq!(styled.style, StyleId::Sketch);
        assert!(doc.shadow);
        let flat = doc.apply(Op::SetShadow(false));
        assert!(!flat.shadow);
        assert!(
            !flat
                .transformed(kurbo::Affine::IDENTITY, 100, 50, 1.0)
                .shadow
        );
    }

    #[test]
    fn transformed_documents_keep_their_marks() {
        let doc = Document::new(100, 100, 1.0)
            .apply(Op::SetStyle(StyleId::Ink))
            .apply(line(10.0));
        let moved = doc.transformed(
            kurbo::Affine::translate((-5.0, 0.0)).then_scale(2.0),
            40,
            40,
            2.0,
        );
        assert_eq!((moved.width, moved.height), (40, 40));
        assert!((moved.unit - 2.0).abs() < f64::EPSILON);
        assert_eq!(moved.style, StyleId::Ink);
        assert_eq!(moved.next_id(), doc.next_id());
        assert_eq!(
            moved.shape(ShapeId(1)).unwrap().kind.bounds(),
            Rect::new(10.0, 0.0, 10.0, 20.0)
        );
    }

    #[test]
    fn history_undoes_and_redoes() {
        let mut history = History::new(Document::new(10, 10, 1.0));
        assert!(!history.can_undo());
        assert!(!history.undo());
        history.apply(line(1.0));
        history.apply(line(2.0));
        assert_eq!(history.current().shapes().len(), 2);
        assert!(history.undo());
        assert_eq!(history.current().shapes().len(), 1);
        assert!(history.can_redo());
        assert!(history.redo());
        assert_eq!(history.current().shapes().len(), 2);
        assert!(!history.redo());
        history.undo();
        history.apply(line(3.0));
        assert!(!history.can_redo());
    }

    #[test]
    fn edits_that_change_nothing_leave_no_step() {
        let mut history = History::new(Document::new(10, 10, 1.0));
        history.apply(Op::Clear);
        history.apply(Op::Remove {
            ids: vec![ShapeId(9)],
        });
        assert!(!history.can_undo());
    }

    #[test]
    fn a_drag_undoes_in_one_step() {
        let mut history = History::new(Document::new(10, 10, 1.0));
        history.apply(line(1.0));
        history.apply_gradually(line(2.0), false);
        history.apply_gradually(line(3.0), true);
        history.apply_gradually(line(4.0), true);
        assert_eq!(history.current().shapes().len(), 4);
        history.undo();
        assert_eq!(history.current().shapes().len(), 1);
        assert!(!History::new(Document::new(1, 1, 1.0)).apply(Op::Clear));
    }

    #[test]
    fn a_drag_never_joins_a_step_it_did_not_begin() {
        let mut history = History::new(Document::new(10, 10, 1.0));
        history.apply(line(1.0));
        history.apply_gradually(line(2.0), false);
        history.undo();
        history.apply_gradually(line(3.0), true);
        assert!(
            history.undo(),
            "the move after the undo is a step of its own"
        );
        assert_eq!(
            history.current().shapes().len(),
            1,
            "the first line is kept"
        );
        history.apply_gradually(line(5.0), false);
        history.apply(line(6.0));
        history.apply_gradually(line(7.0), true);
        history.undo();
        assert_eq!(history.current().shapes().len(), 3);
    }

    #[test]
    fn history_is_bounded() {
        let mut history = History::new(Document::new(10, 10, 1.0));
        for x in 0..(History::LIMIT + 10) {
            #[allow(clippy::cast_precision_loss)]
            history.apply(line(x as f64));
        }
        let mut steps = 0;
        while history.undo() {
            steps += 1;
        }
        assert_eq!(steps, History::LIMIT);
    }
}
