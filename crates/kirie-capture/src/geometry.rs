//! Screens in the compositor's logical coordinate space and the pixels captured from them.

use image::{RgbaImage, imageops};

/// A rectangle in logical (scale-independent) desktop coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

impl Rect {
    pub const fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    pub const fn right(&self) -> i32 {
        self.x + self.width
    }

    pub const fn bottom(&self) -> i32 {
        self.y + self.height
    }

    pub const fn is_empty(&self) -> bool {
        self.width <= 0 || self.height <= 0
    }

    pub fn intersection(&self, other: &Rect) -> Option<Rect> {
        let x = self.x.max(other.x);
        let y = self.y.max(other.y);
        let rect = Rect::new(
            x,
            y,
            self.right().min(other.right()) - x,
            self.bottom().min(other.bottom()) - y,
        );
        (!rect.is_empty()).then_some(rect)
    }

    #[must_use]
    pub fn union(&self, other: &Rect) -> Rect {
        let x = self.x.min(other.x);
        let y = self.y.min(other.y);
        Rect::new(
            x,
            y,
            self.right().max(other.right()) - x,
            self.bottom().max(other.bottom()) - y,
        )
    }

    pub const fn contains(&self, x: i32, y: i32) -> bool {
        x >= self.x && x < self.right() && y >= self.y && y < self.bottom()
    }
}

/// How an output's buffer is rotated or flipped relative to how it is shown, as in `wl_output`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Transform {
    #[default]
    Normal,
    Rotate90,
    Rotate180,
    Rotate270,
    Flipped,
    Flipped90,
    Flipped180,
    Flipped270,
}

impl Transform {
    /// The transform for a `wl_output.transform` value; unknown values count as normal.
    pub const fn from_wayland(value: u32) -> Self {
        match value {
            1 => Self::Rotate90,
            2 => Self::Rotate180,
            3 => Self::Rotate270,
            4 => Self::Flipped,
            5 => Self::Flipped90,
            6 => Self::Flipped180,
            7 => Self::Flipped270,
            _ => Self::Normal,
        }
    }

    /// Turns a buffer captured in the output's own orientation into the image as it is shown.
    /// `wl_output.transform` rotates counter-clockwise, so showing it rotates clockwise.
    pub fn apply(self, image: &RgbaImage) -> RgbaImage {
        let flipped = || imageops::flip_horizontal(image);
        match self {
            Self::Normal => image.clone(),
            Self::Rotate90 => imageops::rotate270(image),
            Self::Rotate180 => imageops::rotate180(image),
            Self::Rotate270 => imageops::rotate90(image),
            Self::Flipped => flipped(),
            Self::Flipped90 => imageops::rotate270(&flipped()),
            Self::Flipped180 => imageops::rotate180(&flipped()),
            Self::Flipped270 => imageops::rotate90(&flipped()),
        }
    }
}

/// A screen as the compositor lays it out.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Output {
    /// The connector name, such as `DP-1`.
    pub name: String,
    pub description: String,
    pub logical: Rect,
    pub transform: Transform,
}

/// The pixels of one screen at its native resolution.
#[derive(Debug, Clone)]
pub struct Frame {
    pub output: Output,
    pub image: RgbaImage,
}

impl Frame {
    /// Physical pixels per logical pixel; fractional on fractionally scaled screens.
    pub fn scale(&self) -> f64 {
        if self.output.logical.width <= 0 {
            return 1.0;
        }
        f64::from(self.image.width()) / f64::from(self.output.logical.width)
    }
}

/// The area every frame covers together.
pub fn bounds(frames: &[Frame]) -> Option<Rect> {
    frames
        .iter()
        .map(|frame| frame.output.logical)
        .reduce(|all, rect| all.union(&rect))
}

