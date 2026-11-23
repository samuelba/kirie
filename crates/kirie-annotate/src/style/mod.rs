//! Annotation styles: the same marks drawn crisp and clean, sketched by hand, or brushed in ink.

mod bubble;
mod clean;
mod ink;
mod sketch;
mod spotlight;

use kurbo::{BezPath, Point, Rect, Shape as _, Vec2};
use serde::{Deserialize, Serialize};

use crate::document::Document;
use crate::primitive::{Effect, FillRule, Font, Primitive};
use crate::shape::{Kind, PALETTE, Rgba, Shape, SpotShape};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
pub enum StyleId {
    /// Crisp vectors with a soft shadow.
    #[default]
    Clean,
    /// Hand-drawn lines and hatching, with handwritten text.
    Sketch,
    /// Brush strokes that swell and taper, like sumi ink.
    Ink,
}

impl StyleId {
    pub const ALL: [StyleId; 3] = [StyleId::Clean, StyleId::Sketch, StyleId::Ink];

    pub const fn name(self) -> &'static str {
        match self {
            StyleId::Clean => "Clean",
            StyleId::Sketch => "Sketch",
            StyleId::Ink => "Ink",
        }
    }

    /// The standard colors in this style's shades, in the order of [`PALETTE`]: bright pastels
    /// for pencil, the pigments of ink painting for ink.
    pub const fn palette(self) -> [Rgba; 6] {
        match self {
            StyleId::Clean => PALETTE,
            StyleId::Sketch => [
                Rgba::rgb(0xf2, 0x70, 0x6d),
                Rgba::rgb(0xf5, 0xc4, 0x51),
                Rgba::rgb(0x5f, 0xc8, 0x8f),
                Rgba::rgb(0x6a, 0xa6, 0xf8),
                Rgba::rgb(0xff, 0xfc, 0xf5),
                Rgba::rgb(0x3b, 0x3f, 0x48),
            ],
            StyleId::Ink => [
                Rgba::rgb(0xc7, 0x3a, 0x2c),
                Rgba::rgb(0xc9, 0x95, 0x2a),
                Rgba::rgb(0x2c, 0x7a, 0x54),
                Rgba::rgb(0x2b, 0x4f, 0x8f),
                Rgba::rgb(0xf5, 0xf0, 0xe6),
                Rgba::rgb(0x1f, 0x1d, 0x1a),
            ],
        }
    }

    /// The face text is set in unless a mark picks one.
    pub const fn font(self) -> Font {
        match self {
            StyleId::Clean => Font::Sans,
            StyleId::Sketch | StyleId::Ink => Font::Hand,
        }
    }
}

/// Everything the document's marks draw, bottom to top, in image coordinates.
pub fn render_list(document: &Document) -> Vec<Primitive> {
    let stage = Stage::new(document, document.visible(), document.shapes());
    document
        .shapes()
        .flat_map(|shape| primitives(shape, document.style, document.unit, &stage))
        .collect()
}

/// What marks are drawn onto: the image area shown, whether marks cast shadows, and the
/// spotlights. All spotlights dim that area once, together, where the lowest of them sits, so
/// overlapping spotlights never darken it twice.
#[derive(Debug, Clone)]
pub struct Stage {
    visible: Rect,
    shadow: bool,
    /// The opacity of the shade spotlights put over the rest.
    dim: f64,
    holes: Vec<spotlight::Hole>,
    lowest: Option<crate::shape::ShapeId>,
}

impl Default for Stage {
    fn default() -> Self {
        Self {
            visible: Rect::ZERO,
            shadow: false,
            dim: 1.0 - Document::SPOTLIGHT_BRIGHTNESS,
            holes: Vec::new(),
            lowest: None,
        }
    }
}

impl Stage {
    /// The stage for `document`'s look, showing `visible`, with `shapes` bottom to top.
    pub fn new<'a>(
        document: &Document,
        visible: Rect,
        shapes: impl IntoIterator<Item = &'a Shape>,
    ) -> Self {
        let mut stage = Self {
            visible,
            shadow: document.shadow,
            dim: 1.0 - Document::spotlight_brightness(document.spotlight_brightness),
            ..Self::default()
        };
        for shape in shapes {
            if let Kind::Spotlight { rect } = shape.kind {
                stage.lowest.get_or_insert(shape.id);
                stage.holes.push(spotlight::Hole {
                    rect: rect.abs(),
                    oval: shape.paint.spot == SpotShape::Ellipse,
                });
            }
        }
        stage
    }
}

