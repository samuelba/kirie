//! Speech bubbles: words in a box filled with the mark's color, with a tail that points at
//! something.

use kurbo::{BezPath, Point, Rect, RoundedRect, Shape as _, Vec2};

use super::{Context, StyleId, clean, ink, sketch, text_font, text_on, text_size};
use crate::primitive::{Font, Primitive};
use crate::shape::Shape;

/// Where a bubble's words sit, and the box around them.
pub(super) struct Layout {
    pub font: Font,
    pub size: f64,
    /// The top left of the words.
    pub words: Point,
    pub body: Rect,
}

pub(super) fn layout(
    shape: &Shape,
    center: Point,
    text: &str,
    style: StyleId,
    unit: f64,
) -> Layout {
    let font = text_font(shape, style);
    let size = text_size(shape, font, unit);
    let words = Rect::from_center_size(center, crate::text::measure(text, font, size));
    Layout {
        font,
        size,
        words: words.origin(),
        body: words.inflate(size * 0.5, size * 0.3),
    }
}

/// The box and its tail as one outline. The tail is wound the same way as the box, so a
/// nonzero fill joins them without filling their overlap twice.
pub(super) fn outline(body: Rect, tail: Point, radius: f64) -> BezPath {
    let radius = radius.min(body.width() / 2.0).min(body.height() / 2.0);
    let mut path = RoundedRect::from_rect(body, radius).to_path(0.1);
    if body.contains(tail) {
        return path;
    }
    let center = body.center();
    let along = tail - center;
    let half = body.width().min(body.height()) * 0.32;
    let across = Vec2::new(-along.y, along.x) / along.hypot() * half;
    let (left, right) = (center + across, center - across);
    let same_way = (right - left).cross(tail - left).signum() == path.area().signum();
    let (first, second) = if same_way {
        (left, right)
    } else {
        (right, left)
    };
    path.move_to(first);
    path.line_to(second);
    path.line_to(tail);
    path.close_path();
    path
}

/// What a bubble draws in `style`: its filled outline, then its words in a color that reads
/// on the fill.
pub(super) fn primitives(
    shape: &Shape,
    center: Point,
    tail: Point,
    text: &str,
    style: StyleId,
    context: &Context,
) -> Vec<Primitive> {
    let layout = layout(shape, center, text, style, context.unit);
    let outline = outline(layout.body, tail, layout.size * 0.5);
    let mut drawn = match style {
        StyleId::Clean => clean::bubble(outline, context),
        StyleId::Sketch => sketch::bubble(&outline, context),
        StyleId::Ink => ink::bubble(outline, context),
    };
    drawn.push(Primitive::Text {
        at: layout.words,
        text: text.to_owned(),
        size: layout.size,
        color: text_on(context.color()),
        font: layout.font,
        align: context.paint.text.align,
        halo: None,
        centered: false,
    });
    drawn
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Rect {
        Rect::new(0.0, 0.0, 100.0, 40.0)
    }

    #[test]
    fn the_tail_reaches_its_tip_and_joins_the_box() {
        let tip = Point::new(150.0, 120.0);
        let path = outline(body(), tip, 8.0);
        assert!(path.bounding_box().inflate(0.01, 0.01).contains(tip));
        let toward = body().center().lerp(tip, 0.6);
        assert_ne!(path.winding(toward), 0, "inside the tail");
        let overlap = body().center().lerp(tip, 0.05);
        assert_eq!(path.winding(overlap).abs(), 2, "box and tail wound alike");
    }

    #[test]
    fn a_tip_inside_the_box_draws_no_tail() {
        let drawn = outline(body(), Point::new(50.0, 20.0), 8.0).bounding_box();
        assert!(drawn.inflate(0.01, 0.01).contains_rect(body()));
        assert!(body().inflate(0.01, 0.01).contains_rect(drawn));
    }

    #[test]
    fn tails_join_whichever_way_they_point() {
        for tip in [
            Point::new(-80.0, -60.0),
            Point::new(200.0, 20.0),
            Point::new(50.0, 140.0),
            Point::new(-90.0, 90.0),
        ] {
            let path = outline(body(), tip, 8.0);
            let overlap = body().center().lerp(tip, 0.05);
            assert_eq!(path.winding(overlap).abs(), 2, "{tip:?}");
        }
    }
}