/// Cuts `region` out of the frames, at the highest scale among the screens it touches, so a
/// region on a high-density screen keeps its detail. Parts of the region that no screen covers stay
/// transparent.
#[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
pub fn crop(frames: &[Frame], region: Rect) -> Option<RgbaImage> {
    if region.is_empty() {
        return None;
    }
    let touched: Vec<&Frame> = frames
        .iter()
        .filter(|frame| frame.output.logical.intersection(&region).is_some())
        .collect();
    let scale = touched.iter().map(|frame| frame.scale()).reduce(f64::max)?;
    let to_px = |value: i32| (f64::from(value) * scale).round() as u32;
    let mut out = RgbaImage::new(to_px(region.width), to_px(region.height));
    for frame in touched {
        let Some(part) = frame.output.logical.intersection(&region) else {
            continue;
        };
        let source_scale = frame.scale();
        let source = |value: i32| (f64::from(value) * source_scale).round() as u32;
        let (sx, sy) = (
            source(part.x - frame.output.logical.x),
            source(part.y - frame.output.logical.y),
        );
        let (sw, sh) = (
            source(part.width).min(frame.image.width().saturating_sub(sx)),
            source(part.height).min(frame.image.height().saturating_sub(sy)),
        );
        if sw == 0 || sh == 0 {
            continue;
        }
        let piece = imageops::crop_imm(&frame.image, sx, sy, sw, sh).to_image();
        let (tw, th) = (to_px(part.width), to_px(part.height));
        let piece = if (tw, th) == (sw, sh) {
            piece
        } else {
            imageops::resize(&piece, tw, th, imageops::FilterType::Lanczos3)
        };
        imageops::replace(
            &mut out,
            &piece,
            i64::from(to_px(part.x - region.x)),
            i64::from(to_px(part.y - region.y)),
        );
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn solid(width: u32, height: u32, color: [u8; 4]) -> RgbaImage {
        RgbaImage::from_pixel(width, height, Rgba(color))
    }

    fn frame(name: &str, logical: Rect, image: RgbaImage) -> Frame {
        Frame {
            output: Output {
                name: name.into(),
                logical,
                ..Output::default()
            },
            image,
        }
    }

    #[test]
    fn rect_intersection_and_union() {
        let a = Rect::new(0, 0, 100, 50);
        let b = Rect::new(80, 40, 50, 50);
        assert_eq!(a.intersection(&b), Some(Rect::new(80, 40, 20, 10)));
        assert_eq!(a.union(&b), Rect::new(0, 0, 130, 90));
        assert_eq!(a.intersection(&Rect::new(100, 0, 10, 10)), None);
        assert!(a.contains(99, 49));
        assert!(!a.contains(100, 0));
    }

    #[test]
    fn transform_from_wayland_values() {
        assert_eq!(Transform::from_wayland(0), Transform::Normal);
        assert_eq!(Transform::from_wayland(1), Transform::Rotate90);
        assert_eq!(Transform::from_wayland(7), Transform::Flipped270);
        assert_eq!(Transform::from_wayland(42), Transform::Normal);
    }

    #[test]
    fn rotated_outputs_come_out_upright() {
        let red = Rgba([255, 0, 0, 255]);
        let mut buffer = solid(2, 1, [0, 0, 0, 255]);
        buffer.put_pixel(0, 0, red);
        let shown = Transform::Rotate90.apply(&buffer);
        assert_eq!(shown.dimensions(), (1, 2));
        assert_eq!(shown.get_pixel(0, 1), &red);
        assert_eq!(Transform::Rotate270.apply(&buffer).get_pixel(0, 0), &red);
        assert_eq!(Transform::Flipped.apply(&buffer).get_pixel(1, 0), &red);
        assert_eq!(Transform::Rotate180.apply(&buffer).get_pixel(1, 0), &red);
        assert_eq!(Transform::Normal.apply(&buffer), buffer);
        assert_eq!(Transform::Flipped180.apply(&buffer).get_pixel(0, 0), &red);
        assert_eq!(Transform::Flipped90.apply(&buffer).dimensions(), (1, 2));
        assert_eq!(Transform::Flipped270.apply(&buffer).dimensions(), (1, 2));
    }

    #[test]
    fn scale_follows_the_captured_pixels() {
        let hidpi = frame("A", Rect::new(0, 0, 100, 50), solid(150, 75, [0; 4]));
        assert!((hidpi.scale() - 1.5).abs() < f64::EPSILON);
        let broken = frame("B", Rect::new(0, 0, 0, 0), solid(1, 1, [0; 4]));
        assert!((broken.scale() - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn crop_within_one_screen() {
        let mut image = solid(200, 100, [0, 0, 255, 255]);
        image.put_pixel(20, 10, Rgba([255, 0, 0, 255]));
        let frames = [frame("A", Rect::new(0, 0, 100, 50), image)];
        let out = crop(&frames, Rect::new(10, 5, 10, 10)).unwrap();
        assert_eq!(out.dimensions(), (20, 20));
        assert_eq!(out.get_pixel(0, 0), &Rgba([255, 0, 0, 255]));
    }

    #[test]
    fn crop_across_screens_uses_the_highest_scale() {
        let frames = [
            frame(
                "left",
                Rect::new(0, 0, 100, 100),
                solid(100, 100, [255, 0, 0, 255]),
            ),
            frame(
                "right",
                Rect::new(100, 0, 100, 100),
                solid(200, 200, [0, 255, 0, 255]),
            ),
        ];
        assert_eq!(bounds(&frames), Some(Rect::new(0, 0, 200, 100)));
        let out = crop(&frames, Rect::new(90, 0, 20, 10)).unwrap();
        assert_eq!(out.dimensions(), (40, 20));
        assert_eq!(out.get_pixel(5, 5), &Rgba([255, 0, 0, 255]));
        assert_eq!(out.get_pixel(35, 5), &Rgba([0, 255, 0, 255]));
    }

    #[test]
    fn crop_outside_every_screen_is_nothing() {
        let frames = [frame("A", Rect::new(0, 0, 10, 10), solid(10, 10, [0; 4]))];
        assert!(crop(&frames, Rect::new(50, 50, 5, 5)).is_none());
        assert!(crop(&frames, Rect::new(0, 0, 0, 5)).is_none());
        assert!(bounds(&[]).is_none());
    }
}