/// The area `shape` covers in `style` on an image of `unit`, before stroke width; text is
/// measured in its face and size.
pub fn bounds(shape: &Shape, style: StyleId, unit: f64) -> Rect {
    match &shape.kind {
        Kind::Text { at, text } => {
            let font = text_font(shape, style);
            let size = crate::text::measure(text, font, text_size(shape, font, unit));
            Rect::from_origin_size(*at, size)
        }
        Kind::Bubble { tail, .. } => bubble_body(shape, style, unit)
            .unwrap_or_default()
            .union_pt(*tail),
        kind => kind.bounds(),
    }
}

/// The box of a speech bubble around its words, without its tail.
pub fn bubble_body(shape: &Shape, style: StyleId, unit: f64) -> Option<Rect> {
    let Kind::Bubble { center, text, .. } = &shape.kind else {
        return None;
    };
    Some(bubble::layout(shape, *center, text, style, unit).body)
}

fn text_font(shape: &Shape, style: StyleId) -> Font {
    shape
        .paint
        .text
        .font
        .unwrap_or(shape.style.unwrap_or(style).font())
}

fn text_size(shape: &Shape, font: Font, unit: f64) -> f64 {
    shape.paint.text.size * unit * font.scale()
}

/// Where `primitive` puts paint, roughly: its outline grown by its stroke.
pub fn reach(primitive: &Primitive) -> Option<Rect> {
    use kurbo::Shape as _;
    match primitive {
        Primitive::Fill { path, .. } | Primitive::Cover { path, .. } => Some(path.bounding_box()),
        Primitive::Stroke { path, width, .. } => {
            Some(path.bounding_box().inflate(width / 2.0, width / 2.0))
        }
        Primitive::Text {
            at,
            text,
            size,
            font,
            halo,
            centered,
            ..
        } => {
            let measured = crate::text::measure(text, *font, *size);
            let origin = if *centered {
                *at - Vec2::new(measured.width / 2.0, measured.height / 2.0)
            } else {
                *at
            };
            let halo = if halo.is_some() { size * 0.08 } else { 0.0 };
            Some(Rect::from_origin_size(origin, measured).inflate(halo, halo))
        }
        Primitive::Effect { rect, .. } | Primitive::Image { rect, .. } => Some(rect.abs()),
    }
}

/// What one mark draws on `stage`.
pub fn primitives(shape: &Shape, style: StyleId, unit: f64, stage: &Stage) -> Vec<Primitive> {
    let drawn = unturned(shape, style, unit, stage);
    if shape.rotation == 0.0 {
        return drawn;
    }
    let turn = shape.turn();
    drawn
        .into_iter()
        .map(|primitive| match primitive {
            Primitive::Fill { path, color, rule } => Primitive::Fill {
                path: turn * path,
                color,
                rule,
            },
            Primitive::Stroke { path, color, width } => Primitive::Stroke {
                path: turn * path,
                color,
                width,
            },
            Primitive::Cover { path, color } => Primitive::Cover {
                path: turn * path,
                color,
            },
            other => other,
        })
        .collect()
}

