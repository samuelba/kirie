//! Draws a document onto its image with tiny-skia: what gets saved, copied and exported.

use image::RgbaImage;
use kurbo::{Affine, BezPath, PathEl, Vec2};
use tiny_skia::{
    FillRule as SkiaFillRule, LineCap, LineJoin, Paint, PathBuilder, Pixmap, Stroke, Transform,
};

use crate::document::Document;
use crate::effects::{self, Region};
use crate::primitive::{Effect, FillRule, Primitive};
use crate::shape::Rgba;
use crate::style::render_list;
use crate::text::Typesetter;

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum RenderError {
    #[error("the image is {got_width}×{got_height}, the document expects {width}×{height}")]
    SizeMismatch {
        width: u32,
        height: u32,
        got_width: u32,
        got_height: u32,
    },
    #[error("the image is empty")]
    Empty,
}

/// The finished image: what the document's canvas covers, the kept part of `base` in its
/// frame, with every mark drawn on top.
pub fn render(
    document: &Document,
    base: &RgbaImage,
    typesetter: &mut Typesetter,
) -> Result<RgbaImage, RenderError> {
    if base.dimensions() != (document.width, document.height) {
        return Err(RenderError::SizeMismatch {
            width: document.width,
            height: document.height,
            got_width: base.width(),
            got_height: base.height(),
        });
    }
    let kept =
        Region::clamp(document.visible(), base.width(), base.height()).ok_or(RenderError::Empty)?;
    let canvas = document.canvas();
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let mut pixmap =
        Pixmap::new(canvas.width() as u32, canvas.height() as u32).ok_or(RenderError::Empty)?;
    let frame = document.frame;
    if let Some(background) = frame.background {
        pixmap.fill(tiny_skia::Color::from_rgba8(
            background.r,
            background.g,
            background.b,
            background.a,
        ));
    }
    let shift = -canvas.origin().to_vec2();
    #[allow(clippy::cast_precision_loss)]
    let picture = kurbo::Rect::new(
        kept.x as f64,
        kept.y as f64,
        (kept.x + kept.width) as f64,
        (kept.y + kept.height) as f64,
    ) + shift;
    if frame.shadow {
        cast_shadow(&mut pixmap, picture, frame.margin, document.unit);
    }
    #[allow(clippy::cast_possible_truncation)]
    let part = image::imageops::crop_imm(
        base,
        kept.x as u32,
        kept.y as u32,
        kept.width as u32,
        kept.height as u32,
    )
    .to_image();
    let part = to_pixmap(&part).ok_or(RenderError::Empty)?;
    #[allow(clippy::cast_possible_truncation)]
    pixmap.draw_pixmap(
        picture.x0 as i32,
        picture.y0 as i32,
        part.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );
    for primitive in render_list(document) {
        draw(&mut pixmap, &shifted(primitive, shift), typesetter);
    }
    Ok(from_pixmap(&pixmap))
}

/// `primitive` moved `by`.
fn shifted(primitive: Primitive, by: Vec2) -> Primitive {
    let moved = Affine::translate(by);
    match primitive {
        Primitive::Fill { path, color, rule } => Primitive::Fill {
            path: moved * path,
            color,
            rule,
        },
        Primitive::Stroke { path, color, width } => Primitive::Stroke {
            path: moved * path,
            color,
            width,
        },
        Primitive::Cover { path, color } => Primitive::Cover {
            path: moved * path,
            color,
        },
        Primitive::Text {
            at,
            text,
            size,
            color,
            font,
            align,
            halo,
            centered,
        } => Primitive::Text {
            at: at + by,
            text,
            size,
            color,
            font,
            align,
            halo,
            centered,
        },
        Primitive::Effect { rect, effect } => Primitive::Effect {
            rect: rect + by,
            effect,
        },
    }
}

