//! Draws a document onto its image with tiny-skia: what gets saved, copied and exported.

use image::RgbaImage;
use kurbo::{BezPath, PathEl};
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

/// The finished image: `base` with every mark drawn on it, cut to the document's crop.
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
    let mut pixmap = to_pixmap(base).ok_or(RenderError::Empty)?;
    for primitive in render_list(document) {
        draw(&mut pixmap, &primitive, typesetter);
    }
    let image = from_pixmap(&pixmap);
    let Some(crop) = document
        .crop
        .and_then(|crop| Region::clamp(crop, image.width(), image.height()))
    else {
        return Ok(image);
    };
    #[allow(clippy::cast_possible_truncation)]
    Ok(image::imageops::crop_imm(
        &image,
        crop.x as u32,
        crop.y as u32,
        crop.width as u32,
        crop.height as u32,
    )
    .to_image())
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
            halo,
            centered,
        } => {
            let outline = typesetter.outline(text, *font, *size, *at, *centered);
            if let Some(halo) = halo {
                stroke(pixmap, &outline, *halo, size * 0.16);
            }
            fill(pixmap, &outline, *color, FillRule::NonZero);
        }
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