fn unturned(shape: &Shape, style: StyleId, unit: f64, stage: &Stage) -> Vec<Primitive> {
    let context = Context {
        paint: shape.paint,
        unit,
        seed: shape.id.0,
        shadow: stage.shadow,
    };
    match &shape.kind {
        Kind::Blur { rect } => vec![Primitive::Effect {
            rect: rect.abs(),
            effect: Effect::Blur {
                radius: 10.0 * unit,
            },
        }],
        Kind::Pixelate { rect } => vec![Primitive::Effect {
            rect: rect.abs(),
            effect: Effect::Pixelate { cell: 12.0 * unit },
        }],
        Kind::Spotlight { .. } if stage.lowest == Some(shape.id) => {
            vec![spotlight::shade(
                stage.visible,
                &stage.holes,
                6.0 * unit,
                stage.dim,
            )]
        }
        Kind::Spotlight { .. } => Vec::new(),
        Kind::Image { rect, pixels } => vec![Primitive::Image {
            rect: rect.abs(),
            pixels: pixels.clone(),
        }],
        Kind::Text { at, text } => {
            let font = text_font(shape, style);
            vec![text_mark(
                *at,
                text,
                font,
                text_size(shape, font, unit),
                &context,
            )]
        }
        Kind::Bubble { center, tail, text } => bubble::primitives(
            shape,
            *center,
            *tail,
            text,
            shape.style.unwrap_or(style),
            &context,
        ),
        Kind::Redact { rect } => {
            let style = shape.style.unwrap_or(style);
            let cover = Primitive::Cover {
                path: rect.abs().inflate(COVER_REACH, COVER_REACH).to_path(0.1),
                color: redact_color(style),
            };
            std::iter::once(cover)
                .chain(styled(&shape.kind, style, &context))
                .collect()
        }
        kind => styled(kind, shape.style.unwrap_or(style), &context),
    }
}

/// How far past a redaction its cover reaches, in pixels: enough to take in every pixel the
/// redaction touches, turned or not.
const COVER_REACH: f64 = 0.75;

fn styled(kind: &Kind, style: StyleId, context: &Context) -> Vec<Primitive> {
    match style {
        StyleId::Clean => clean::primitives(kind, context),
        StyleId::Sketch => sketch::primitives(kind, context),
        StyleId::Ink => ink::primitives(kind, context),
    }
}

fn redact_color(style: StyleId) -> Rgba {
    match style {
        StyleId::Clean => clean::REDACT,
        StyleId::Sketch => sketch::REDACT,
        StyleId::Ink => ink::REDACT,
    }
}

/// What a style needs to know about the mark it draws.
#[derive(Debug, Clone, Copy)]
struct Context {
    paint: crate::shape::Paint,
    unit: f64,
    /// Makes hand-drawn wobble the same on every redraw of the same mark.
    seed: u64,
    /// Whether marks cast a shadow, in the styles that draw one.
    shadow: bool,
}

impl Context {
    fn width(&self) -> f64 {
        self.paint.weight.base() * self.unit
    }

    fn color(&self) -> Rgba {
        self.paint.color
    }

    fn tint(&self) -> Rgba {
        self.paint.color.faded(0x40)
    }

    fn counter_radius(&self) -> f64 {
        (10.0 + self.paint.weight.base() * 1.5) * self.unit
    }
}

/// A text mark, the same in every style but for its face.
fn text_mark(at: Point, text: &str, font: Font, size: f64, context: &Context) -> Primitive {
    let look = context.paint.text;
    let color = context.color();
    Primitive::Text {
        at,
        text: text.to_owned(),
        size,
        color,
        font,
        align: look.align,
        halo: look.outlined.then(|| halo_for(color)),
        centered: false,
    }
}

/// A highlighter stroke: the pointer's path smoothed out, drawn as one band of even width with
/// flat ends, filled once so it never darkens where it crosses itself.
fn marker(points: &[Point], context: &Context) -> Primitive {
    let smoothed = chaikin(&thin_out(points, 2.0 * context.unit), 2);
    Primitive::Fill {
        path: ink::brush(
            &smoothed,
            context.width() * 4.5,
            ink::Taper {
                start: 0.0,
                end: 0.0,
            },
        ),
        color: context.color().faded(0x70),
        rule: FillRule::NonZero,
    }
}

/// Rounds the corners of a polyline `rounds` times, keeping its ends.
fn chaikin(points: &[Point], rounds: usize) -> Vec<Point> {
    let mut points = points.to_vec();
    for _ in 0..rounds {
        if points.len() < 3 {
            break;
        }
        let mut next = Vec::with_capacity(points.len() * 2);
        next.push(points[0]);
        for pair in points.windows(2) {
            next.push(pair[0].lerp(pair[1], 0.25));
            next.push(pair[0].lerp(pair[1], 0.75));
        }
        next.push(points[points.len() - 1]);
        points = next;
    }
    points
}

