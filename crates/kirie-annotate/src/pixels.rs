//! The pixels of an image mark: shared between the document versions undo keeps, and written
//! into projects and the clipboard as a PNG.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use image::RgbaImage;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The widest or tallest image a mark may hold, in pixels.
pub const MAX_SIDE: u32 = 16_384;

static NEXT: AtomicU64 = AtomicU64::new(1);

#[derive(Clone)]
pub struct Pixels {
    image: Arc<RgbaImage>,
    /// Tells these pixels apart from any others for as long as Kirie runs, so a painter can keep
    /// what it made of them.
    id: u64,
}

impl Pixels {
    pub fn new(image: RgbaImage) -> Self {
        Self {
            image: Arc::new(image),
            id: NEXT.fetch_add(1, Ordering::Relaxed),
        }
    }

    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    pub fn id(&self) -> u64 {
        self.id
    }

    pub fn width(&self) -> u32 {
        self.image.width()
    }

    pub fn height(&self) -> u32 {
        self.image.height()
    }
}

impl PartialEq for Pixels {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.image, &other.image) || self.image == other.image
    }
}

impl std::fmt::Debug for Pixels {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Pixels({}×{})", self.width(), self.height())
    }
}

impl Serialize for Pixels {
    fn serialize<S: Serializer>(&self, to: S) -> Result<S::Ok, S::Error> {
        let mut png = Vec::new();
        self.image
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .map_err(serde::ser::Error::custom)?;
        to.serialize_str(&base64::engine::general_purpose::STANDARD.encode(png))
    }
}

impl<'de> Deserialize<'de> for Pixels {
    fn deserialize<D: Deserializer<'de>>(from: D) -> Result<Self, D::Error> {
        use serde::de::Error as _;
        let text = String::deserialize(from)?;
        let png = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(D::Error::custom)?;
        let mut reader =
            image::ImageReader::with_format(std::io::Cursor::new(png), image::ImageFormat::Png);
        let mut limits = image::Limits::default();
        limits.max_image_width = Some(MAX_SIDE);
        limits.max_image_height = Some(MAX_SIDE);
        reader.limits(limits);
        let image = reader.decode().map_err(D::Error::custom)?.to_rgba8();
        Ok(Self::new(image))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pixels_survive_serialization_and_compare_by_content() {
        #[allow(clippy::cast_possible_truncation)]
        let image = RgbaImage::from_fn(3, 2, |x, y| image::Rgba([x as u8 * 80, y as u8, 7, 200]));
        let pixels = Pixels::new(image.clone());
        let text = ron::to_string(&pixels).unwrap();
        let back: Pixels = ron::from_str(&text).unwrap();
        assert_eq!(back.image(), &image);
        assert_eq!(back, pixels);
        assert_ne!(back.id(), pixels.id());
        assert_ne!(Pixels::new(RgbaImage::new(3, 2)), pixels);
        assert_eq!(format!("{pixels:?}"), "Pixels(3×2)");
    }

    #[test]
    fn broken_pixels_are_refused() {
        assert!(ron::from_str::<Pixels>("\"not base64!\"").is_err());
        assert!(ron::from_str::<Pixels>("\"aGVsbG8=\"").is_err());
    }
}
