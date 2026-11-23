//! The pixels of an image mark: shared between the document versions undo keeps, and written
//! into projects and the clipboard as a PNG.

use std::cell::Cell;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use base64::Engine as _;
use image::RgbaImage;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The widest or tallest image a mark may hold, in pixels.
pub const MAX_SIDE: u32 = 16_384;
/// The most pixels an image mark may hold: an 8K screenshot fits, with room to spare.
pub const MAX_PIXELS: u64 = 48_000_000;

static NEXT: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// How many more pixels the images being read may hold together, while [`budget`] runs.
    static LEFT: Cell<Option<u64>> = const { Cell::new(None) };
}

/// Runs `read` with image marks it reads holding `pixels` together at most, so a project or a
/// paste full of small files that unpack into huge images is refused before they unpack.
pub fn budget<T>(pixels: u64, read: impl FnOnce() -> T) -> T {
    /// Puts back the budget there was, even when reading panics.
    struct Restore(Option<u64>);
    impl Drop for Restore {
        fn drop(&mut self) {
            LEFT.set(self.0);
        }
    }
    let _restore = Restore(LEFT.replace(Some(pixels)));
    read()
}

/// Whether `width`×`height` is an image a mark may hold, taking it from the budget if one runs.
fn allowed(width: u32, height: u32) -> Result<(), String> {
    let pixels = u64::from(width) * u64::from(height);
    if width > MAX_SIDE || height > MAX_SIDE || pixels > MAX_PIXELS {
        return Err(format!("an image of {width}×{height} pixels is too large"));
    }
    match LEFT.get() {
        Some(left) if pixels > left => Err("the images hold too many pixels together".to_owned()),
        Some(left) => {
            LEFT.set(Some(left - pixels));
            Ok(())
        }
        None => Ok(()),
    }
}

/// Whether `width`×`height` is an image a mark may hold.
pub fn fits(width: u32, height: u32) -> bool {
    width > 0
        && height > 0
        && width <= MAX_SIDE
        && height <= MAX_SIDE
        && u64::from(width) * u64::from(height) <= MAX_PIXELS
}

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
        use image::ImageDecoder as _;
        use serde::de::Error as _;
        let text = String::deserialize(from)?;
        let png = base64::engine::general_purpose::STANDARD
            .decode(text)
            .map_err(D::Error::custom)?;
        let decoder = image::codecs::png::PngDecoder::new(std::io::Cursor::new(png))
            .map_err(D::Error::custom)?;
        let (width, height) = decoder.dimensions();
        allowed(width, height).map_err(D::Error::custom)?;
        let image = image::DynamicImage::from_decoder(decoder)
            .map_err(D::Error::custom)?
            .to_rgba8();
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
    fn images_too_large_together_are_refused_before_they_unpack() {
        let text = ron::to_string(&Pixels::new(RgbaImage::new(100, 100))).unwrap();
        let two = format!("({text}, {text})");
        let read = |pixels| budget(pixels, || ron::from_str::<(Pixels, Pixels)>(&two));
        assert!(read(20_000).is_ok());
        assert!(read(15_000).is_err());
        assert!(
            ron::from_str::<(Pixels, Pixels)>(&two).is_ok(),
            "no budget outside one"
        );
        let wide = ron::to_string(&Pixels::new(RgbaImage::new(MAX_SIDE + 1, 1))).unwrap();
        assert!(ron::from_str::<Pixels>(&wide).is_err());
        assert!(fits(10, 10) && !fits(0, 10) && !fits(MAX_SIDE, MAX_SIDE));
        let panicked = std::panic::catch_unwind(|| budget(1, || panic!("broken")));
        assert!(panicked.is_err());
        assert!(
            ron::from_str::<(Pixels, Pixels)>(&two).is_ok(),
            "a panic leaves no budget behind"
        );
    }

    #[test]
    fn broken_pixels_are_refused() {
        assert!(ron::from_str::<Pixels>("\"not base64!\"").is_err());
        assert!(ron::from_str::<Pixels>("\"aGVsbG8=\"").is_err());
    }
}
