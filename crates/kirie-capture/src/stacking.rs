//! Which of several overlapping windows is on top, from their pixels: where two windows
//! overlap, the screen shows the upper one, so the window whose own pixels match the screen
//! there is above the other. For compositors that tell where windows are but not how they stack.

use image::{Rgba, RgbaImage};

use crate::geometry::{Frame, Rect};
use crate::pixels::View;

/// The spacing, in logical pixels, of the points where pixels are compared.
const STEP: i32 = 8;
/// How many more points one window must win than the other before it counts as above it.
const MIN_VOTES: usize = 3;
/// How far a colour channel may differ and still match, for compositors that dither.
const TOLERANCE: u8 = 8;

/// Pixels to compare, wherever they are kept.
pub(crate) trait Pixels {
    fn dimensions(&self) -> (u32, u32);
    fn get(&self, x: u32, y: u32) -> Option<Rgba<u8>>;
}

impl Pixels for RgbaImage {
    fn dimensions(&self) -> (u32, u32) {
        self.dimensions()
    }

    fn get(&self, x: u32, y: u32) -> Option<Rgba<u8>> {
        self.get_pixel_checked(x, y).copied()
    }
}

impl Pixels for View<'_> {
    fn dimensions(&self) -> (u32, u32) {
        self.dimensions()
    }

    fn get(&self, x: u32, y: u32) -> Option<Rgba<u8>> {
        self.get(x, y)
    }
}

/// Where on a grid over a window its own pixels match what the screen shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Matches {
    rect: Rect,
    columns: usize,
    bits: Vec<bool>,
}

impl Matches {
    /// Compares `image`, a window's own pixels laid over `rect`, with the screens in `frames`.
    pub(crate) fn new(rect: Rect, image: &impl Pixels, frames: &[Frame]) -> Self {
        let columns = grid(rect.x, rect.right()).count();
        let bits = grid(rect.y, rect.bottom())
            .flat_map(|y| grid(rect.x, rect.right()).map(move |x| (x, y)))
            .map(|(x, y)| {
                let own = pixel(image, rect, x, y);
                let shown = frames
                    .iter()
                    .find(|frame| frame.output.logical.contains(x, y))
                    .and_then(|frame| pixel(&frame.image, frame.output.logical, x, y));
                own.zip(shown).is_some_and(|(own, shown)| close(own, shown))
            })
            .collect();
        Self {
            rect,
            columns,
            bits,
        }
    }

    /// Whether the window matches the screen at grid point (`x`, `y`).
    fn at(&self, x: i32, y: i32) -> Option<bool> {
        let column = usize::try_from((x - first(self.rect.x)) / STEP).ok()?;
        let row = usize::try_from((y - first(self.rect.y)) / STEP).ok()?;
        if column >= self.columns {
            return None;
        }
        self.bits.get(row * self.columns + column).copied()
    }
}

/// The windows topmost first, as indices into `windows`. Windows the pixels say nothing
/// about, or that were not captured (`None`), keep the order they come in.
pub(crate) fn order(windows: &[Option<Matches>]) -> Vec<usize> {
    let count = windows.len();
    let mut covers: Vec<Vec<usize>> = vec![Vec::new(); count];
    let mut covered_by = vec![0usize; count];
    for (upper, lower) in pairs(windows).filter_map(|(i, j)| above(windows, i, j)) {
        covers[upper].push(lower);
        covered_by[lower] += 1;
    }
    let mut placed = vec![false; count];
    let mut stacked = Vec::with_capacity(count);
    while stacked.len() < count {
        let next = (0..count)
            .find(|&index| !placed[index] && covered_by[index] == 0)
            .or_else(|| (0..count).find(|&index| !placed[index]))
            .expect("a window is left to place");
        placed[next] = true;
        stacked.push(next);
        for &lower in &covers[next] {
            covered_by[lower] = covered_by[lower].saturating_sub(1);
        }
    }
    stacked
}