/// A dark or light outline color that sets `color` apart from any background.
fn halo_for(color: Rgba) -> Rgba {
    let luminance =
        0.2126 * f64::from(color.r) + 0.7152 * f64::from(color.g) + 0.0722 * f64::from(color.b);
    let (shade, alpha) = if luminance > 150.0 {
        (Rgba::rgb(0x16, 0x18, 0x1d), 0xd0)
    } else {
        (Rgba::rgb(0xff, 0xff, 0xff), 0xe6)
    };
    shade.with_alpha(color.faded(alpha).a)
}

/// Text that reads on `fill`.
fn text_on(fill: Rgba) -> Rgba {
    if halo_for(fill).r == 0xff {
        Rgba::rgb(0xff, 0xff, 0xff)
    } else {
        Rgba::rgb(0x16, 0x18, 0x1d)
    }
}

/// The two back corners of an arrow head of `length` pointing at `tip` from `from`.
fn arrow_wings(from: Point, tip: Point, length: f64, spread: f64) -> Option<(Point, Point)> {
    let direction = tip - from;
    let distance = direction.hypot();
    if distance < f64::EPSILON {
        return None;
    }
    let back = direction / distance * -length;
    let (sin, cos) = spread.sin_cos();
    let rotate = |v: Vec2, sin: f64| Vec2::new(v.x * cos - v.y * sin, v.x * sin + v.y * cos);
    Some((tip + rotate(back, sin), tip + rotate(back, -sin)))
}

/// A smooth curve through `points` (Catmull-Rom turned into cubic Béziers).
fn smooth(points: &[Point]) -> BezPath {
    let mut path = BezPath::new();
    let Some(first) = points.first() else {
        return path;
    };
    path.move_to(*first);
    if points.len() == 1 {
        path.line_to(*first);
        return path;
    }
    for index in 0..points.len() - 1 {
        let p0 = points[index.saturating_sub(1)];
        let p1 = points[index];
        let p2 = points[index + 1];
        let p3 = points[(index + 2).min(points.len() - 1)];
        let c1 = p1 + (p2 - p0) / 6.0;
        let c2 = p2 - (p3 - p1) / 6.0;
        path.curve_to(c1, c2, p2);
    }
    path
}

/// `count + 1` points along an ellipse from `start` radians over `sweep` radians.
fn ellipse_points(rect: Rect, count: usize, start: f64, sweep: f64) -> Vec<Point> {
    let center = rect.center();
    let (rx, ry) = (rect.width() / 2.0, rect.height() / 2.0);
    #[allow(clippy::cast_precision_loss)]
    (0..=count)
        .map(|index| {
            let angle = start + sweep * index as f64 / count as f64;
            Point::new(center.x + rx * angle.cos(), center.y + ry * angle.sin())
        })
        .collect()
}

/// Drops points closer than `min` to the previous kept one; the last point always stays.
fn thin_out(points: &[Point], min: f64) -> Vec<Point> {
    let mut kept: Vec<Point> = Vec::with_capacity(points.len());
    for (index, point) in points.iter().enumerate() {
        let last = index == points.len() - 1;
        if kept
            .last()
            .is_none_or(|previous| previous.distance(*point) >= min)
            || last
        {
            kept.push(*point);
        }
    }
    kept
}

