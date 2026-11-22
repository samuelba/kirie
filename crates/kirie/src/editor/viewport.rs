//! How the image sits in the editor's canvas: fitted or zoomed, and panned. What is shown, the
//! content, is a rectangle of the image: what a crop keeps, or all of it.

use iced::{Point, Size, Vector};
use kurbo::{Affine, Rect};

/// Room kept around a fitted image.
const MARGIN: f32 = 32.0;
const MIN_ZOOM: f32 = 0.05;
const MAX_ZOOM: f32 = 32.0;

#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Viewport {
    /// `None` fits the image into the canvas.
    zoom: Option<f32>,
    pan: Vector,
}

impl Viewport {
    pub fn fitted() -> Self {
        Self::default()
    }

    pub fn is_fitted(&self) -> bool {
        self.zoom.is_none()
    }

    /// Canvas pixels per image pixel.
    pub fn scale(&self, content: Rect, canvas: Size) -> f32 {
        self.zoom.unwrap_or_else(|| fit(size(content), canvas))
    }

    /// Where the image's top left sits in the canvas.
    fn offset(&self, content: Rect, canvas: Size) -> Vector {
        centered(content, self.scale(content, canvas), canvas) + self.pan
    }

    /// Image coordinates to canvas coordinates.
    pub fn transform(&self, content: Rect, canvas: Size) -> Affine {
        let scale = f64::from(self.scale(content, canvas));
        let offset = self.offset(content, canvas);
        Affine::scale(scale)
            .then_translate(kurbo::Vec2::new(f64::from(offset.x), f64::from(offset.y)))
    }

    pub fn image_point(&self, at: Point, content: Rect, canvas: Size) -> kurbo::Point {
        self.transform(content, canvas).inverse()
            * kurbo::Point::new(f64::from(at.x), f64::from(at.y))
    }

    #[must_use]
    pub fn panned(self, by: Vector) -> Self {
        Self {
            pan: self.pan + by,
            ..self
        }
    }

    /// Zooms by `factor`, keeping the image point under `anchor` where it is.
    #[must_use]
    pub fn zoomed(self, factor: f32, anchor: Point, content: Rect, canvas: Size) -> Self {
        let old = self.scale(content, canvas);
        let new = (old * factor).clamp(MIN_ZOOM, MAX_ZOOM);
        let offset = self.offset(content, canvas);
        let under = Vector::new((anchor.x - offset.x) / old, (anchor.y - offset.y) / old);
        let wanted = Vector::new(anchor.x - under.x * new, anchor.y - under.y * new);
        Self {
            zoom: Some(new),
            pan: wanted - centered(content, new, canvas),
        }
    }

    /// Shows image pixels one to one, centered.
    pub fn actual_size() -> Self {
        Self {
            zoom: Some(1.0),
            pan: Vector::ZERO,
        }
    }
}

/// Where the image's top left goes for `content` at `scale` to sit in the middle of `canvas`.
#[allow(clippy::cast_possible_truncation)]
fn centered(content: Rect, scale: f32, canvas: Size) -> Vector {
    let shown = size(content);
    Vector::new(
        (canvas.width - shown.width * scale) / 2.0 - content.x0 as f32 * scale,
        (canvas.height - shown.height * scale) / 2.0 - content.y0 as f32 * scale,
    )
}

#[allow(clippy::cast_possible_truncation)]
fn size(content: Rect) -> Size {
    Size::new(content.width() as f32, content.height() as f32)
}

fn fit(image: Size, canvas: Size) -> f32 {
    if image.width <= 0.0 || image.height <= 0.0 {
        return 1.0;
    }
    let room = Size::new(
        (canvas.width - MARGIN * 2.0).max(1.0),
        (canvas.height - MARGIN * 2.0).max(1.0),
    );
    (room.width / image.width)
        .min(room.height / image.height)
        .min(1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    const IMAGE: Rect = Rect::new(0.0, 0.0, 2000.0, 1000.0);
    const CANVAS: Size = Size::new(1064.0, 800.0);

    #[test]
    fn fitting_shrinks_but_never_enlarges() {
        let viewport = Viewport::fitted();
        assert!((viewport.scale(IMAGE, CANVAS) - 0.5).abs() < 1e-6);
        assert!((viewport.scale(Rect::new(0.0, 0.0, 10.0, 10.0), CANVAS) - 1.0).abs() < 1e-6);
        assert!((viewport.scale(Rect::ZERO, CANVAS) - 1.0).abs() < 1e-6);
        assert!(viewport.is_fitted());
    }

    #[test]
    fn fitted_images_are_centered() {
        let center = Viewport::fitted().transform(IMAGE, CANVAS) * kurbo::Point::new(1000.0, 500.0);
        assert!((center.x - 532.0).abs() < 1e-3 && (center.y - 400.0).abs() < 1e-3);
    }

    #[test]
    fn a_crop_is_fitted_and_centered_on_its_own() {
        let crop = Rect::new(1000.0, 200.0, 1400.0, 400.0);
        let viewport = Viewport::fitted();
        assert!((viewport.scale(crop, CANVAS) - 1.0).abs() < 1e-6);
        let middle = viewport.transform(crop, CANVAS) * crop.center();
        assert!((middle.x - 532.0).abs() < 1e-3 && (middle.y - 400.0).abs() < 1e-3);
        let anchor = Point::new(100.0, 50.0);
        let before = viewport.image_point(anchor, crop, CANVAS);
        let after = viewport
            .zoomed(3.0, anchor, crop, CANVAS)
            .image_point(anchor, crop, CANVAS);
        assert!((before.x - after.x).abs() < 1e-2 && (before.y - after.y).abs() < 1e-2);
    }

    #[test]
    fn canvas_points_map_back_to_the_image() {
        let viewport = Viewport::fitted().panned(Vector::new(10.0, -5.0));
        let canvas = viewport.transform(IMAGE, CANVAS) * kurbo::Point::new(300.0, 200.0);
        #[allow(clippy::cast_possible_truncation)]
        let back =
            viewport.image_point(Point::new(canvas.x as f32, canvas.y as f32), IMAGE, CANVAS);
        assert!((back.x - 300.0).abs() < 1e-2 && (back.y - 200.0).abs() < 1e-2);
    }

    #[test]
    fn zooming_keeps_the_point_under_the_pointer() {
        let anchor = Point::new(200.0, 300.0);
        let before = Viewport::fitted().image_point(anchor, IMAGE, CANVAS);
        let zoomed = Viewport::fitted().zoomed(2.0, anchor, IMAGE, CANVAS);
        assert!((zoomed.scale(IMAGE, CANVAS) - 1.0).abs() < 1e-6);
        assert!(!zoomed.is_fitted());
        let after = zoomed.image_point(anchor, IMAGE, CANVAS);
        assert!((before.x - after.x).abs() < 1e-2 && (before.y - after.y).abs() < 1e-2);
    }

    #[test]
    fn zoom_is_bounded() {
        let tiny = Viewport::fitted().zoomed(0.0001, Point::ORIGIN, IMAGE, CANVAS);
        assert!((tiny.scale(IMAGE, CANVAS) - MIN_ZOOM).abs() < 1e-6);
        let huge = Viewport::actual_size().zoomed(1000.0, Point::ORIGIN, IMAGE, CANVAS);
        assert!((huge.scale(IMAGE, CANVAS) - MAX_ZOOM).abs() < 1e-6);
    }
}
