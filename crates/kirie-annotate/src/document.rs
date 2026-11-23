//! An image and the marks on it. Every edit makes a new document; the marks are shared between
//! versions, so keeping old versions for undo is cheap.

use std::sync::Arc;

use kurbo::{Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::shape::{Kind, Paint, Rgba, Shape, ShapeId};
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
    /// How bright spotlights leave the rest of the image, from black at 0 to untouched at 1.
    #[serde(default = "usual_spotlight_brightness")]
    pub spotlight_brightness: f64,
    /// The part of the image that is kept, if cropped.
    pub crop: Option<Rect>,
    /// What is around the picture when it is saved.
    #[serde(default)]
    pub frame: Frame,
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
    /// Several edits as one step, applied in order.
    Batch(Vec<Op>),
    /// Changes the style. Marks in the document's style painted the first color of a pair in
    /// `recolor` take the second: the old style's shade of a standard color becomes the new one's.
    SetStyle {
        style: StyleId,
        recolor: Vec<(Rgba, Rgba)>,
    },
    SetShadow(bool),
    SetSpotlightBrightness(f64),
    Crop(Option<Rect>),
    SetFrame(Frame),
}

/// What is around the picture when it is saved: room, what fills it, and a shadow.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Frame {
    /// Room around the picture, in image pixels.
    pub margin: f64,
    /// What fills the room around the picture; none leaves it transparent.
    pub background: Option<Rgba>,
    /// The picture casts a soft shadow on the room around it.
    pub shadow: bool,
    /// The saved image grows to take in the marks beyond the picture.
    pub overflow: bool,
}

impl Frame {
    /// The widest margin, in logical pixels.
    pub const MAX_MARGIN: f64 = 200.0;
    /// How far beyond the picture marks are taken in, in logical pixels, so a stray mark far
    /// away cannot make a saved image too large to hold.
    pub const MAX_OVERFLOW: f64 = 500.0;
    /// How soft the picture's shadow is, in logical pixels.
    pub const SHADOW: f64 = 16.0;
}

fn casts_shadows() -> bool {
    true
}

fn usual_spotlight_brightness() -> f64 {
    Document::SPOTLIGHT_BRIGHTNESS
}

impl Document {
    pub const SPOTLIGHT_BRIGHTNESS: f64 = 0.45;
    pub const SPOTLIGHT_BRIGHTNESS_RANGE: std::ops::RangeInclusive<f64> = 0.05..=0.9;

    /// `brightness` kept within [`Self::SPOTLIGHT_BRIGHTNESS_RANGE`].
    pub fn spotlight_brightness(brightness: f64) -> f64 {
        if brightness.is_finite() {
            brightness.clamp(
                *Self::SPOTLIGHT_BRIGHTNESS_RANGE.start(),
                *Self::SPOTLIGHT_BRIGHTNESS_RANGE.end(),
            )
        } else {
            Self::SPOTLIGHT_BRIGHTNESS
        }
    }

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
            spotlight_brightness: Self::SPOTLIGHT_BRIGHTNESS,
            crop: None,
            frame: Frame::default(),
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
    pub fn hit(&self, point: Point, reach: f64) -> Option<ShapeId> {
        self.shapes()
            .rev()
            .find(|shape| self.distance(shape, point) <= reach)
            .map(|shape| shape.id)
    }

    /// The marks lying wholly inside `rect`, as turned, bottom first.
    pub fn within(&self, rect: Rect) -> Vec<ShapeId> {
        let rect = rect.abs();
        self.shapes()
            .filter(|shape| {
                let area = shape.turn().transform_rect_bbox(self.bounds(shape));
                rect.x0 <= area.x0 && area.x1 <= rect.x1 && rect.y0 <= area.y0 && area.y1 <= rect.y1
            })
            .map(|shape| shape.id)
            .collect()
    }

    /// The area `shape` covers here, before stroke width, with text measured in its face.
    pub fn bounds(&self, shape: &Shape) -> Rect {
        crate::style::bounds(shape, self.style, self.unit)
    }

