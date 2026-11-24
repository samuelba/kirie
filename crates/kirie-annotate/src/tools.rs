//! What each tool does with a press, a drag and a release. Tools only describe marks; the
//! overlay and the editor turn finished drafts into document edits.

use kurbo::{Point, Rect, Vec2};

use crate::document::{Document, Op};
use crate::shape::{Kind, Paint, Shape, ShapeId};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Tool {
    /// Picks marks and moves them.
    Select,
    #[default]
    Arrow,
    Line,
    Rectangle,
    Ellipse,
    Pen,
    Highlighter,
    Text,
    /// Words in a speech bubble pointing at something.
    Bubble,
    Counter,
    /// An emoji placed like a sticker.
    Sticker,
    Blur,
    Pixelate,
    Redact,
    Spotlight,
}

impl Tool {
    pub const ALL: [Tool; 15] = [
        Tool::Select,
        Tool::Arrow,
        Tool::Line,
        Tool::Rectangle,
        Tool::Ellipse,
        Tool::Pen,
        Tool::Highlighter,
        Tool::Text,
        Tool::Bubble,
        Tool::Counter,
        Tool::Sticker,
        Tool::Blur,
        Tool::Pixelate,
        Tool::Redact,
        Tool::Spotlight,
    ];

    pub const fn name(self) -> &'static str {
        match self {
            Tool::Select => "Select",
            Tool::Arrow => "Arrow",
            Tool::Line => "Line",
            Tool::Rectangle => "Rectangle",
            Tool::Ellipse => "Ellipse",
            Tool::Pen => "Pen",
            Tool::Highlighter => "Highlighter",
            Tool::Text => "Text",
            Tool::Bubble => "Speech bubble",
            Tool::Counter => "Step number",
            Tool::Sticker => "Sticker",
            Tool::Blur => "Blur",
            Tool::Pixelate => "Pixelate",
            Tool::Redact => "Redact",
            Tool::Spotlight => "Spotlight",
        }
    }

    /// The name with its key, for tooltips.
    pub const fn label(self) -> &'static str {
        match self {
            Tool::Select => "Select (V)",
            Tool::Arrow => "Arrow (A)",
            Tool::Line => "Line (L)",
            Tool::Rectangle => "Rectangle (R)",
            Tool::Ellipse => "Ellipse (E)",
            Tool::Pen => "Pen (P)",
            Tool::Highlighter => "Highlighter (H)",
            Tool::Text => "Text (T)",
            Tool::Bubble => "Speech bubble (C)",
            Tool::Counter => "Step number (N)",
            Tool::Sticker => "Sticker (K)",
            Tool::Blur => "Blur (B)",
            Tool::Pixelate => "Pixelate (X)",
            Tool::Redact => "Redact (D)",
            Tool::Spotlight => "Spotlight (S)",
        }
    }

    /// The key that picks the tool.
    pub const fn key(self) -> char {
        match self {
            Tool::Select => 'v',
            Tool::Arrow => 'a',
            Tool::Line => 'l',
            Tool::Rectangle => 'r',
            Tool::Ellipse => 'e',
            Tool::Pen => 'p',
            Tool::Highlighter => 'h',
            Tool::Text => 't',
            Tool::Bubble => 'c',
            Tool::Counter => 'n',
            Tool::Sticker => 'k',
            Tool::Blur => 'b',
            Tool::Pixelate => 'x',
            Tool::Redact => 'd',
            Tool::Spotlight => 's',
        }
    }

    pub fn from_key(key: char) -> Option<Tool> {
        Tool::ALL
            .into_iter()
            .find(|tool| tool.key() == key.to_ascii_lowercase())
    }
}

/// A mark being drawn, or marks being moved.
#[derive(Debug, Clone, PartialEq)]
pub enum Draft {
    Span {
        tool: Tool,
        start: Point,
        end: Point,
    },
    Stroke {
        tool: Tool,
        points: Vec<Point>,
    },
    Moving {
        ids: Vec<ShapeId>,
        /// The mark pressed on, which a click without a move picks alone.
        held: ShapeId,
        start: Point,
        by: Vec2,
    },
    /// A box dragged over empty space, picking the marks inside it.
    Marquee {
        start: Point,
        end: Point,
        /// Adds to the picked marks, as Shift held at the press asked.
        extend: bool,
    },
    /// An emoji being placed: a square dragged from `start`, or one of `size` centered on a
    /// click.
    Sticker {
        emoji: String,
        start: Point,
        end: Point,
        size: f64,
    },
    /// A grip of a picked mark being dragged.
    Editing {
        grip: crate::edit::Grip,
        origin: Box<Shape>,
        current: Box<Shape>,
    },
}