fn pairs(windows: &[Option<Matches>]) -> impl Iterator<Item = (usize, usize)> + '_ {
    (0..windows.len()).flat_map(move |i| (i + 1..windows.len()).map(move |j| (i, j)))
}

/// Which of windows `i` and `j` is above the other, as (upper, lower), if the pixels tell.
fn above(windows: &[Option<Matches>], i: usize, j: usize) -> Option<(usize, usize)> {
    let (first_window, second_window) = (windows[i].as_ref()?, windows[j].as_ref()?);
    let overlap = first_window.rect.intersection(&second_window.rect)?;
    let (mut first_wins, mut second_wins) = (0usize, 0usize);
    for y in grid(overlap.y, overlap.bottom()) {
        for x in grid(overlap.x, overlap.right()) {
            match (first_window.at(x, y), second_window.at(x, y)) {
                (Some(true), Some(false)) => first_wins += 1,
                (Some(false), Some(true)) => second_wins += 1,
                _ => {}
            }
        }
    }
    if first_wins >= MIN_VOTES && first_wins > 2 * second_wins {
        Some((i, j))
    } else if second_wins >= MIN_VOTES && second_wins > 2 * first_wins {
        Some((j, i))
    } else {
        None
    }
}

/// The first grid coordinate at or after `from`.
fn first(from: i32) -> i32 {
    from + (STEP - from.rem_euclid(STEP)) % STEP
}

/// The grid coordinates in `from..until`.
fn grid(from: i32, until: i32) -> impl Iterator<Item = i32> + Clone {
    (first(from)..until).step_by(STEP.unsigned_abs() as usize)
}

/// The pixel of `image`, laid over `rect`, at desktop point (`x`, `y`).
#[allow(
    clippy::cast_possible_truncation,
    clippy::cast_sign_loss,
    clippy::cast_precision_loss
)]
fn pixel(image: &impl Pixels, rect: Rect, x: i32, y: i32) -> Option<Rgba<u8>> {
    if rect.is_empty() || !rect.contains(x, y) {
        return None;
    }
    let scale = |offset: i32, logical: i32, physical: u32| {
        ((f64::from(offset) * f64::from(physical) / f64::from(logical)) as u32)
            .min(physical.saturating_sub(1))
    };
    let (width, height) = image.dimensions();
    image.get(
        scale(x - rect.x, rect.width, width),
        scale(y - rect.y, rect.height, height),
    )
}