/// A soft shadow under `picture`, as deep as the `margin` around it leaves room for.
fn cast_shadow(pixmap: &mut Pixmap, picture: kurbo::Rect, margin: f64, unit: f64) {
    let radius = (margin * 0.4).min(24.0 * unit);
    if radius < 1.0 {
        return;
    }
    let Some(mut layer) = Pixmap::new(pixmap.width(), pixmap.height()) else {
        return;
    };
    let dropped = picture + Vec2::new(0.0, radius * 0.4);
    #[allow(clippy::cast_possible_truncation)]
    let Some(rect) = tiny_skia::Rect::from_ltrb(
        dropped.x0 as f32,
        dropped.y0 as f32,
        dropped.x1 as f32,
        dropped.y1 as f32,
    ) else {
        return;
    };
    layer.fill_rect(
        rect,
        &paint(Rgba::rgb(0, 0, 0).with_alpha(0x73)),
        Transform::identity(),
        None,
    );
    let region = Region {
        x: 0,
        y: 0,
        width: layer.width() as usize,
        height: layer.height() as usize,
    };
    let stride = region.width * 4;
    effects::soften(layer.data_mut(), stride, region, radius);
    pixmap.draw_pixmap(
        0,
        0,
        layer.as_ref(),
        &tiny_skia::PixmapPaint::default(),
        Transform::identity(),
        None,
    );
}

/// Draws one primitive.
pub fn draw(pixmap: &mut Pixmap, primitive: &Primitive, typesetter: &mut Typesetter) {
    match primitive {
        Primitive::Fill { path, color, rule } => fill(pixmap, path, *color, *rule),
        Primitive::Stroke { path, color, width } => stroke(pixmap, path, *color, *width),
        Primitive::Text {
            at,
            text,
            size,
            color,
            font,
            align,
            halo,
            centered,
        } => {
            let outline = typesetter.outline(text, *font, *size, *at, *align, *centered);
            if let Some(halo) = halo {
                stroke(pixmap, &outline, *halo, size * 0.16);
            }
            fill(pixmap, &outline, *color, FillRule::NonZero);
        }
        Primitive::Cover { path, color } => cover(pixmap, path, *color),
        Primitive::Effect { rect, effect } => {
            let Some(region) = Region::clamp(*rect, pixmap.width(), pixmap.height()) else {
                return;
            };
            let stride = pixmap.width() as usize * 4;
            match effect {
                Effect::Blur { radius } => {
                    effects::blur(pixmap.data_mut(), stride, region, *radius);
                }
                Effect::Pixelate { cell } => {
                    effects::pixelate(pixmap.data_mut(), stride, region, *cell);
                }
            }
        }
    }
}

fn paint(color: Rgba) -> Paint<'static> {
    let mut paint = Paint::default();
    paint.set_color_rgba8(color.r, color.g, color.b, color.a);
    paint.anti_alias = true;
    paint
}

/// Fills every pixel whose center is inside `path` with `color`, fully opaque.
fn cover(pixmap: &mut Pixmap, path: &BezPath, color: Rgba) {
    let Some(path) = to_skia(path) else {
        return;
    };
    let mut paint = paint(color.with_alpha(0xff));
    paint.anti_alias = false;
    pixmap.fill_path(
        &path,
        &paint,
        SkiaFillRule::Winding,
        Transform::identity(),
        None,
    );
}

fn fill(pixmap: &mut Pixmap, path: &BezPath, color: Rgba, rule: FillRule) {
    let Some(path) = to_skia(path) else {
        return;
    };
    let rule = match rule {
        FillRule::NonZero => SkiaFillRule::Winding,
        FillRule::EvenOdd => SkiaFillRule::EvenOdd,
    };
    pixmap.fill_path(&path, &paint(color), rule, Transform::identity(), None);
}

#[allow(clippy::cast_possible_truncation)]
fn stroke(pixmap: &mut Pixmap, path: &BezPath, color: Rgba, width: f64) {
    let Some(path) = to_skia(path) else {
        return;
    };
    let stroke = Stroke {
        width: width as f32,
        line_cap: LineCap::Round,
        line_join: LineJoin::Round,
        ..Stroke::default()
    };
    pixmap.stroke_path(&path, &paint(color), &stroke, Transform::identity(), None);
}