/// What a press with a tool starts.
#[derive(Debug, Clone, PartialEq)]
pub enum Press {
    Draft(Draft),
    /// A mark made by the press alone.
    Done(Op),
    /// Ask for text to place here.
    TypeAt(Point),
    /// Place the chosen emoji here, with [`Draft::sticker`].
    PlaceSticker(Point),
    /// Adds the mark to the picked ones, or takes it out.
    Toggle(ShapeId),
    Nothing,
}

/// Where a clicked speech bubble's words go from the tip of its tail.
const BUBBLE_OFFSET: Vec2 = Vec2::new(48.0, -56.0);

/// How wide a sticker placed with a click is, in logical pixels.
pub const STICKER_SIZE: f64 = 64.0;

/// Drags shorter than this make no mark.
const MIN_DRAG: f64 = 3.0;
/// Freehand points closer than this are dropped.
const MIN_STEP: f64 = 1.5;

/// What a press with `tool` at `at` starts. `reach` is how near a mark or grip must be, and
/// `picked` the marks picked, a lone one showing its grips. `extend` (Shift) adds a mark to
/// them or takes it out, instead of picking it alone.
pub fn press(
    tool: Tool,
    at: Point,
    document: &Document,
    paint: Paint,
    reach: f64,
    picked: &[ShapeId],
    extend: bool,
) -> Press {
    if tool == Tool::Select
        && let [id] = picked
        && let Some(shape) = document.shape(*id)
        && let Some(grip) = crate::edit::grip_at(shape, at, reach, reach / 6.0)
    {
        return Press::Draft(Draft::Editing {
            grip,
            origin: Box::new(shape.clone()),
            current: Box::new(shape.clone()),
        });
    }
    match tool {
        Tool::Select => match document.hit(at, reach) {
            Some(id) if extend => Press::Toggle(id),
            Some(id) => {
                let ids = if picked.contains(&id) {
                    picked.to_vec()
                } else {
                    vec![id]
                };
                Press::Draft(Draft::Moving {
                    ids,
                    held: id,
                    start: at,
                    by: Vec2::ZERO,
                })
            }
            None => Press::Draft(Draft::Marquee {
                start: at,
                end: at,
                extend,
            }),
        },
        Tool::Text => Press::TypeAt(at),
        Tool::Sticker => Press::PlaceSticker(at),
        Tool::Counter => Press::Done(Op::Add {
            kind: Kind::Counter {
                at,
                number: document.next_counter(),
            },
            paint,
        }),
        Tool::Pen | Tool::Highlighter => Press::Draft(Draft::Stroke {
            tool,
            points: vec![at],
        }),
        _ => Press::Draft(Draft::Span {
            tool,
            start: at,
            end: at,
        }),
    }
}

impl Draft {
    /// Starts placing `emoji` at `at`; a click without a drag makes it `size` across.
    pub fn sticker(emoji: impl Into<String>, at: Point, size: f64) -> Draft {
        Draft::Sticker {
            emoji: emoji.into(),
            start: at,
            end: at,
            size,
        }
    }

    /// Follows the pointer. `constrain` (Shift) snaps lines to 45° and boxes to squares, and
    /// straightens a highlighter.
    #[must_use]
    pub fn drag(self, at: Point, constrain: bool) -> Draft {
        match self {
            Draft::Span { tool, start, .. } => {
                let end = if constrain {
                    constrained(tool, start, at)
                } else {
                    at
                };
                Draft::Span { tool, start, end }
            }
            Draft::Stroke { tool, mut points } => {
                if constrain && tool == Tool::Highlighter {
                    let first = points[0];
                    let end = snap_45(first, at);
                    points = vec![first, end];
                } else if points
                    .last()
                    .is_none_or(|last| last.distance(at) >= MIN_STEP)
                {
                    points.push(at);
                }
                Draft::Stroke { tool, points }
            }
            Draft::Moving {
                ids, held, start, ..
            } => Draft::Moving {
                ids,
                held,
                start,
                by: at - start,
            },
            Draft::Marquee { start, extend, .. } => Draft::Marquee {
                start,
                end: at,
                extend,
            },
            Draft::Sticker {
                emoji, start, size, ..
            } => Draft::Sticker {
                emoji,
                start,
                end: constrained(Tool::Sticker, start, at),
                size,
            },
            Draft::Editing { grip, origin, .. } => {
                let current = Box::new(crate::edit::drag(&origin, grip, at, constrain));
                Draft::Editing {
                    grip,
                    origin,
                    current,
                }
            }
        }
    }

