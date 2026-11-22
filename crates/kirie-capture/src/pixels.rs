//! The `wl_shm` pixel formats Kirie reads, turned into RGBA.

use image::{Rgba, RgbaImage};

/// A `wl_shm` format Kirie can read. Values are the `wl_shm.format` codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShmFormat {
    /// 32 bits per pixel, bytes B G R A in memory.
    Argb8888,
    /// As `Argb8888` with an undefined alpha byte.
    Xrgb8888,
    /// 32 bits per pixel, bytes R G B A in memory.
    Abgr8888,
    /// As `Abgr8888` with an undefined alpha byte.
    Xbgr8888,
}

impl ShmFormat {
    const ABGR8888: u32 = u32::from_le_bytes(*b"AB24");
    const XBGR8888: u32 = u32::from_le_bytes(*b"XB24");

    pub const fn from_wayland(value: u32) -> Option<Self> {
        match value {
            0 => Some(Self::Argb8888),
            1 => Some(Self::Xrgb8888),
            Self::ABGR8888 => Some(Self::Abgr8888),
            Self::XBGR8888 => Some(Self::Xbgr8888),
            _ => None,
        }
    }

    pub const fn to_wayland(self) -> u32 {
        match self {
            Self::Argb8888 => 0,
            Self::Xrgb8888 => 1,
            Self::Abgr8888 => Self::ABGR8888,
            Self::Xbgr8888 => Self::XBGR8888,
        }
    }

    /// Opaque formats first: a screen has no transparency, and their alpha is meaningless.
    const fn preference(self) -> u8 {
        match self {
            Self::Xbgr8888 => 0,
            Self::Xrgb8888 => 1,
            Self::Abgr8888 => 2,
            Self::Argb8888 => 3,
        }
    }

    /// The best of the formats the compositor offers, if Kirie can read any.
    pub fn pick(offered: impl IntoIterator<Item = u32>) -> Option<Self> {
        offered
            .into_iter()
            .filter_map(Self::from_wayland)
            .min_by_key(|format| format.preference())
    }

    const fn swaps_red_and_blue(self) -> bool {
        matches!(self, Self::Argb8888 | Self::Xrgb8888)
    }

    const fn has_alpha(self) -> bool {
        matches!(self, Self::Argb8888 | Self::Abgr8888)
    }
}

/// Why a buffer could not be read.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum PixelError {
    #[error("a {width}×{height} buffer with stride {stride} needs {needed} bytes, got {got}")]
    TooShort {
        width: u32,
        height: u32,
        stride: u32,
        needed: usize,
        got: usize,
    },
    #[error("stride {stride} is too small for {width} pixels")]
    Stride { width: u32, stride: u32 },
}

/// Converts a 32-bit buffer to RGBA. `y_invert` flips it upside down, as wlr-screencopy asks.
pub fn to_rgba(
    format: ShmFormat,
    width: u32,
    height: u32,
    stride: u32,
    bytes: &[u8],
    y_invert: bool,
) -> Result<RgbaImage, PixelError> {
    check(width, height, stride, bytes.len())?;
    let row = width as usize * 4;
    let mut out = Vec::with_capacity(row * height as usize);
    for y in 0..height as usize {
        let source = if y_invert { height as usize - 1 - y } else { y };
        let start = source * stride as usize;
        for pixel in bytes[start..start + row].as_chunks::<4>().0 {
            let (r, b) = if format.swaps_red_and_blue() {
                (pixel[2], pixel[0])
            } else {
                (pixel[0], pixel[2])
            };
            let a = if format.has_alpha() { pixel[3] } else { 255 };
            out.extend_from_slice(&[r, pixel[1], b, a]);
        }
    }
    Ok(RgbaImage::from_raw(width, height, out).expect("the buffer has width × height pixels"))
}

/// Whether `len` bytes hold a `width`×`height` buffer with rows `stride` bytes apart.
fn check(width: u32, height: u32, stride: u32, len: usize) -> Result<(), PixelError> {
    let row = width as usize * 4;
    if (stride as usize) < row {
        return Err(PixelError::Stride { width, stride });
    }
    let needed = if height == 0 {
        0
    } else {
        stride as usize * (height as usize - 1) + row
    };
    if len < needed {
        return Err(PixelError::TooShort {
            width,
            height,
            stride,
            needed,
            got: len,
        });
    }
    Ok(())
}

/// A 32-bit buffer read in place, a pixel at a time, for when only a few of its pixels matter.
#[derive(Debug, Clone, Copy)]
pub struct View<'a> {
    format: ShmFormat,
    width: u32,
    height: u32,
    stride: u32,
    bytes: &'a [u8],
    y_invert: bool,
}