#[allow(clippy::cast_possible_truncation)]
fn to_skia(path: &BezPath) -> Option<tiny_skia::Path> {
    let mut builder = PathBuilder::new();
    let f32s = |p: kurbo::Point| (p.x as f32, p.y as f32);
    for element in path.elements() {
        match *element {
            PathEl::MoveTo(p) => {
                let (x, y) = f32s(p);
                builder.move_to(x, y);
            }
            PathEl::LineTo(p) => {
                let (x, y) = f32s(p);
                builder.line_to(x, y);
            }
            PathEl::QuadTo(c, p) => {
                let ((cx, cy), (x, y)) = (f32s(c), f32s(p));
                builder.quad_to(cx, cy, x, y);
            }
            PathEl::CurveTo(c1, c2, p) => {
                let ((ax, ay), (bx, by), (x, y)) = (f32s(c1), f32s(c2), f32s(p));
                builder.cubic_to(ax, ay, bx, by, x, y);
            }
            PathEl::ClosePath => builder.close(),
        }
    }
    builder.finish()
}

/// A premultiplied pixmap of a straight-alpha image.
pub fn to_pixmap(image: &RgbaImage) -> Option<Pixmap> {
    let mut pixmap = Pixmap::new(image.width(), image.height())?;
    for (target, source) in pixmap
        .data_mut()
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip(image.as_raw().as_chunks::<4>().0)
    {
        let alpha = u16::from(source[3]);
        #[allow(clippy::cast_possible_truncation)]
        let premultiply = |channel: u8| ((u16::from(channel) * alpha + 127) / 255) as u8;
        target.copy_from_slice(&[
            premultiply(source[0]),
            premultiply(source[1]),
            premultiply(source[2]),
            source[3],
        ]);
    }
    Some(pixmap)
}

/// A straight-alpha image of a premultiplied pixmap.
pub fn from_pixmap(pixmap: &Pixmap) -> RgbaImage {
    let mut out = Vec::with_capacity(pixmap.data().len());
    for pixel in pixmap.pixels() {
        let color = pixel.demultiply();
        out.extend_from_slice(&[color.red(), color.green(), color.blue(), color.alpha()]);
    }
    RgbaImage::from_raw(pixmap.width(), pixmap.height(), out)
        .expect("a pixmap has width × height pixels")
}

#[cfg(test)]
mod tests {
    use image::Rgba as Pixel;
    use kurbo::{Point, Rect};

    use super::*;
    use crate::document::Op;
    use crate::shape::{Kind, Paint as MarkPaint};