    /// The picked mark as an edit has it now, shown in place of the mark.
    pub fn edited(&self) -> Option<&Shape> {
        match self {
            Draft::Editing { current, .. } => Some(current),
            _ => None,
        }
    }

    /// The mark as it would be if released now, to show while drawing.
    pub fn preview(&self) -> Option<Kind> {
        match self {
            Draft::Span { tool, start, end } => span_kind(*tool, *start, *end),
            Draft::Stroke { tool, points } => match tool {
                Tool::Highlighter => Some(Kind::Highlighter {
                    points: points.clone(),
                }),
                _ => Some(Kind::Pen {
                    points: points.clone(),
                }),
            },
            Draft::Sticker {
                emoji,
                start,
                end,
                size,
            } => Some(Kind::Sticker {
                rect: if start.distance(*end) < MIN_DRAG {
                    Rect::from_center_size(*start, (*size, *size))
                } else {
                    Rect::from_points(*start, *end)
                },
                emoji: emoji.clone(),
            }),
            Draft::Moving { .. } | Draft::Editing { .. } | Draft::Marquee { .. } => None,
        }
    }

    /// The box of a marquee being dragged.
    pub fn marquee(&self) -> Option<Rect> {
        match self {
            Draft::Marquee { start, end, .. } => Some(Rect::from_points(*start, *end)),
            _ => None,
        }
    }

    /// The edit the draft makes when released, if it made anything.
    pub fn finish(self, paint: Paint) -> Option<Op> {
        match self {
            Draft::Span { start, end, .. } if start.distance(end) < MIN_DRAG => None,
            Draft::Marquee { .. }
            | Draft::Span {
                tool: Tool::Bubble, ..
            } => None,
            Draft::Moving { by, .. } if by.hypot() < f64::EPSILON => None,
            Draft::Moving { ids, by, .. } => Some(Op::Move { ids, by }),
            Draft::Editing {
                origin, current, ..
            } if origin == current => None,
            Draft::Editing { current, .. } => Some(Op::Reshape {
                id: current.id,
                kind: current.kind,
                rotation: current.rotation,
            }),
            draft => draft.preview().map(|kind| Op::Add { kind, paint }),
        }
    }

    /// Where the words of a speech bubble drawn by this draft go, and the tip of its tail: the
    /// press is the tip and the release the middle of the words. A click without a drag puts
    /// the words up and to the right of the tip.
    pub fn bubble(&self) -> Option<(Point, Point)> {
        match *self {
            Draft::Span {
                tool: Tool::Bubble,
                start,
                end,
            } if start.distance(end) < MIN_DRAG => Some((start + BUBBLE_OFFSET, start)),
            Draft::Span {
                tool: Tool::Bubble,
                start,
                end,
            } => Some((end, start)),
            _ => None,
        }
    }
}

fn span_kind(tool: Tool, start: Point, end: Point) -> Option<Kind> {
    let rect = Rect::from_points(start, end);
    Some(match tool {
        Tool::Arrow => Kind::Arrow {
            from: start,
            to: end,
        },
        Tool::Line => Kind::Line {
            from: start,
            to: end,
        },
        Tool::Rectangle => Kind::Rectangle { rect },
        Tool::Ellipse => Kind::Ellipse { rect },
        Tool::Blur => Kind::Blur { rect },
        Tool::Pixelate => Kind::Pixelate { rect },
        Tool::Redact => Kind::Redact { rect },
        Tool::Spotlight => Kind::Spotlight { rect },
        Tool::Bubble => Kind::Bubble {
            center: end,
            tail: start,
            text: "|".into(),
        },
        Tool::Select
        | Tool::Pen
        | Tool::Highlighter
        | Tool::Text
        | Tool::Counter
        | Tool::Sticker => {
            return None;
        }
    })
}

