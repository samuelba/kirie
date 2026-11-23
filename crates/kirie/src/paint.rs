//! Draws annotation primitives on an iced canvas, matching what `kirie_annotate::render` saves:
//! the same paths, the same glyph outlines, and effects computed from the pixels underneath.

use std::cell::RefCell;
use std::collections::HashMap;

use iced::widget::canvas::{self, Frame, Path, Stroke, fill};
use iced::widget::image;
use iced::{Color, Point, Rectangle, Size};
use kirie_annotate::effects::{self, Region};
use kirie_annotate::{
    Document, Draft, Effect, FillRule, Primitive, Rgba, Shape, ShapeId, Typesetter, edit, style,
};
use kirie_capture::{Frame as Capture, geometry};
use kurbo::{Affine, BezPath, PathEl};

/// Where effects get the pixels under a mark, in the marks' coordinates.
pub trait PixelSource {
    fn pixels(&self, rect: geometry::Rect) -> Option<::image::RgbaImage>;
}

/// Frozen screens, in desktop coordinates.
#[derive(Debug)]
pub struct Captures<'a>(pub &'a [Capture]);

impl PixelSource for Captures<'_> {
    fn pixels(&self, rect: geometry::Rect) -> Option<::image::RgbaImage> {
        geometry::crop(self.0, rect)
    }
}

/// A plain image whose pixels are the marks' coordinates.
#[derive(Debug)]
pub struct ImageSource<'a>(pub &'a ::image::RgbaImage);

impl PixelSource for ImageSource<'_> {
    #[allow(clippy::cast_sign_loss)]
    fn pixels(&self, rect: geometry::Rect) -> Option<::image::RgbaImage> {
        let image = geometry::Rect::new(
            0,
            0,
            i32::try_from(self.0.width()).ok()?,
            i32::try_from(self.0.height()).ok()?,
        );
        let part = image.intersection(&rect)?;
        if part != rect {
            return None;
        }
        Some(
            ::image::imageops::crop_imm(
                self.0,
                part.x as u32,
                part.y as u32,
                part.width as u32,
                part.height as u32,
            )
            .to_image(),
        )
    }
}

/// The pixels under the marks and the things worth keeping between redraws.
pub struct Painter<'a> {
    pub source: &'a dyn PixelSource,
    pub typesetter: &'a RefCell<Typesetter>,
    pub patches: &'a RefCell<HashMap<PatchKey, image::Handle>>,
    /// Image marks' pixels as the renderer holds them, by [`Pixels::id`].
    ///
    /// [`Pixels::id`]: kirie_annotate::Pixels::id
    pub pictures: &'a RefCell<HashMap<u64, image::Handle>>,
}

/// The marks to draw: the document's, changed by the draft in progress, and a new one.
pub struct Marks<'a> {
    pub document: &'a Document,
    pub draft: Option<&'a Draft>,
    /// A mark being drawn or typed: a new one, or one in place of the mark with its id.
    pub preview: Option<Shape>,
    pub picked: &'a [ShapeId],
    /// The image area a spotlight dims around.
    pub visible: kurbo::Rect,
    /// Only the marks of this layer, see [`layers`]; all of them if `None`.
    pub layer: Option<usize>,
}

/// What one of the canvases stacked over a picture does. The input stays the lowest, so it
/// keeps its state as layers come and go.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Takes the pointer, and draws nothing.
    Input,
    /// Draws the marks of one layer, see [`layers`].
    Marks(usize),
    /// Draws what goes over every mark: outlines of the picked marks, a marquee, and the like.
    Chrome,
}

impl Part {
    /// The input, then each layer of `document`'s marks, then the chrome.
    pub fn all(document: &Document) -> impl Iterator<Item = Part> {
        std::iter::once(Part::Input)
            .chain((0..layers(document)).map(Part::Marks))
            .chain(std::iter::once(Part::Chrome))
    }
}

/// How many canvases, one above the other, the marks of `document` take: a screen draws the
/// pixels of a canvas over its paths, so each image or effect ends a layer, and the marks above
/// it go on the next.
pub fn layers(document: &Document) -> usize {
    1 + document
        .shapes()
        .filter(|shape| shape.kind.is_raster())
        .count()
}

/// Patches made for marks that are gone are dropped once there are this many.
const PATCH_LIMIT: usize = 64;

/// An effect over a desktop region, at whole logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PatchKey {
    rect: geometry::Rect,
    kind: u8,
    strength: u32,
}

