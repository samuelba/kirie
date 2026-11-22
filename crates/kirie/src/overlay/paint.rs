//! Draws annotation primitives on an iced canvas, matching what `kirie_annotate::render` saves:
//! the same paths, the same glyph outlines, and effects computed from the frozen pixels.

use std::cell::RefCell;
use std::collections::HashMap;

use iced::widget::canvas::{self, Frame, Path, Stroke, fill};
use iced::widget::image;
use iced::{Color, Point, Rectangle, Size};
use kirie_annotate::effects::{self, Region};
use kirie_annotate::{Effect, FillRule, Primitive, Rgba, Typesetter};
use kirie_capture::{Frame as Capture, geometry};
use kurbo::{BezPath, PathEl};

/// The frozen screens and the things worth keeping between redraws.
pub struct Painter<'a> {
    pub captures: &'a [Capture],
    pub typesetter: &'a RefCell<Typesetter>,
    pub patches: &'a RefCell<HashMap<PatchKey, image::Handle>>,
}

/// An effect over a desktop region, at whole logical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PatchKey {
    rect: geometry::Rect,
    kind: u8,
    strength: u32,
}

impl Painter<'_> {
    /// Draws `primitive`, given in desktop coordinates, onto a surface whose top left is
    /// `origin` on the desktop.
    pub fn draw(&self, frame: &mut Frame, primitive: &Primitive, origin: kurbo::Vec2) {
        match primitive {
            Primitive::Fill { path, color, rule } => {
                frame.fill(
                    &to_iced(path, origin),
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
                frame.stroke(&to_iced(path, origin), stroke(*color, *width));
            }
            Primitive::Text {
                at,
                text,
                size,
                color,
                font,
                halo,
                centered,
            } => {
                let outline = self
                    .typesetter
                    .borrow_mut()
                    .outline(text, *font, *size, *at, *centered);
                let path = to_iced(&outline, origin);
                if let Some(halo) = halo {
                    frame.stroke(&path, stroke(*halo, size * 0.16));
                }
                frame.fill(&path, color_of(*color));
            }
            Primitive::Effect { rect, effect } => {
                #[allow(clippy::cast_possible_truncation)]
                if let Some((handle, bounds)) = self.patch(*rect, *effect) {
                    let local = Rectangle {
                        x: bounds.x - origin.x as f32,
                        y: bounds.y - origin.y as f32,
                        ..bounds
                    };
                    frame.draw_image(local, &handle);
                }
            }
        }
    }

    /// The frozen pixels under `rect` with `effect` applied, and where they go.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    fn patch(&self, rect: kurbo::Rect, effect: Effect) -> Option<(image::Handle, Rectangle)> {
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
        #[allow(clippy::cast_precision_loss)]
        let bounds = Rectangle::new(
            Point::new(whole.x as f32, whole.y as f32),
            Size::new(whole.width as f32, whole.height as f32),
        );
        if let Some(handle) = self.patches.borrow().get(&key) {
            return Some((handle.clone(), bounds));
        }
        let mut pixels = geometry::crop(self.captures, whole)?;
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
        self.patches.borrow_mut().insert(key, handle.clone());
        Some((handle, bounds))
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
fn to_iced(path: &BezPath, origin: kurbo::Vec2) -> Path {
    let at = |p: kurbo::Point| Point::new((p.x - origin.x) as f32, (p.y - origin.y) as f32);
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