fn constrained(tool: Tool, start: Point, at: Point) -> Point {
    match tool {
        Tool::Arrow | Tool::Line => snap_45(start, at),
        _ => {
            let delta = at - start;
            let side = delta.x.abs().max(delta.y.abs());
            start + Vec2::new(side.copysign(delta.x), side.copysign(delta.y))
        }
    }
}

/// `at`, moved onto the nearest multiple of 45° around `start`.
fn snap_45(start: Point, at: Point) -> Point {
    let delta = at - start;
    let length = delta.hypot();
    let step = std::f64::consts::FRAC_PI_4;
    let angle = (delta.atan2() / step).round() * step;
    start + Vec2::from_angle(angle) * length
}

#[cfg(test)]
mod tests {
    use super::*;

    fn doc() -> Document {
        Document::new(200, 200, 1.0)
    }

    fn drawn(tool: Tool, from: Point, to: Point) -> Option<Op> {
        match press(tool, from, &doc(), Paint::default(), 4.0, &[], false) {
            Press::Draft(draft) => draft.drag(to, false).finish(Paint::default()),
            other => panic!("{tool:?} started {other:?}"),
        }
    }

    #[test]
    fn keys_pick_tools() {
        for tool in Tool::ALL {
            assert_eq!(Tool::from_key(tool.key()), Some(tool));
            assert_eq!(Tool::from_key(tool.key().to_ascii_uppercase()), Some(tool));
            assert!(!tool.name().is_empty());
            let key = tool.key().to_ascii_uppercase();
            assert!(tool.label().starts_with(tool.name()));
            assert!(tool.label().ends_with(&format!("({key})")));
        }
        assert_eq!(Tool::from_key('z'), None);
    }

    #[test]
    fn stickers_are_squares_or_clicks() {
        let at = Point::new(50.0, 50.0);
        assert_eq!(
            press(Tool::Sticker, at, &doc(), Paint::default(), 4.0, &[], false),
            Press::PlaceSticker(at)
        );
        let clicked = Draft::sticker("\u{1f44d}", at, 64.0).drag(Point::new(51.0, 50.0), false);
        let Some(Op::Add {
            kind: Kind::Sticker { rect, emoji },
            ..
        }) = clicked.finish(Paint::default())
        else {
            panic!()
        };
        assert_eq!(rect, Rect::from_center_size(at, (64.0, 64.0)));
        assert_eq!(emoji, "\u{1f44d}");
        let dragged = Draft::sticker("x", at, 64.0).drag(Point::new(70.0, 10.0), false);
        assert_eq!(
            dragged.preview().map(|kind| kind.bounds()),
            Some(Rect::new(50.0, 10.0, 90.0, 50.0))
        );
    }

    #[test]
    fn spans_make_their_kind() {
        let (a, b) = (Point::new(10.0, 10.0), Point::new(60.0, 40.0));
        for tool in [
            Tool::Arrow,
            Tool::Line,
            Tool::Rectangle,
            Tool::Ellipse,
            Tool::Blur,
            Tool::Pixelate,
            Tool::Redact,
            Tool::Spotlight,
        ] {
            let Some(Op::Add { kind, .. }) = drawn(tool, a, b) else {
                panic!("{tool:?} made nothing");
            };
            assert_eq!(kind.bounds(), Rect::from_points(a, b), "{tool:?}");
        }
    }

    #[test]
    fn tiny_drags_make_nothing() {
        assert_eq!(
            drawn(Tool::Rectangle, Point::ORIGIN, Point::new(1.0, 1.0)),
            None
        );
    }

    #[test]
    fn shift_snaps() {
        let draft = Draft::Span {
            tool: Tool::Line,
            start: Point::ORIGIN,
            end: Point::ORIGIN,
        }
        .drag(Point::new(100.0, 10.0), true);
        let Some(Kind::Line { to, .. }) = draft.preview() else {
            panic!()
        };
        assert!(to.y.abs() < 1e-9 && (to.x - 100.5).abs() < 0.1);

        let square = Draft::Span {
            tool: Tool::Rectangle,
            start: Point::ORIGIN,
            end: Point::ORIGIN,
        }
        .drag(Point::new(-30.0, 10.0), true);
        let Some(Kind::Rectangle { rect }) = square.preview() else {
            panic!()
        };
        assert_eq!(rect, Rect::new(-30.0, 0.0, 0.0, 30.0));
    }

