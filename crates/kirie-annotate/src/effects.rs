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

/// Blurs the region with three box blurs, close to a Gaussian of `radius`.
pub fn blur(pixels: &mut [u8], stride: usize, region: Region, radius: f64) {
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

/// Replaces the region with blocks of `cell` pixels, each the average of its pixels.
pub fn pixelate(pixels: &mut [u8], stride: usize, region: Region, cell: f64) {
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    let cell = (cell.round() as usize).max(2);
    for block_y in (region.y..region.y + region.height).step_by(cell) {
        for block_x in (region.x..region.x + region.width).step_by(cell) {
            let x_end = (block_x + cell).min(region.x + region.width);
            let y_end = (block_y + cell).min(region.y + region.height);
            let mut sum = [0u64; 4];
            for y in block_y..y_end {
                for x in block_x..x_end {
                    let at = y * stride + x * 4;
                    for (channel, total) in sum.iter_mut().enumerate() {
                        *total += u64::from(pixels[at + channel]);
                    }
                }
            }
            let count = ((x_end - block_x) * (y_end - block_y)) as u64;
            #[allow(clippy::cast_possible_truncation)]
            let average = sum.map(|total| ((total + count / 2) / count) as u8);
            for y in block_y..y_end {
                for x in block_x..x_end {
                    let at = y * stride + x * 4;
                    pixels[at..at + 4].copy_from_slice(&average);
                }
            }
        }
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

    #[test]
    fn pixelate_fills_blocks_with_their_average() {
        let mut pixels = checker(8, 8);
        pixelate(
            &mut pixels,
            8 * 4,
            Region {
                x: 0,
                y: 0,
                width: 8,
                height: 8,
            },
            4.0,
        );
        assert!(
            pixels
                .as_chunks::<4>()
                .0
                .iter()
                .all(|pixel| pixel[0] == 128)
        );
        let mut odd = checker(5, 5);
        pixelate(
            &mut odd,
            5 * 4,
            Region {
                x: 0,
                y: 0,
                width: 5,
                height: 5,
            },
            4.0,
        );
        assert_eq!(
            odd[(4 * 5 + 4) * 4],
            255,
            "a 1×1 edge block keeps its pixel"
        );
    }
}