    fn white(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Pixel([255, 255, 255, 255]))
    }

    fn add(document: &Document, kind: Kind) -> Document {
        document.apply(Op::Add {
            kind,
            paint: MarkPaint::default(),
        })
    }

    #[test]
    fn pixmaps_round_trip() {
        let mut image = white(3, 1);
        image.put_pixel(1, 0, Pixel([200, 100, 50, 128]));
        image.put_pixel(2, 0, Pixel([0, 0, 0, 0]));
        let back = from_pixmap(&to_pixmap(&image).unwrap());
        assert_eq!(back.get_pixel(0, 0), &Pixel([255, 255, 255, 255]));
        let half = back.get_pixel(1, 0);
        assert!(half[0].abs_diff(200) <= 2 && half[3] == 128);
        assert_eq!(back.get_pixel(2, 0)[3], 0);
        assert!(to_pixmap(&RgbaImage::new(0, 0)).is_none());
    }

    #[test]
    fn a_redaction_hides_what_is_under_it() {
        let document = add(
            &Document::new(40, 40, 1.0),
            Kind::Redact {
                rect: Rect::new(10.0, 10.0, 30.0, 30.0),
            },
        );
        let out = render(&document, &white(40, 40), &mut Typesetter::new()).unwrap();
        assert!(out.get_pixel(20, 20)[0] < 40);
        assert_eq!(out.get_pixel(2, 2), &Pixel([255, 255, 255, 255]));
    }

    #[test]
    fn a_redaction_leaves_no_trace_in_any_pixel_it_touches() {
        let rect = Rect::new(10.3, 10.7, 29.6, 29.2);
        for style in crate::style::StyleId::ALL {
            for rotation in [0.0, 0.3] {
                let redacted = add(
                    &Document::new(40, 40, 1.0).apply(Op::SetStyle {
                        style,
                        recolor: Vec::new(),
                    }),
                    Kind::Redact { rect },
                );
                let id = redacted.shapes().next().unwrap().id;
                let document = redacted.apply(Op::Reshape {
                    id,
                    kind: Kind::Redact { rect },
                    rotation,
                });
                let out = render(&document, &white(40, 40), &mut Typesetter::new()).unwrap();
                let turn = document.shape(id).unwrap().turn();
                let touched = |x: u32, y: u32| {
                    let middle =
                        turn.inverse() * Point::new(f64::from(x) + 0.5, f64::from(y) + 0.5);
                    rect.inflate(0.5, 0.5).contains(middle)
                };
                let colors: std::collections::HashSet<_> = (0..40)
                    .flat_map(|y| (0..40).map(move |x| (x, y)))
                    .filter(|&(x, y)| touched(x, y))
                    .map(|(x, y)| *out.get_pixel(x, y))
                    .collect();
                assert_eq!(colors.len(), 1, "{style:?} turned {rotation}: {colors:?}");
            }
        }
    }

    #[test]
    fn lines_draw_in_their_color() {
        let document = add(
            &Document::new(40, 40, 1.0),
            Kind::Line {
                from: Point::new(5.0, 20.0),
                to: Point::new(35.0, 20.0),
            },
        );
        let out = render(&document, &white(40, 40), &mut Typesetter::new()).unwrap();
        let pixel = out.get_pixel(20, 20);
        assert_eq!((pixel[0], pixel[1], pixel[2]), (0xe4, 0x57, 0x2e));
    }

    #[test]
    fn text_puts_ink_on_the_page() {
        let document = add(
            &Document::new(120, 60, 1.0),
            Kind::Text {
                at: Point::new(10.0, 10.0),
                text: "Kirie".into(),
            },
        );
        let out = render(&document, &white(120, 60), &mut Typesetter::new()).unwrap();
        let inked = out.pixels().filter(|pixel| pixel[1] < 200).count();
        assert!(inked > 50, "{inked}");
    }

    fn framed(document: &Document, frame: crate::Frame) -> Document {
        document.apply(Op::SetFrame(frame))
    }

    #[test]
    fn a_frame_puts_room_around_the_picture() {
        let red = Rgba::rgb(200, 0, 0);
        let document = framed(
            &Document::new(40, 30, 1.0),
            crate::Frame {
                margin: 20.0,
                background: Some(red),
                shadow: true,
                overflow: false,
            },
        );
        let out = render(&document, &white(40, 30), &mut Typesetter::new()).unwrap();
        assert_eq!(out.dimensions(), (80, 70));
        assert_eq!(out.get_pixel(1, 1), &Pixel([200, 0, 0, 255]));
        assert_eq!(out.get_pixel(40, 35), &Pixel([255, 255, 255, 255]));
        let under = out.get_pixel(40, 53);
        assert!(
            under[0] < 190 && under[3] == 255,
            "a shadow below: {under:?}"
        );
    }

    #[test]
    fn an_empty_background_is_transparent() {
        let document = framed(
            &Document::new(10, 10, 1.0),
            crate::Frame {
                margin: 5.0,
                ..crate::Frame::default()
            },
        );
        let out = render(&document, &white(10, 10), &mut Typesetter::new()).unwrap();
        assert_eq!(out.dimensions(), (20, 20));
        assert_eq!(out.get_pixel(0, 0)[3], 0);
        assert_eq!(out.get_pixel(10, 10), &Pixel([255, 255, 255, 255]));
    }

    #[test]
    fn marks_beyond_the_picture_can_be_taken_in() {
        let line = add(
            &Document::new(40, 40, 1.0),
            Kind::Line {
                from: Point::new(20.0, 20.0),
                to: Point::new(70.0, 20.0),
            },
        );
        let cut = render(&line, &white(40, 40), &mut Typesetter::new()).unwrap();
        assert_eq!(cut.dimensions(), (40, 40));
        let whole = framed(
            &line,
            crate::Frame {
                overflow: true,
                ..crate::Frame::default()
            },
        );
        let out = render(&whole, &white(40, 40), &mut Typesetter::new()).unwrap();
        assert!(out.width() >= 70, "{:?}", out.dimensions());
        let tip = out.get_pixel(65, 20);
        assert_eq!((tip[0], tip[1], tip[2]), (0xe4, 0x57, 0x2e));
        assert_eq!(out.get_pixel(60, 5)[3], 0);
    }

    #[test]
    fn crops_cut_the_result() {
        let document =
            Document::new(40, 30, 1.0).apply(Op::Crop(Some(Rect::new(5.0, 5.0, 25.0, 15.0))));
        let out = render(&document, &white(40, 30), &mut Typesetter::new()).unwrap();
        assert_eq!(out.dimensions(), (20, 10));
    }

    #[test]
    fn overlapping_spotlights_dim_evenly() {
        let spot = |x0: f64, x1: f64| Kind::Spotlight {
            rect: Rect::new(x0, 10.0, x1, 30.0),
        };
        let one = add(&Document::new(60, 40, 1.0), spot(5.0, 30.0));
        let two = add(&one, spot(20.0, 50.0));
        let single = render(&one, &white(60, 40), &mut Typesetter::new()).unwrap();
        let double = render(&two, &white(60, 40), &mut Typesetter::new()).unwrap();
        assert_eq!(double.get_pixel(25, 20), &Pixel([255, 255, 255, 255]));
        assert_eq!(double.get_pixel(45, 20), &Pixel([255, 255, 255, 255]));
        assert_eq!(double.get_pixel(55, 2), single.get_pixel(55, 2));
        assert!(single.get_pixel(55, 2)[0] < 150);
    }

    #[test]
    fn effects_change_only_their_region() {
        let mut base = white(40, 40);
        for x in 0..40 {
            base.put_pixel(x, 20, Pixel([0, 0, 0, 255]));
        }
        let document = add(
            &Document::new(40, 40, 1.0),
            Kind::Blur {
                rect: Rect::new(0.0, 10.0, 20.0, 30.0),
            },
        );
        let out = render(&document, &base, &mut Typesetter::new()).unwrap();
        assert!(out.get_pixel(10, 20)[0] > 0, "blurred inside");
        assert_eq!(out.get_pixel(30, 20)[0], 0, "sharp outside");
        let pixelated = add(
            &Document::new(40, 40, 1.0),
            Kind::Pixelate {
                rect: Rect::new(0.0, 0.0, 40.0, 40.0),
            },
        );
        assert!(render(&pixelated, &base, &mut Typesetter::new()).is_ok());
        let outside = add(
            &Document::new(40, 40, 1.0),
            Kind::Blur {
                rect: Rect::new(100.0, 100.0, 120.0, 120.0),
            },
        );
        assert_eq!(
            render(&outside, &base, &mut Typesetter::new()).unwrap(),
            base
        );
    }

    #[test]
    fn the_image_must_match_the_document() {
        let error = render(
            &Document::new(10, 10, 1.0),
            &white(5, 5),
            &mut Typesetter::new(),
        );
        assert!(matches!(error, Err(RenderError::SizeMismatch { .. })));
    }
}