impl Painter<'_> {
    /// Draws `primitive`, mapped onto the canvas by `transform` (a scale and a translation).
    pub fn draw(&self, frame: &mut Frame, primitive: &Primitive, transform: Affine) {
        let scale = transform.as_coeffs()[0];
        match primitive {
            Primitive::Fill { path, color, rule } => {
                frame.fill(
                    &to_iced(path, transform),
                    canvas::Fill {
                        style: canvas::Style::Solid(color_of(*color)),
                        rule: match rule {
                            FillRule::NonZero => fill::Rule::NonZero,
                            FillRule::EvenOdd => fill::Rule::EvenOdd,
                        },
                    },
                );
            }
            Primitive::Stroke { path, color, width } => {
                frame.stroke(&to_iced(path, transform), stroke(*color, *width * scale));
            }
            Primitive::Cover { path, color } => {
                frame.fill(&to_iced(path, transform), color_of(color.with_alpha(0xff)));
            }
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
                let outline = self
                    .typesetter
                    .borrow_mut()
                    .outline(text, *font, *size, *at, *align, *centered);
                let path = to_iced(&outline, transform);
                if let Some(halo) = halo {
                    frame.stroke(&path, stroke(*halo, size * 0.16 * scale));
                }
                frame.fill(&path, color_of(*color));
            }
            Primitive::Image { rect, pixels } => {
                let handle = self.picture(pixels);
                let mapped = transform.transform_rect_bbox(rect.abs());
                #[allow(clippy::cast_possible_truncation)]
                let local = Rectangle::new(
                    Point::new(mapped.x0 as f32, mapped.y0 as f32),
                    Size::new(mapped.width() as f32, mapped.height() as f32),
                );
                frame.draw_image(
                    local,
                    canvas::Image::new(handle).filter_method(if scale >= 2.0 {
                        image::FilterMethod::Nearest
                    } else {
                        image::FilterMethod::Linear
                    }),
                );
            }
            Primitive::Effect { rect, effect } => {
                if let Some((handle, bounds)) = self.patch(*rect, *effect) {
                    let mapped = transform.transform_rect_bbox(bounds);
                    #[allow(clippy::cast_possible_truncation)]
                    let local = Rectangle::new(
                        Point::new(mapped.x0 as f32, mapped.y0 as f32),
                        Size::new(mapped.width() as f32, mapped.height() as f32),
                    );
                    frame.draw_image(local, &handle);
                }
            }
        }
    }

    /// Draws every mark; returns the picked ones as they are shown, for their outlines.
    pub fn draw_marks(&self, frame: &mut Frame, marks: Marks<'_>, transform: Affine) -> Vec<Shape> {
        let document = marks.document;
        let moving = match marks.draft {
            Some(Draft::Moving { ids, by, .. }) => Some((ids.as_slice(), *by)),
            _ => None,
        };
        let edited = marks.draft.and_then(Draft::edited);
        let preview = marks.preview;
        let replaces = preview
            .as_ref()
            .is_some_and(|preview| document.shape(preview.id).is_some());
        let (in_place, added) = if replaces {
            (preview, None)
        } else {
            (None, preview)
        };
        let shapes: Vec<Shape> = document
            .shapes()
            .map(|shape| match &in_place {
                Some(preview) if preview.id == shape.id => preview.clone(),
                _ => shape.clone(),
            })
            .chain(added)
            .map(|shape| match (moving, edited) {
                (Some((ids, by)), _) if ids.contains(&shape.id) => Shape {
                    kind: shape.kind.translated(by),
                    ..shape
                },
                (_, Some(edited)) if edited.id == shape.id => edited.clone(),
                _ => shape,
            })
            .collect();
        self.forget_pictures(&shapes);
        let stage = style::Stage::new(document, marks.visible, &shapes);
        let mut picked = Vec::new();
        let mut layer = 0;
        for shape in shapes {
            let shown = marks.layer.is_none_or(|only| only == layer);
            if shape.kind.is_raster() {
                layer += 1;
            }
            if marks.picked.contains(&shape.id) {
                picked.push(shape.clone());
            }
            if !shown {
                continue;
            }
            let draft = shape.id == document.next_id();
            for primitive in style::primitives(&shape, document.style, document.unit, &stage) {
                if draft {
                    self.draw_draft(frame, &primitive, transform);
                } else {
                    self.draw(frame, &primitive, transform);
                }
            }
        }
        picked
    }

    /// Draws a mark still being drawn: like `draw`, but an effect shows as a light veil, so a
    /// drag does not blur a new patch of pixels on every move.
    pub fn draw_draft(&self, frame: &mut Frame, primitive: &Primitive, transform: Affine) {
        let Primitive::Effect { rect, .. } = primitive else {
            self.draw(frame, primitive, transform);
            return;
        };
        let mapped = transform.transform_rect_bbox(rect.abs());
        #[allow(clippy::cast_possible_truncation)]
        let area = Path::rectangle(
            Point::new(mapped.x0 as f32, mapped.y0 as f32),
            Size::new(mapped.width() as f32, mapped.height() as f32),
        );
        frame.fill(&area, Color::WHITE.scale_alpha(0.28));
        frame.stroke(
            &area,
            Stroke::default()
                .with_color(Color::WHITE.scale_alpha(0.8))
                .with_width(1.0),
        );
    }

    /// Drops the handles on pictures none of `shapes` shows any more.
    fn forget_pictures(&self, shapes: &[Shape]) {
        let shown: std::collections::HashSet<u64> = shapes
            .iter()
            .filter_map(|shape| match &shape.kind {
                kirie_annotate::Kind::Image { pixels, .. } => Some(pixels.id()),
                _ => None,
            })
            .collect();
        let mut pictures = self.pictures.borrow_mut();
        if pictures.len() > shown.len() {
            pictures.retain(|id, _| shown.contains(id));
        }
    }

    /// The renderer's handle on `pixels`, made once.
    fn picture(&self, pixels: &kirie_annotate::Pixels) -> image::Handle {
        let mut pictures = self.pictures.borrow_mut();
        if let Some(handle) = pictures.get(&pixels.id()) {
            return handle.clone();
        }
        let handle = image::Handle::from_rgba(
            pixels.width(),
            pixels.height(),
            pixels.image().as_raw().clone(),
        );
        pictures.insert(pixels.id(), handle.clone());
        handle
    }

    /// The frozen pixels under `rect` with `effect` applied, and where they go.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn patch(&self, rect: kurbo::Rect, effect: Effect) -> Option<(image::Handle, kurbo::Rect)> {
        let rect = rect.abs();
        let whole = geometry::Rect::new(
            rect.x0.floor() as i32,
            rect.y0.floor() as i32,
            (rect.x1.ceil() - rect.x0.floor()) as i32,
            (rect.y1.ceil() - rect.y0.floor()) as i32,
        );
        let (kind, strength) = match effect {
            Effect::Blur { radius } => (0, radius.to_bits() as u32),
            Effect::Pixelate { cell } => (1, cell.to_bits() as u32),
        };
        let key = PatchKey {
            rect: whole,
            kind,
            strength,
        };
        let bounds = kurbo::Rect::new(
            f64::from(whole.x),
            f64::from(whole.y),
            f64::from(whole.right()),
            f64::from(whole.bottom()),
        );
        if let Some(handle) = self.patches.borrow().get(&key) {
            return Some((handle.clone(), bounds));
        }
        let mut pixels = self.source.pixels(whole)?;
        let scale = f64::from(pixels.width()) / f64::from(whole.width.max(1));
        let region = Region::clamp(
            kurbo::Rect::new(
                0.0,
                0.0,
                f64::from(pixels.width()),
                f64::from(pixels.height()),
            ),
            pixels.width(),
            pixels.height(),
        )?;
        let stride = pixels.width() as usize * 4;
        match effect {
            Effect::Blur { radius } => effects::blur(&mut pixels, stride, region, radius * scale),
            Effect::Pixelate { cell } => {
                effects::pixelate(&mut pixels, stride, region, cell * scale);
            }
        }
        let handle = image::Handle::from_rgba(pixels.width(), pixels.height(), pixels.into_raw());
        let mut patches = self.patches.borrow_mut();
        if patches.len() >= PATCH_LIMIT {
            patches.clear();
        }
        patches.insert(key, handle.clone());
        Some((handle, bounds))
    }
}

