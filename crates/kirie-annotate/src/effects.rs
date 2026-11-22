//! Pixel effects on a region of a premultiplied RGBA buffer.

/// A region of whole pixels inside a buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region {
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
}

impl Region {
    /// The pixels of `rect` that lie inside a `width`×`height` buffer, if any.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    pub fn clamp(rect: kurbo::Rect, width: u32, height: u32) -> Option<Region> {
        let rect = rect.abs();
        let x0 = rect.x0.floor().clamp(0.0, f64::from(width)) as usize;
        let y0 = rect.y0.floor().clamp(0.0, f64::from(height)) as usize;
        let x1 = rect.x1.ceil().clamp(0.0, f64::from(width)) as usize;
        let y1 = rect.y1.ceil().clamp(0.0, f64::from(height)) as usize;
        (x1 > x0 && y1 > y0).then_some(Region {
            x: x0,
            y: y0,
            width: x1 - x0,
            height: y1 - y0,
        })
    }
}

/// How far, in color levels, each block of a mosaic is randomly brightened or darkened.
const SHIFT: i32 = 12;

/// Blurs the region with three box blurs, close to a Gaussian of `radius`, over a mosaic of
/// cells as wide as `radius`: nothing finer than a cell survives to be sharpened back.
pub fn blur(pixels: &mut [u8], stride: usize, region: Region, radius: f64) {
    pixelate(pixels, stride, region, radius);
    soften(pixels, stride, region, radius);
}

/// Softens the region with three box blurs, close to a Gaussian of `radius`.
pub fn soften(pixels: &mut [u8], stride: usize, region: Region, radius: f64) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let box_radius = ((radius.max(1.0) * 0.6).round() as usize).max(1);
    let mut work = extract(pixels, stride, region);
    let mut scratch = work.clone();
    for _ in 0..3 {
        box_pass(
            &work,
            &mut scratch,
            region.width,
            region.height,
            box_radius,
            true,
        );
        box_pass(
            &scratch,
            &mut work,
            region.width,
            region.height,
            box_radius,
            false,
        );
    }
    insert(pixels, stride, region, &work);
}

/// Replaces the region with blocks of about `cell` pixels, each the average of its pixels,
/// randomly brightened or darkened a little.
///
/// The blocks split the region evenly, so none is a thin strip that keeps its own pixels, and
/// the random shift keeps guesses of what was there from matching the blocks exactly.
pub fn pixelate(pixels: &mut [u8], stride: usize, region: Region, cell: f64) {
    if region.width == 0 || region.height == 0 {
        return;
    }
    let mut noise = Noise::new();
    let columns = cuts(region.width, cell);
    let rows = cuts(region.height, cell);
    for rows in rows.windows(2) {
        let (y0, y1) = (region.y + rows[0], region.y + rows[1]);
        for columns in columns.windows(2) {
            let (x0, x1) = (region.x + columns[0], region.x + columns[1]);
            let mut sum = [0u64; 4];
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = y * stride + x * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u64::from(pixels[at + channel]);
                    }
                }
            }
            let count = ((x1 - x0) * (y1 - y0)) as u64;
            #[allow(clippy::cast_possible_truncation)]
            let average = sum.map(|total| ((total + count / 2) / count) as u8);
            let block = shifted(average, noise.shift());
            for y in y0..y1 {
                for x in x0..x1 {
                    let at = y * stride + x * 4;
                    pixels[at..at + 4].copy_from_slice(&block);
                }
            }
        }
    }
}

/// Where blocks of about `cell` pixels split `length` evenly, from 0 to `length`.
fn cuts(length: usize, cell: f64) -> Vec<usize> {
    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        clippy::cast_precision_loss
    )]
    let blocks = ((length as f64 / cell.max(2.0)).round() as usize).clamp(1, length.max(1));
    (0..=blocks).map(|block| block * length / blocks).collect()
}

/// A premultiplied `pixel` with `shift` added to its color, kept within its alpha.
fn shifted(pixel: [u8; 4], shift: i32) -> [u8; 4] {
    let alpha = i32::from(pixel[3]);
    let mut out = pixel;
    for channel in &mut out[..3] {
        #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
        {
            *channel = (i32::from(*channel) + shift).clamp(0, alpha) as u8;
        }
    }
    out
}

/// Random shifts no one can replay: seeded afresh from the system for every effect.
struct Noise(u64);

impl Noise {
    fn new() -> Self {
        use std::hash::BuildHasher as _;
        Self(std::hash::RandomState::new().hash_one(std::time::Instant::now()))
    }

    /// A shift in `-SHIFT..=SHIFT`.
    fn shift(&mut self) -> i32 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^= z >> 31;
        let span = u64::from(SHIFT.unsigned_abs() * 2 + 1);
        #[allow(clippy::cast_possible_truncation, clippy::cast_possible_wrap)]
        let drawn = (z % span) as i32;
        drawn - SHIFT
    }
}

fn extract(pixels: &[u8], stride: usize, region: Region) -> Vec<u8> {
    let mut out = Vec::with_capacity(region.width * region.height * 4);
    for y in region.y..region.y + region.height {
        let start = y * stride + region.x * 4;
        out.extend_from_slice(&pixels[start..start + region.width * 4]);
    }
    out
}

fn insert(pixels: &mut [u8], stride: usize, region: Region, work: &[u8]) {
    for row in 0..region.height {
        let start = (region.y + row) * stride + region.x * 4;
        let source = row * region.width * 4;
        pixels[start..start + region.width * 4]
            .copy_from_slice(&work[source..source + region.width * 4]);
    }
}