impl<'a> View<'a> {
    pub fn new(
        format: ShmFormat,
        width: u32,
        height: u32,
        stride: u32,
        bytes: &'a [u8],
        y_invert: bool,
    ) -> Result<Self, PixelError> {
        check(width, height, stride, bytes.len())?;
        Ok(Self {
            format,
            width,
            height,
            stride,
            bytes,
            y_invert,
        })
    }

    pub const fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// The pixel at (`x`, `y`) as RGBA, as [`to_rgba`] would convert it.
    pub fn get(&self, x: u32, y: u32) -> Option<Rgba<u8>> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let row = if self.y_invert {
            self.height - 1 - y
        } else {
            y
        };
        let start = row as usize * self.stride as usize + x as usize * 4;
        let pixel = self.bytes.get(start..start + 4)?;
        let (red, blue) = if self.format.swaps_red_and_blue() {
            (pixel[2], pixel[0])
        } else {
            (pixel[0], pixel[2])
        };
        let alpha = if self.format.has_alpha() {
            pixel[3]
        } else {
            255
        };
        Some(Rgba([red, pixel[1], blue, alpha]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_round_trip_through_wayland_codes() {
        for format in [
            ShmFormat::Argb8888,
            ShmFormat::Xrgb8888,
            ShmFormat::Abgr8888,
            ShmFormat::Xbgr8888,
        ] {
            assert_eq!(ShmFormat::from_wayland(format.to_wayland()), Some(format));
        }
        assert_eq!(ShmFormat::from_wayland(0x3231_5258), None);
    }

    #[test]
    fn picks_an_opaque_format_first() {
        assert_eq!(ShmFormat::pick([0, 1, 0x1234]), Some(ShmFormat::Xrgb8888));
        assert_eq!(
            ShmFormat::pick([
                ShmFormat::Abgr8888.to_wayland(),
                ShmFormat::Xbgr8888.to_wayland()
            ]),
            Some(ShmFormat::Xbgr8888)
        );
        assert_eq!(ShmFormat::pick([0x1234]), None);
    }

    #[test]
    fn xrgb_swaps_channels_and_ignores_alpha() {
        let bytes = [10, 20, 30, 0, 40, 50, 60, 0];
        let image = to_rgba(ShmFormat::Xrgb8888, 2, 1, 8, &bytes, false).unwrap();
        assert_eq!(image.get_pixel(0, 0), &Rgba([30, 20, 10, 255]));
        assert_eq!(image.get_pixel(1, 0), &Rgba([60, 50, 40, 255]));
    }

    #[test]
    fn abgr_keeps_channels_and_alpha() {
        let bytes = [10, 20, 30, 128];
        let image = to_rgba(ShmFormat::Abgr8888, 1, 1, 4, &bytes, false).unwrap();
        assert_eq!(image.get_pixel(0, 0), &Rgba([10, 20, 30, 128]));
    }

    #[test]
    fn stride_padding_and_y_invert() {
        let bytes = [1, 1, 1, 1, 9, 9, 2, 2, 2, 2, 9, 9];
        let image = to_rgba(ShmFormat::Xbgr8888, 1, 2, 6, &bytes, true).unwrap();
        assert_eq!(image.get_pixel(0, 0), &Rgba([2, 2, 2, 255]));
        assert_eq!(image.get_pixel(0, 1), &Rgba([1, 1, 1, 255]));
    }

    #[test]
    fn short_buffers_and_strides_are_errors() {
        assert!(matches!(
            to_rgba(ShmFormat::Argb8888, 2, 2, 8, &[0; 10], false),
            Err(PixelError::TooShort { needed: 16, .. })
        ));
        assert_eq!(
            to_rgba(ShmFormat::Argb8888, 2, 1, 4, &[0; 8], false),
            Err(PixelError::Stride {
                width: 2,
                stride: 4
            })
        );
        assert_eq!(
            to_rgba(ShmFormat::Argb8888, 0, 0, 0, &[], false)
                .unwrap()
                .dimensions(),
            (0, 0)
        );
    }

    #[test]
    fn views_read_the_pixels_to_rgba_converts() {
        let bytes = [1, 2, 3, 4, 9, 9, 5, 6, 7, 8, 9, 9];
        for format in [
            ShmFormat::Argb8888,
            ShmFormat::Xrgb8888,
            ShmFormat::Abgr8888,
            ShmFormat::Xbgr8888,
        ] {
            for y_invert in [false, true] {
                let image = to_rgba(format, 1, 2, 6, &bytes, y_invert).unwrap();
                let view = View::new(format, 1, 2, 6, &bytes, y_invert).unwrap();
                assert_eq!(view.get(0, 0).as_ref(), Some(image.get_pixel(0, 0)));
                assert_eq!(view.get(0, 1).as_ref(), Some(image.get_pixel(0, 1)));
                assert_eq!(view.get(1, 0), None);
                assert_eq!(view.get(0, 2), None);
            }
        }
        assert!(View::new(ShmFormat::Argb8888, 2, 2, 8, &[0; 10], false).is_err());
    }
}