/// Outlines the picked marks, `shown` as they are drawn and measured in `document`; a lone pick
/// shows its grips too. `pixel` is one screen pixel in mark coordinates.
pub fn draw_picked(
    frame: &mut Frame,
    shown: &[Shape],
    picked: &[ShapeId],
    document: &Document,
    transform: Affine,
    pixel: f64,
    accent: Color,
) {
    if let ([shape], [_]) = (shown, picked) {
        draw_grips(
            frame,
            shape,
            document.bounds(shape),
            transform,
            pixel,
            accent,
        );
        return;
    }
    for shape in shown {
        draw_outline(frame, shape, document.bounds(shape), transform, accent);
    }
}

/// The box being dragged to pick the marks inside it.
pub fn draw_marquee(frame: &mut Frame, rect: kurbo::Rect, transform: Affine, accent: Color) {
    let rect = transform.transform_rect_bbox(rect);
    #[allow(clippy::cast_possible_truncation)]
    let area = Path::rectangle(
        Point::new(rect.x0 as f32, rect.y0 as f32),
        Size::new(rect.width() as f32, rect.height() as f32),
    );
    frame.fill(&area, accent.scale_alpha(0.12));
    frame.stroke(&area, Stroke::default().with_color(accent).with_width(1.0));
}

fn screen_point(transform: Affine) -> impl Fn(kurbo::Point) -> Point {
    #[allow(clippy::cast_possible_truncation)]
    move |point: kurbo::Point| {
        let point = transform * point;
        Point::new(point.x as f32, point.y as f32)
    }
}