    #[test]
    fn strokes_collect_points_and_highlighters_straighten() {
        let mut draft = match press(
            Tool::Pen,
            Point::ORIGIN,
            &doc(),
            Paint::default(),
            4.0,
            &[],
            false,
        ) {
            Press::Draft(draft) => draft,
            other => panic!("{other:?}"),
        };
        for x in [0.5, 5.0, 10.0] {
            draft = draft.drag(Point::new(x, 0.0), false);
        }
        let Some(Kind::Pen { points }) = draft.preview() else {
            panic!()
        };
        assert_eq!(points.len(), 3, "the too-close point is dropped");

        let straight = Draft::Stroke {
            tool: Tool::Highlighter,
            points: vec![Point::ORIGIN, Point::new(3.0, 3.0)],
        }
        .drag(Point::new(50.0, 4.0), true);
        let Some(Kind::Highlighter { points }) = straight.preview() else {
            panic!()
        };
        assert_eq!(points.len(), 2);
        assert!(points[1].y.abs() < 1e-9);
        assert!(straight.finish(Paint::default()).is_some());
    }

    #[test]
    fn counters_number_themselves() {
        let document = doc().apply(Op::Add {
            kind: Kind::Counter {
                at: Point::ORIGIN,
                number: 1,
            },
            paint: Paint::default(),
        });
        let Press::Done(Op::Add {
            kind: Kind::Counter { number, .. },
            ..
        }) = press(
            Tool::Counter,
            Point::new(5.0, 5.0),
            &document,
            Paint::default(),
            4.0,
            &[],
            false,
        )
        else {
            panic!()
        };
        assert_eq!(number, 2);
    }

    #[test]
    fn text_asks_for_words() {
        let at = Point::new(3.0, 4.0);
        assert_eq!(
            press(Tool::Text, at, &doc(), Paint::default(), 4.0, &[], false),
            Press::TypeAt(at)
        );
    }

    #[test]
    fn select_moves_the_mark_under_the_pointer() {
        let document = doc().apply(Op::Add {
            kind: Kind::Rectangle {
                rect: Rect::new(10.0, 10.0, 50.0, 50.0),
            },
            paint: Paint::default(),
        });
        let paint = Paint::default();
        let empty = Point::new(150.0, 150.0);
        assert_eq!(
            press(Tool::Select, empty, &document, paint, 4.0, &[], false),
            Press::Draft(Draft::Marquee {
                start: empty,
                end: empty,
                extend: false,
            })
        );
        let Press::Draft(draft) = press(
            Tool::Select,
            Point::new(20.0, 20.0),
            &document,
            paint,
            4.0,
            &[],
            false,
        ) else {
            panic!()
        };
        assert_eq!(draft.preview(), None);
        let still = draft.clone().drag(Point::new(20.0, 20.0), false);
        assert_eq!(still.finish(paint), None);
        let op = draft.drag(Point::new(30.0, 25.0), false).finish(paint);
        assert_eq!(
            op,
            Some(Op::Move {
                ids: vec![ShapeId(1)],
                by: Vec2::new(10.0, 5.0)
            })
        );
    }

    #[test]
    fn grips_of_the_picked_mark_reshape_it() {
        let document = doc().apply(Op::Add {
            kind: Kind::Rectangle {
                rect: Rect::new(10.0, 10.0, 50.0, 50.0),
            },
            paint: Paint::default(),
        });
        let paint = Paint::default();
        let Press::Draft(draft) = press(
            Tool::Select,
            Point::new(50.0, 50.0),
            &document,
            paint,
            4.0,
            &[ShapeId(1)],
            false,
        ) else {
            panic!()
        };
        let draft = draft.drag(Point::new(80.0, 70.0), false);
        assert_eq!(
            draft.edited().map(|shape| shape.kind.bounds()),
            Some(Rect::new(10.0, 10.0, 80.0, 70.0))
        );
        let Some(Op::Reshape { kind, rotation, .. }) = draft.finish(paint) else {
            panic!()
        };
        assert_eq!(kind.bounds(), Rect::new(10.0, 10.0, 80.0, 70.0));
        assert!(rotation.abs() < f64::EPSILON);
        let Press::Draft(untouched) = press(
            Tool::Select,
            Point::new(10.0, 10.0),
            &document,
            paint,
            4.0,
            &[ShapeId(1)],
            false,
        ) else {
            panic!()
        };
        assert_eq!(untouched.finish(paint), None);
    }