    /// How far `point` is from `shape`, 0 when on or inside it.
    pub fn distance(&self, shape: &Shape, point: Point) -> f64 {
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

    /// What the saved image covers, in whole pixels: the image area kept and, when the frame
    /// takes them in, the marks beyond it, with the frame's margin around, and room for the
    /// picture's shadow.
    pub fn canvas(&self) -> Rect {
        self.canvas_with(&[])
    }

    /// Like [`Self::canvas`], with `drafts`, marks still being drawn, taken in too.
    pub fn canvas_with(&self, drafts: &[Shape]) -> Rect {
        let visible = self.visible();
        let reach = if self.frame.overflow {
            let stage = crate::style::Stage::default();
            let drafted = drafts
                .iter()
                .flat_map(|shape| crate::style::primitives(shape, self.style, self.unit, &stage));
            crate::style::render_list(self)
                .into_iter()
                .chain(drafted)
                .filter_map(|primitive| crate::style::reach(&primitive))
                .fold(visible, |all, drawn| all.union(drawn))
                .intersect(visible.inflate(
                    Frame::MAX_OVERFLOW * self.unit,
                    Frame::MAX_OVERFLOW * self.unit,
                ))
        } else {
            visible
        };
        let margin = self.frame.margin;
        let mut area = reach.inflate(margin, margin);
        if self.frame.shadow {
            let room = 1.5 * self.shadow_radius();
            area = area.union(visible.inflate(room, room) + Vec2::new(0.0, self.shadow_drop()));
        }
        Rect::new(
            area.x0.floor(),
            area.y0.floor(),
            area.x1.ceil(),
            area.y1.ceil(),
        )
    }

    /// How soft the picture's shadow is, in image pixels.
    pub fn shadow_radius(&self) -> f64 {
        Frame::SHADOW * self.unit
    }

    /// How far below the picture its shadow falls, in image pixels.
    pub fn shadow_drop(&self) -> f64 {
        0.4 * self.shadow_radius()
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
        next.spotlight_brightness = self.spotlight_brightness;
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
            Op::Batch(ops) => {
                for op in ops {
                    next = next.apply(op);
                }
            }
            Op::SetStyle { style, recolor } => {
                next.style = style;
                let painted = |shape: &Shape| {
                    recolor
                        .iter()
                        .find(|(from, _)| *from == shape.paint.color)
                        .filter(|_| shape.style.is_none())
                        .map(|(_, to)| *to)
                };
                let ids: Vec<ShapeId> = next
                    .shapes()
                    .filter(|shape| painted(shape).is_some())
                    .map(|shape| shape.id)
                    .collect();
                next.map(&ids, |shape| Shape {
                    paint: Paint {
                        color: painted(shape).unwrap_or(shape.paint.color),
                        ..shape.paint
                    },
                    ..shape.clone()
                });
            }
            Op::SetShadow(shadow) => next.shadow = shadow,
            Op::SetSpotlightBrightness(brightness) => {
                next.spotlight_brightness = Self::spotlight_brightness(brightness);
            }
            Op::Crop(crop) => {
                next.crop = crop
                    .map(|rect| self.clamp(rect))
                    .filter(|r| r.area() >= 1.0);
            }
            Op::SetFrame(frame) => {
                let most = Frame::MAX_MARGIN * self.unit;
                next.frame = Frame {
                    margin: if frame.margin.is_finite() {
                        frame.margin.clamp(0.0, most)
                    } else {
                        0.0
                    },
                    ..frame
                };
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

    use crate::shape::PALETTE;

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
    fn a_batch_is_one_step() {
        let mut history = History::new(Document::new(100, 100, 1.0).apply(line(1.0)));
        history.apply(Op::Batch(vec![
            line(2.0),
            Op::Move {
                ids: vec![ShapeId(1)],
                by: Vec2::new(3.0, 0.0),
            },
        ]));
        assert_eq!(history.current().shapes().len(), 2);
        assert_eq!(
            history
                .current()
                .shape(ShapeId(1))
                .unwrap()
                .kind
                .bounds()
                .x0,
            4.0
        );
        history.undo();
        assert_eq!(history.current().shapes().len(), 1);
        assert!(!history.apply(Op::Batch(Vec::new())));
    }

    #[test]
    fn within_finds_marks_wholly_inside() {
        let doc = Document::new(100, 100, 1.0)
            .apply(line(10.0))
            .apply(line(50.0));
        assert_eq!(doc.within(Rect::new(0.0, -1.0, 20.0, 20.0)), [ShapeId(1)]);
        assert_eq!(
            doc.within(Rect::new(60.0, 20.0, 0.0, -1.0)),
            [ShapeId(1), ShapeId(2)]
        );
        assert!(
            doc.within(Rect::new(0.0, 5.0, 60.0, 20.0)).is_empty(),
            "lines cut in half"
        );
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
        let sketched = doc.apply(Op::SetStyle {
            style: StyleId::Sketch,
            recolor: Vec::new(),
        });
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
    fn a_new_style_reshades_standard_colors_and_keeps_the_rest() {
        let red = (PALETTE[0], StyleId::Ink.palette()[0]);
        let teal = Rgba::rgb(0x14, 0xb8, 0xa6);
        let painted = |color| Op::Add {
            kind: Kind::Rectangle {
                rect: Rect::new(0.0, 0.0, 5.0, 5.0),
            },
            paint: Paint {
                color,
                ..Paint::default()
            },
        };
        let doc = Document::new(10, 10, 1.0)
            .apply(painted(red.0))
            .apply(painted(teal))
            .apply(painted(red.0));
        let own = doc.shapes().last().unwrap().id;
        let mut doc = doc;
        Arc::make_mut(&mut doc.shapes[2]).style = Some(StyleId::Clean);
        let inked = doc.apply(Op::SetStyle {
            style: StyleId::Ink,
            recolor: vec![red],
        });
        assert_eq!(inked.style, StyleId::Ink);
        let colors: Vec<Rgba> = inked.shapes().map(|shape| shape.paint.color).collect();
        assert_eq!(colors, [red.1, teal, red.0]);
        assert_eq!(inked.shape(own).unwrap().style, Some(StyleId::Clean));
    }

    #[test]
    fn the_canvas_takes_in_the_margin_and_marks_beyond_the_picture() {
        let doc = Document::new(100, 50, 1.0).apply(Op::Add {
            kind: Kind::Line {
                from: kurbo::Point::new(50.0, 25.0),
                to: kurbo::Point::new(150.0, 25.0),
            },
            paint: Paint::default(),
        });
        assert_eq!(doc.canvas(), Rect::new(0.0, 0.0, 100.0, 50.0));
        let framed = doc.apply(Op::SetFrame(Frame {
            margin: 10.4,
            ..Frame::default()
        }));
        assert_eq!(framed.canvas(), Rect::new(-11.0, -11.0, 111.0, 61.0));
        let taken_in = doc.apply(Op::SetFrame(Frame {
            overflow: true,
            ..Frame::default()
        }));
        let canvas = taken_in.canvas();
        assert!(canvas.x1 >= 150.0, "{canvas:?}");
        assert!(canvas.x0.abs() < f64::EPSILON && (canvas.y1 - 50.0).abs() < f64::EPSILON);
        let far = doc
            .apply(Op::Add {
                kind: Kind::Line {
                    from: kurbo::Point::new(50.0, 25.0),
                    to: kurbo::Point::new(30000.0, 20000.0),
                },
                paint: Paint::default(),
            })
            .apply(Op::SetFrame(Frame {
                overflow: true,
                ..Frame::default()
            }));
        assert_eq!(far.canvas(), Rect::new(0.0, 0.0, 600.0, 550.0));
        let huge = doc.apply(Op::SetFrame(Frame {
            margin: f64::INFINITY,
            ..Frame::default()
        }));
        assert!(huge.frame.margin.abs() < f64::EPSILON);
        let shaded = doc.apply(Op::SetFrame(Frame {
            shadow: true,
            ..Frame::default()
        }));
        assert_eq!(shaded.canvas(), Rect::new(-24.0, -18.0, 124.0, 81.0));
        let drafted = Shape {
            id: ShapeId(99),
            kind: Kind::Line {
                from: kurbo::Point::new(50.0, 25.0),
                to: kurbo::Point::new(50.0, 90.0),
            },
            paint: Paint::default(),
            style: None,
            rotation: 0.0,
        };
        assert!(taken_in.canvas_with(std::slice::from_ref(&drafted)).y1 >= 90.0);
        assert!((doc.canvas_with(&[drafted]).y1 - 50.0).abs() < f64::EPSILON);
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
        let styled = doc.apply(Op::SetStyle {
            style: StyleId::Sketch,
            recolor: Vec::new(),
        });
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
    fn spotlight_brightness_stays_within_range() {
        let doc = Document::new(10, 10, 1.0);
        let dark = doc.apply(Op::SetSpotlightBrightness(-1.0));
        assert!((dark.spotlight_brightness - 0.05).abs() < 1e-9);
        let bright = doc.apply(Op::SetSpotlightBrightness(0.7));
        assert!((bright.spotlight_brightness - 0.7).abs() < 1e-9);
        let kept = bright.transformed(kurbo::Affine::IDENTITY, 10, 10, 1.0);
        assert!((kept.spotlight_brightness - 0.7).abs() < 1e-9);
        let broken = doc.apply(Op::SetSpotlightBrightness(f64::NAN));
        assert!((broken.spotlight_brightness - Document::SPOTLIGHT_BRIGHTNESS).abs() < 1e-9);
    }

    #[test]
    fn transformed_documents_keep_their_marks() {
        let doc = Document::new(100, 100, 1.0)
            .apply(Op::SetStyle {
                style: StyleId::Ink,
                recolor: Vec::new(),
            })
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