/// One horizontal or vertical box blur pass, with edges clamped.
fn box_pass(
    source: &[u8],
    target: &mut [u8],
    width: usize,
    height: usize,
    radius: usize,
    horizontal: bool,
) {
    let (lines, length) = if horizontal {
        (height, width)
    } else {
        (width, height)
    };
    let index = |line: usize, along: usize| {
        if horizontal {
            (line * width + along) * 4
        } else {
            (along * width + line) * 4
        }
    };
    let window = u32::try_from(radius * 2 + 1).unwrap_or(u32::MAX);
    for line in 0..lines {
        let mut sum = [0u32; 4];
        for offset in 0..window as usize {
            let along = offset.saturating_sub(radius).min(length - 1);
            for (channel, total) in sum.iter_mut().enumerate() {
                *total += u32::from(source[index(line, along) + channel]);
            }
        }
        for along in 0..length {
            let at = index(line, along);
            for channel in 0..4 {
                #[allow(clippy::cast_possible_truncation)]
                {
                    target[at + channel] = ((sum[channel] + window / 2) / window) as u8;
                }
            }
            let leaving = index(line, along.saturating_sub(radius));
            let entering = index(line, (along + radius + 1).min(length - 1));
            for (channel, total) in sum.iter_mut().enumerate() {
                *total = *total + u32::from(source[entering + channel])
                    - u32::from(source[leaving + channel]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn checker(width: usize, height: usize) -> Vec<u8> {
        let mut pixels = vec![0u8; width * height * 4];
        for y in 0..height {
            for x in 0..width {
                let value = if (x + y) % 2 == 0 { 255 } else { 0 };
                let at = (y * width + x) * 4;
                pixels[at..at + 4].copy_from_slice(&[value, value, value, 255]);
            }
        }
        pixels
    }

    #[test]
    fn regions_clamp_to_the_buffer() {
        let region = Region::clamp(kurbo::Rect::new(-5.0, 2.5, 8.2, 20.0), 10, 10).unwrap();
        assert_eq!(
            region,
            Region {
                x: 0,
                y: 2,
                width: 9,
                height: 8
            }
        );
        assert!(Region::clamp(kurbo::Rect::new(20.0, 20.0, 30.0, 30.0), 10, 10).is_none());
    }

    #[test]
    fn blur_evens_out_a_checkerboard_inside_the_region_only() {
        let mut pixels = checker(20, 20);
        let region = Region {
            x: 5,
            y: 5,
            width: 10,
            height: 10,
        };
        blur(&mut pixels, 20 * 4, region, 4.0);
        let at = |x: usize, y: usize| pixels[(y * 20 + x) * 4];
        let middle = at(10, 10);
        assert!((100..=155).contains(&middle), "{middle}");
        assert!(at(0, 0) == 255 && at(1, 0) == 0, "outside stays sharp");
        assert_eq!(pixels[(10 * 20 + 10) * 4 + 3], 255);
    }

    fn whole(size: usize) -> Region {
        Region {
            x: 0,
            y: 0,
            width: size,
            height: size,
        }
    }

    #[test]
    fn pixelate_fills_blocks_with_their_average_shifted_a_little() {
        let mut pixels = checker(8, 8);
        pixelate(&mut pixels, 8 * 4, whole(8), 4.0);
        let at = |x: usize, y: usize| pixels[(y * 8 + x) * 4];
        for (x, y) in [(0, 0), (4, 0), (0, 4), (4, 4)] {
            let block = at(x, y);
            assert!(block.abs_diff(128) <= 12, "{block}");
            assert!((0..4).all(|dy| (0..4).all(|dx| at(x + dx, y + dy) == block)));
        }
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| { pixel[0] == pixel[1] && pixel[1] == pixel[2] && pixel[3] == 255 })
        );
    }

    #[test]
    fn no_block_is_a_strip_that_keeps_its_pixels() {
        let mut odd = checker(5, 5);
        pixelate(&mut odd, 5 * 4, whole(5), 4.0);
        let first = odd[0];
        assert!(odd.as_chunks::<4>().0.iter().all(|pixel| pixel[0] == first));
        assert_eq!(cuts(13, 4.0), [0, 4, 8, 13]);
        assert_eq!(cuts(9, 12.0), [0, 9]);
        assert_eq!(cuts(0, 12.0), [0, 0]);
        let mut empty = checker(4, 4);
        let before = empty.clone();
        pixelate(
            &mut empty,
            4 * 4,
            Region {
                x: 1,
                y: 1,
                width: 0,
                height: 3,
            },
            4.0,
        );
        assert_eq!(empty, before);
    }

    #[test]
    fn the_same_pixels_never_pixelate_or_blur_the_same_way_twice() {
        let pixelated = || {
            let mut pixels = checker(32, 32);
            pixelate(&mut pixels, 32 * 4, whole(32), 4.0);
            pixels
        };
        assert_ne!(pixelated(), pixelated());
        let blurred = || {
            let mut pixels = checker(32, 32);
            blur(&mut pixels, 32 * 4, whole(32), 4.0);
            pixels
        };
        assert_ne!(blurred(), blurred());
    }

    #[test]
    fn shifts_stay_within_the_alpha() {
        assert_eq!(shifted([250, 5, 100, 255], 12), [255, 17, 112, 255]);
        assert_eq!(shifted([10, 120, 128, 128], 12), [22, 128, 128, 128]);
        assert_eq!(shifted([3, 0, 0, 255], -12), [0, 0, 0, 255]);
    }
}