    #[test]
    fn picked_marks_move_together_and_shift_toggles() {
        let document = doc()
            .apply(Op::Add {
                kind: Kind::Rectangle {
                    rect: Rect::new(10.0, 10.0, 50.0, 50.0),
                },
                paint: Paint::default(),
            })
            .apply(Op::Add {
                kind: Kind::Rectangle {
                    rect: Rect::new(100.0, 100.0, 140.0, 140.0),
                },
                paint: Paint::default(),
            });
        let paint = Paint::default();
        let both = [ShapeId(1), ShapeId(2)];
        let on_first = Point::new(30.0, 10.0);
        let Press::Draft(Draft::Moving { ids, .. }) =
            press(Tool::Select, on_first, &document, paint, 4.0, &both, false)
        else {
            panic!()
        };
        assert_eq!(ids, both);
        let Press::Draft(Draft::Moving { ids, .. }) = press(
            Tool::Select,
            on_first,
            &document,
            paint,
            4.0,
            &[ShapeId(2)],
            false,
        ) else {
            panic!()
        };
        assert_eq!(ids, [ShapeId(1)], "an unpicked mark is picked alone");
        assert_eq!(
            press(Tool::Select, on_first, &document, paint, 4.0, &both, true),
            Press::Toggle(ShapeId(1))
        );
        let corner = Point::new(50.0, 50.0);
        assert!(
            matches!(
                press(Tool::Select, corner, &document, paint, 4.0, &both, false),
                Press::Draft(Draft::Moving { .. })
            ),
            "grips show on a lone pick only"
        );
        assert!(
            matches!(
                press(
                    Tool::Select,
                    corner,
                    &document,
                    paint,
                    4.0,
                    &[ShapeId(1)],
                    true
                ),
                Press::Draft(Draft::Editing { .. })
            ),
            "Shift held on a grip constrains the resize"
        );
        assert!(matches!(
            press(
                Tool::Select,
                Point::ORIGIN,
                &document,
                paint,
                4.0,
                &both,
                true
            ),
            Press::Draft(Draft::Marquee { extend: true, .. })
        ));
    }

    #[test]
    fn a_marquee_spans_its_drag_and_makes_no_edit() {
        let draft = Draft::Marquee {
            start: Point::new(40.0, 40.0),
            end: Point::new(40.0, 40.0),
            extend: false,
        }
        .drag(Point::new(10.0, 70.0), true);
        assert_eq!(draft.marquee(), Some(Rect::new(10.0, 40.0, 40.0, 70.0)));
        assert_eq!(draft.preview(), None);
        assert_eq!(draft.finish(Paint::default()), None);
    }

    #[test]
    fn bubbles_point_from_the_press_and_wait_for_words() {
        let tip = Point::new(20.0, 80.0);
        let Press::Draft(draft) =
            press(Tool::Bubble, tip, &doc(), Paint::default(), 4.0, &[], false)
        else {
            panic!("a bubble starts a draft");
        };
        assert_eq!(draft.bubble(), Some((tip + BUBBLE_OFFSET, tip)));
        let dragged = draft.drag(Point::new(120.0, 30.0), false);
        assert_eq!(dragged.bubble(), Some((Point::new(120.0, 30.0), tip)));
        assert!(matches!(
            dragged.preview(),
            Some(Kind::Bubble { center, tail, .. }) if center == Point::new(120.0, 30.0) && tail == tip
        ));
        assert_eq!(dragged.finish(Paint::default()), None);
        let other = press(Tool::Arrow, tip, &doc(), Paint::default(), 4.0, &[], false);
        assert!(matches!(other, Press::Draft(draft) if draft.bubble().is_none()));
    }

    #[test]
    fn non_span_tools_have_no_span_kind() {
        for tool in [
            Tool::Select,
            Tool::Pen,
            Tool::Highlighter,
            Tool::Text,
            Tool::Counter,
        ] {
            assert_eq!(span_kind(tool, Point::ORIGIN, Point::new(9.0, 9.0)), None);
        }
    }
}
