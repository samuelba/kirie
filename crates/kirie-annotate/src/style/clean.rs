//! Clean: crisp vectors with round ends and, unless turned off, a soft shadow that lifts them
//! off the screenshot.

use kurbo::{BezPath, Point, RoundedRect, Shape as _, Vec2};

use super::{Context, arrow_wings, smooth, text_on};
use crate::primitive::{Align, FillRule, Font, Primitive};
use crate::shape::{Kind, Rgba};

/// What redactions are filled with.
pub(super) const REDACT: Rgba = Rgba::rgb(0x16, 0x18, 0x1d);

const SHADOW: Rgba = Rgba {
    r: 0,
    g: 0,
    b: 0,
    a: 0x40,
};

pub(super) fn primitives(kind: &Kind, context: &Context) -> Vec<Primitive> {
    let width = context.width();
    let shadow_offset = Vec2::new(0.0, 1.5 * context.unit);
    let stroke = |path: BezPath| {
        let shadow = context.shadow.then(|| Primitive::Stroke {
            path: translated(&path, shadow_offset),
            color: SHADOW,
            width: width + context.unit,
        });
        shadow
            .into_iter()
            .chain([Primitive::Stroke {
                path,
                color: context.color(),
                width,
            }])
            .collect::<Vec<_>>()
    };
    match kind {
        Kind::Line { from, to } => stroke(line(*from, *to)),
        Kind::Arrow { from, to } => arrow(*from, *to, context),
        Kind::Rectangle { rect } => {
            let path =
                RoundedRect::from_rect(rect.abs(), 2.0 * context.unit + width / 2.0).to_path(0.1);
            filled(&path, context)
                .into_iter()
                .chain(stroke(path))
                .collect()
        }
        Kind::Ellipse { rect } => {
            let path = kurbo::Ellipse::from_rect(rect.abs()).to_path(0.1);
            filled(&path, context)
                .into_iter()
                .chain(stroke(path))
                .collect()
        }
        Kind::Pen { points } => stroke(smooth(points)),
        Kind::Highlighter { points } => vec![super::marker(points, context)],
        Kind::Counter { at, number } => counter(*at, *number, context),
        Kind::Redact { rect } => vec![Primitive::Fill {
            path: RoundedRect::from_rect(rect.abs(), 2.0 * context.unit).to_path(0.1),
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

fn line(from: Point, to: Point) -> BezPath {
    let mut path = BezPath::new();
    path.move_to(from);
    path.line_to(to);
    path
}

fn translated(path: &BezPath, by: Vec2) -> BezPath {
    let mut moved = path.clone();
    moved.apply_affine(kurbo::Affine::translate(by));
    moved
}

fn filled(path: &BezPath, context: &Context) -> Option<Primitive> {
    context.paint.filled.then(|| Primitive::Fill {
        path: path.clone(),
        color: context.tint(),
        rule: FillRule::NonZero,
    })
}

fn arrow(from: Point, to: Point, context: &Context) -> Vec<Primitive> {
    let width = context.width();
    let head = (width * 3.5).max(12.0 * context.unit);
    let Some((left, right)) = arrow_wings(from, to, head, 0.45) else {
        return vec![Primitive::Fill {
            path: kurbo::Circle::new(to, width / 2.0).to_path(0.1),
            color: context.color(),
            rule: FillRule::NonZero,
        }];
    };
    // The shaft stops inside the head, so its round end does not poke out of the tip.
    let length = (to - from).hypot();
    let shaft_end = from + (to - from) * ((length - head * 0.6).max(0.0) / length);
    let mut head_path = BezPath::new();
    head_path.move_to(to);
    head_path.line_to(left);
    head_path.line_to(right);
    head_path.close_path();
    let shadow = Vec2::new(0.0, 1.5 * context.unit);
    let mut drawn = Vec::with_capacity(5);
    if context.shadow {
        drawn.extend([
            Primitive::Stroke {
                path: translated(&line(from, shaft_end), shadow),
                color: SHADOW,
                width: width + context.unit,
            },
            Primitive::Fill {
                path: translated(&head_path, shadow),
                color: SHADOW,
                rule: FillRule::NonZero,
            },
        ]);
    }
    drawn.extend([
        Primitive::Stroke {
            path: line(from, shaft_end),
            color: context.color(),
            width,
        },
        Primitive::Fill {
            path: head_path.clone(),
            color: context.color(),
            rule: FillRule::NonZero,
        },
        // Rounds the head's corners.
        Primitive::Stroke {
            path: head_path,
            color: context.color(),
            width: width * 0.6,
        },
    ]);
    drawn
}

fn counter(at: Point, number: u32, context: &Context) -> Vec<Primitive> {
    let radius = context.counter_radius();
    let circle = kurbo::Circle::new(at, radius).to_path(0.1);
    let shadow = kurbo::Circle::new(
        at + Vec2::new(0.0, 1.5 * context.unit),
        radius + context.unit,
    )
    .to_path(0.1);
    let shadow = context.shadow.then_some(Primitive::Fill {
        path: shadow,
        color: SHADOW,
        rule: FillRule::NonZero,
    });
    shadow
        .into_iter()
        .chain([
            Primitive::Fill {
                path: circle,
                color: context.color(),
                rule: FillRule::NonZero,
            },
            Primitive::Text {
                at,
                text: number.to_string(),
                size: radius * 1.15,
                color: text_on(context.color()),
                font: Font::Sans,
                align: Align::Left,
                halo: None,
                centered: true,
            },
        ])
        .collect()
}

/// A speech bubble: the filled outline over its shadow.
pub(super) fn bubble(outline: BezPath, context: &Context) -> Vec<Primitive> {
    let shadow = context.shadow.then(|| Primitive::Fill {
        path: translated(&outline, Vec2::new(0.0, 1.5 * context.unit)),
        color: SHADOW,
        rule: FillRule::NonZero,
    });
    shadow
        .into_iter()
        .chain([Primitive::Fill {
            path: outline,
            color: context.color(),
            rule: FillRule::NonZero,
        }])
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::{Paint, Weight};
    use kurbo::Rect;

    fn bounds(primitives: &[Primitive]) -> Rect {
        primitives
            .iter()
            .filter_map(|primitive| match primitive {
                Primitive::Fill { path, .. } | Primitive::Stroke { path, .. } => {
                    Some(path.bounding_box())
                }
                _ => None,
            })
            .reduce(|a, b| a.union(b))
            .unwrap_or(Rect::ZERO)
    }

    fn context() -> Context {
        Context {
            paint: Paint {
                weight: Weight::MEDIUM,
                ..Paint::default()
            },
            unit: 1.0,
            seed: 1,
            shadow: true,
        }
    }

    #[test]
    fn arrows_end_at_the_tip() {
        let drawn = arrow(Point::ORIGIN, Point::new(100.0, 0.0), &context());
        let reach = bounds(&drawn);
        assert!(reach.x1 <= 100.0 + 2.0);
        assert!(reach.x0 >= -2.0);
    }

    #[test]
    fn zero_length_arrows_are_a_dot() {
        let drawn = arrow(Point::ORIGIN, Point::ORIGIN, &context());
        assert_eq!(drawn.len(), 1);
    }

    #[test]
    fn marks_carry_a_shadow_below() {
        let drawn = primitives(
            &Kind::Line {
                from: Point::ORIGIN,
                to: Point::new(10.0, 0.0),
            },
            &context(),
        );
        let Primitive::Stroke { color, path, .. } = &drawn[0] else {
            panic!("the shadow comes first");
        };
        assert_eq!(*color, SHADOW);
        assert!(path.bounding_box().y0 > 0.0);
    }

    #[test]
    fn shadows_can_be_turned_off() {
        let flat = Context {
            shadow: false,
            ..context()
        };
        let kinds = [
            Kind::Line {
                from: Point::ORIGIN,
                to: Point::new(10.0, 0.0),
            },
            Kind::Arrow {
                from: Point::ORIGIN,
                to: Point::new(40.0, 0.0),
            },
            Kind::Counter {
                at: Point::new(20.0, 20.0),
                number: 1,
            },
        ];
        for kind in kinds {
            let shaded = primitives(&kind, &context());
            let plain = primitives(&kind, &flat);
            assert!(plain.len() < shaded.len(), "{kind:?}");
            assert!(plain.iter().all(|primitive| !matches!(
                primitive,
                Primitive::Fill { color, .. } | Primitive::Stroke { color, .. } if *color == SHADOW
            )));
        }
    }

    #[test]
    fn counters_center_their_number() {
        let drawn = counter(Point::new(50.0, 50.0), 3, &context());
        assert!(drawn.iter().any(|primitive| matches!(
            primitive,
            Primitive::Text { centered: true, text, .. } if text == "3"
        )));
    }
}