fn close(a: Rgba<u8>, b: Rgba<u8>) -> bool {
    a.0.iter()
        .zip(b.0.iter())
        .take(3)
        .all(|(a, b)| a.abs_diff(*b) <= TOLERANCE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::Output;

    const RED: Rgba<u8> = Rgba([200, 0, 0, 255]);
    const BLUE: Rgba<u8> = Rgba([0, 0, 200, 255]);
    const GREEN: Rgba<u8> = Rgba([0, 200, 0, 255]);
    const GREY: Rgba<u8> = Rgba([90, 90, 90, 255]);

    fn screen(width: u32, height: u32, scale: u32) -> Frame {
        Frame {
            output: Output {
                logical: Rect::new(0, 0, width.cast_signed(), height.cast_signed()),
                ..Output::default()
            },
            image: RgbaImage::from_pixel(width * scale, height * scale, GREY),
        }
    }

    /// Paints `rect` of `frame`, given in logical pixels.
    fn paint(frame: &mut Frame, rect: Rect, colour: Rgba<u8>) {
        let scale = frame.image.width() / frame.output.logical.width.unsigned_abs();
        for y in rect.y * scale.cast_signed()..rect.bottom() * scale.cast_signed() {
            for x in rect.x * scale.cast_signed()..rect.right() * scale.cast_signed() {
                frame
                    .image
                    .put_pixel(x.unsigned_abs(), y.unsigned_abs(), colour);
            }
        }
    }

    fn window(rect: Rect, colour: Rgba<u8>, scale: u32, frames: &[Frame]) -> Matches {
        let image = RgbaImage::from_pixel(
            rect.width.unsigned_abs() * scale,
            rect.height.unsigned_abs() * scale,
            colour,
        );
        Matches::new(rect, &image, frames)
    }

    #[test]
    fn the_window_the_screen_shows_in_the_overlap_is_on_top() {
        let (lower, upper) = (Rect::new(0, 0, 200, 200), Rect::new(100, 100, 200, 200));
        let mut frame = screen(400, 400, 1);
        paint(&mut frame, lower, RED);
        paint(&mut frame, upper, BLUE);
        let frames = [frame];
        let windows = [
            Some(window(lower, RED, 1, &frames)),
            Some(window(upper, BLUE, 1, &frames)),
        ];
        assert_eq!(order(&windows), [1, 0]);
    }

    #[test]
    fn a_window_hidden_entirely_goes_below_the_one_covering_it() {
        let (big, small) = (Rect::new(0, 0, 400, 400), Rect::new(50, 50, 100, 100));
        let mut frame = screen(400, 400, 1);
        paint(&mut frame, big, RED);
        let frames = [frame];
        let windows = [
            Some(window(small, BLUE, 1, &frames)),
            Some(window(big, RED, 1, &frames)),
        ];
        assert_eq!(order(&windows), [1, 0]);
    }

    #[test]
    fn chains_of_windows_stack_through_the_middle_one() {
        let (bottom, middle, top) = (
            Rect::new(0, 0, 160, 160),
            Rect::new(100, 0, 160, 160),
            Rect::new(200, 0, 160, 160),
        );
        let mut frame = screen(400, 200, 1);
        paint(&mut frame, bottom, RED);
        paint(&mut frame, middle, GREEN);
        paint(&mut frame, top, BLUE);
        let frames = [frame];
        let windows = [
            Some(window(bottom, RED, 1, &frames)),
            Some(window(top, BLUE, 1, &frames)),
            Some(window(middle, GREEN, 1, &frames)),
        ];
        assert_eq!(order(&windows), [1, 2, 0]);
    }

    #[test]
    fn windows_the_pixels_say_nothing_about_keep_their_order() {
        let (a, b) = (Rect::new(0, 0, 200, 200), Rect::new(100, 100, 200, 200));
        let frames = [screen(400, 400, 1)];
        let same = [
            Some(window(a, GREY, 1, &frames)),
            Some(window(b, GREY, 1, &frames)),
        ];
        assert_eq!(order(&same), [0, 1]);
        let uncaptured = [None, Some(window(b, GREY, 1, &frames))];
        assert_eq!(order(&uncaptured), [0, 1]);
        let apart = [
            Some(window(Rect::new(0, 0, 50, 50), RED, 1, &frames)),
            Some(window(Rect::new(100, 100, 50, 50), BLUE, 1, &frames)),
        ];
        assert_eq!(order(&apart), [0, 1]);
    }

    #[test]
    fn high_density_screens_and_windows_compare_by_logical_position() {
        let (lower, upper) = (Rect::new(0, 0, 100, 100), Rect::new(50, 50, 100, 100));
        let mut frame = screen(200, 200, 2);
        paint(&mut frame, lower, RED);
        paint(&mut frame, upper, BLUE);
        let frames = [frame];
        let windows = [
            Some(window(lower, RED, 2, &frames)),
            Some(window(upper, BLUE, 2, &frames)),
        ];
        assert_eq!(order(&windows), [1, 0]);
    }

    #[test]
    fn grid_points_line_up_across_windows() {
        assert_eq!(grid(3, 30).collect::<Vec<_>>(), [8, 16, 24]);
        assert_eq!(grid(-10, 1).collect::<Vec<_>>(), [-8, 0]);
        assert_eq!(first(16), 16);
    }
}