/// A small, fast generator; seeded per mark so its wobble never changes.
#[derive(Debug, Clone)]
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed ^ 0x9e37_79b9_7f4a_7c15)
    }

    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    /// A value in `-1.0..1.0`.
    #[allow(clippy::cast_precision_loss)]
    fn signed(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64 * 2.0 - 1.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::Op;
    use crate::shape::{Paint, ShapeId, Weight};

    fn every_kind() -> Vec<Kind> {
        let rect = Rect::new(10.0, 10.0, 60.0, 40.0);
        let (a, b) = (Point::new(5.0, 5.0), Point::new(80.0, 50.0));
        vec![
            Kind::Arrow { from: a, to: b },
            Kind::Line { from: a, to: b },
            Kind::Rectangle { rect },
            Kind::Ellipse { rect },
            Kind::Pen {
                points: vec![a, Point::new(20.0, 30.0), b],
            },
            Kind::Highlighter { points: vec![a, b] },
            Kind::Text {
                at: a,
                text: "Hi".into(),
            },
            Kind::Counter { at: a, number: 2 },
            Kind::Bubble {
                center: Point::new(50.0, 20.0),
                tail: b,
                text: "Hi\nthere".into(),
            },
            Kind::Blur { rect },
            Kind::Pixelate { rect },
            Kind::Redact { rect },
            Kind::Spotlight { rect },
        ]
    }

    #[test]
    fn every_style_draws_every_kind() {
        for style in StyleId::ALL {
            for filled in [false, true] {
                for kind in every_kind() {
                    let shape = Shape {
                        id: ShapeId(7),
                        kind: kind.clone(),
                        paint: Paint {
                            filled,
                            ..Paint::default()
                        },
                        style: None,
                        rotation: 0.0,
                    };
                    let stage = Stage::new(
                        &Document::new(100, 100, 1.0),
                        Rect::new(0.0, 0.0, 100.0, 100.0),
                        [&shape],
                    );
                    let drawn = primitives(&shape, style, 1.0, &stage);
                    assert!(!drawn.is_empty(), "{style:?} drew nothing for {kind:?}");
                }
            }
        }
    }

    #[test]
    fn drawing_is_deterministic() {
        let document = Document::new(100, 100, 1.0)
            .apply(Op::SetStyle {
                style: StyleId::Sketch,
                recolor: Vec::new(),
            })
            .apply(Op::Add {
                kind: Kind::Rectangle {
                    rect: Rect::new(10.0, 10.0, 50.0, 50.0),
                },
                paint: Paint::default(),
            });
        assert_eq!(render_list(&document), render_list(&document));
    }

    #[test]
    fn a_marks_own_style_wins() {
        let shape = Shape {
            id: ShapeId(1),
            kind: Kind::Text {
                at: Point::ORIGIN,
                text: "x".into(),
            },
            paint: Paint::default(),
            style: Some(StyleId::Sketch),
            rotation: 0.0,
        };
        let drawn = primitives(&shape, StyleId::Clean, 1.0, &Stage::default());
        assert!(drawn.iter().any(|primitive| matches!(
            primitive,
            Primitive::Text {
                font: crate::primitive::Font::Hand,
                ..
            }
        )));
    }

    #[test]
    fn spotlights_dim_once_together_from_the_lowest() {
        let spot = |x0: f64, x1: f64| Op::Add {
            kind: Kind::Spotlight {
                rect: Rect::new(x0, 20.0, x1, 80.0),
            },
            paint: Paint::default(),
        };
        let document = Document::new(100, 100, 1.0)
            .apply(spot(10.0, 60.0))
            .apply(spot(40.0, 90.0));
        let drawn = render_list(&document);
        let [Primitive::Fill { path, rule, .. }] = drawn.as_slice() else {
            panic!("one dimming for both spotlights: {drawn:?}");
        };
        assert_eq!(*rule, FillRule::NonZero);
        let winding = |x, y| path.winding(Point::new(x, y));
        assert_eq!(winding(5.0, 5.0).abs(), 1, "outside both");
        assert_eq!(winding(50.0, 50.0), 0, "where they overlap");
        assert_eq!(winding(20.0, 50.0), 0, "inside the first");
        assert_eq!(winding(80.0, 50.0), 0, "inside the second");
        assert_eq!(
            winding(95.0, 50.0).abs(),
            1,
            "outside both, between them and the edge"
        );
        assert_eq!(winding(10.5, 20.5).abs(), 1, "a rounded-off corner");
        assert_eq!(
            winding(40.5, 20.5),
            0,
            "a corner inside the other spotlight"
        );
    }

    #[test]
    fn spotlights_dim_as_bright_as_asked() {
        let shade = |brightness: f64| {
            let document = Document::new(100, 100, 1.0)
                .apply(Op::SetSpotlightBrightness(brightness))
                .apply(Op::Add {
                    kind: Kind::Spotlight {
                        rect: Rect::new(10.0, 10.0, 50.0, 50.0),
                    },
                    paint: Paint::default(),
                });
            match render_list(&document).as_slice() {
                [Primitive::Fill { color, .. }] => color.a,
                drawn => panic!("{drawn:?}"),
            }
        };
        assert_eq!(shade(Document::SPOTLIGHT_BRIGHTNESS), 0x8c);
        assert!(shade(0.8) < shade(0.2));
    }

    #[test]
    fn styles_have_names() {
        for style in StyleId::ALL {
            assert!(!style.name().is_empty());
        }
    }

    #[test]
    fn every_style_shades_the_standard_colors_its_own_way() {
        assert_eq!(StyleId::Clean.palette(), PALETTE);
        for style in StyleId::ALL {
            let palette = style.palette();
            for (index, color) in palette.iter().enumerate() {
                assert!(!palette[index + 1..].contains(color), "{style:?} repeats");
            }
            for other in StyleId::ALL.into_iter().filter(|other| *other != style) {
                assert!(
                    palette.iter().all(|color| !other.palette().contains(color)),
                    "{style:?} and {other:?} share a shade"
                );
            }
        }
    }

    #[test]
    fn markers_are_smooth_even_bands() {
        let shaky: Vec<Point> = (0..40)
            .map(|i| Point::new(f64::from(i) * 5.0, if i % 2 == 0 { 0.0 } else { 1.5 }))
            .collect();
        let smoothed = chaikin(&shaky, 2);
        let wobble = |points: &[Point]| {
            points
                .windows(2)
                .map(|p| (p[1].y - p[0].y).abs())
                .fold(0.0, f64::max)
        };
        assert!(wobble(&smoothed) < wobble(&shaky));
        assert_eq!(smoothed.first(), shaky.first());
        assert_eq!(smoothed.last(), shaky.last());
        assert_eq!(chaikin(&shaky[..2], 3), shaky[..2].to_vec());
    }

    #[test]
    fn halos_contrast() {
        assert_eq!(halo_for(Rgba::rgb(255, 255, 255)).r, 0x16);
        assert_eq!(halo_for(Rgba::rgb(0, 0, 0)).r, 0xff);
        assert_eq!(text_on(Rgba::rgb(0xe4, 0x57, 0x2e)).r, 0xff);
        assert_eq!(text_on(Rgba::rgb(0xf4, 0xef, 0xe6)).r, 0x16);
    }

    #[test]
    fn arrow_wings_are_symmetric() {
        let (left, right) = arrow_wings(Point::ORIGIN, Point::new(10.0, 0.0), 5.0, 0.5).unwrap();
        assert!((left.x - right.x).abs() < 1e-9);
        assert!((left.y + right.y).abs() < 1e-9);
        assert!(left.x < 10.0);
        assert!(arrow_wings(Point::ORIGIN, Point::ORIGIN, 5.0, 0.5).is_none());
    }

    #[test]
    fn helpers() {
        assert!(smooth(&[]).elements().is_empty());
        assert_eq!(smooth(&[Point::ORIGIN]).elements().len(), 2);
        let curve = smooth(&[Point::ORIGIN, Point::new(1.0, 1.0), Point::new(2.0, 0.0)]);
        assert_eq!(curve.elements().len(), 3);
        let ring = ellipse_points(Rect::new(0.0, 0.0, 2.0, 2.0), 4, 0.0, std::f64::consts::TAU);
        assert_eq!(ring.len(), 5);
        assert!((ring[0].x - 2.0).abs() < 1e-9);
        let kept = thin_out(
            &[
                Point::ORIGIN,
                Point::new(0.1, 0.0),
                Point::new(5.0, 0.0),
                Point::new(5.1, 0.0),
            ],
            1.0,
        );
        assert_eq!(kept.len(), 3);
        let mut rng = Rng::new(1);
        let values: Vec<f64> = (0..100).map(|_| rng.signed()).collect();
        assert!(values.iter().all(|v| (-1.0..1.0).contains(v)));
        assert!(values.iter().any(|v| *v < 0.0) && values.iter().any(|v| *v > 0.0));
        let context = Context {
            paint: Paint {
                weight: Weight::THICK,
                ..Paint::default()
            },
            unit: 2.0,
            seed: 0,
            shadow: true,
        };
        assert!((context.width() - 14.0).abs() < 1e-9);
        assert_eq!(context.tint().a, 0x40);
    }
}