/// A dashed line around a picked mark that covers `bounds`, turned with it.
fn draw_outline(
    frame: &mut Frame,
    shape: &Shape,
    bounds: kurbo::Rect,
    transform: Affine,
    accent: Color,
) {
    let at = screen_point(transform);
    let turn = shape.turn();
    let corners = [
        kurbo::Point::new(bounds.x0, bounds.y0),
        kurbo::Point::new(bounds.x1, bounds.y0),
        kurbo::Point::new(bounds.x1, bounds.y1),
        kurbo::Point::new(bounds.x0, bounds.y1),
    ];
    let outline = Path::new(|path| {
        path.move_to(at(turn * corners[0]));
        for corner in &corners[1..] {
            path.line_to(at(turn * *corner));
        }
        path.close();
    });
    frame.stroke(
        &outline,
        Stroke {
            line_dash: canvas::LineDash {
                segments: &[6.0, 4.0],
                offset: 0,
            },
            ..Stroke::default().with_color(accent).with_width(1.0)
        },
    );
}

/// The outline and grips of a picked mark that covers `bounds`; `pixel` is one screen pixel in
/// mark coordinates.
pub fn draw_grips(
    frame: &mut Frame,
    shape: &Shape,
    bounds: kurbo::Rect,
    transform: Affine,
    pixel: f64,
    accent: Color,
) {
    let at = screen_point(transform);
    let turn = shape.turn();
    draw_outline(frame, shape, bounds, transform, accent);
    for (grip, point) in edit::grips(shape, pixel) {
        let center = at(point);
        if grip == edit::Grip::Rotate {
            let top = at(turn * kurbo::Point::new(bounds.center().x, bounds.y0));
            frame.stroke(
                &Path::line(top, center),
                Stroke::default().with_color(accent).with_width(1.0),
            );
            let knob = Path::circle(center, 5.0);
            frame.fill(&knob, accent);
            frame.stroke(
                &knob,
                Stroke::default().with_color(Color::WHITE).with_width(1.5),
            );
        } else {
            let knob = Path::rectangle(center - iced::Vector::new(4.0, 4.0), Size::new(8.0, 8.0));
            frame.fill(&knob, Color::WHITE);
            frame.stroke(&knob, Stroke::default().with_color(accent).with_width(1.5));
        }
    }
}

fn stroke(color: Rgba, width: f64) -> Stroke<'static> {
    #[allow(clippy::cast_possible_truncation)]
    Stroke::default()
        .with_color(color_of(color))
        .with_width(width as f32)
        .with_line_cap(canvas::LineCap::Round)
        .with_line_join(canvas::LineJoin::Round)
}

pub fn color_of(color: Rgba) -> Color {
    Color::from_rgba8(color.r, color.g, color.b, f32::from(color.a) / 255.0)
}

#[allow(clippy::cast_possible_truncation)]
fn to_iced(path: &BezPath, transform: Affine) -> Path {
    let at = |p: kurbo::Point| {
        let p = transform * p;
        Point::new(p.x as f32, p.y as f32)
    };
    Path::new(|builder| {
        for element in path.elements() {
            match *element {
                PathEl::MoveTo(p) => builder.move_to(at(p)),
                PathEl::LineTo(p) => builder.line_to(at(p)),
                PathEl::QuadTo(c, p) => builder.quadratic_curve_to(at(c), at(p)),
                PathEl::CurveTo(a, b, p) => builder.bezier_curve_to(at(a), at(b), at(p)),
                PathEl::ClosePath => builder.close(),
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_sources_give_whole_regions_only() {
        let image = ::image::RgbaImage::new(10, 8);
        let source = ImageSource(&image);
        assert_eq!(
            source
                .pixels(geometry::Rect::new(2, 2, 4, 3))
                .map(|p| p.dimensions()),
            Some((4, 3))
        );
        assert!(source.pixels(geometry::Rect::new(8, 0, 4, 3)).is_none());
        assert!(source.pixels(geometry::Rect::new(20, 20, 4, 3)).is_none());
    }

    #[test]
    fn colors_keep_their_alpha() {
        let color = color_of(Rgba::rgb(255, 0, 0).with_alpha(51));
        assert!((color.a - 0.2).abs() < 1e-6);
        assert!((color.r - 1.0).abs() < 1e-6);
    }
}
