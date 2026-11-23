//! A magnifying glass beside the pointer: the pixels around it, large, with the one under it
//! ringed, and a label with its position and color. For cutting exact edges and picking colors.

use iced::widget::canvas::{Frame, Path, Stroke, Text};
use iced::{Color, Pixels, Point, Size, alignment};
use image::RgbaImage;
use kirie_ui::{Palette, radius, space};

/// Pixels shown on each side of the one under the pointer.
pub const REACH: u32 = 7;
/// How large each pixel is drawn, in screen pixels.
const CELL: f32 = 9.0;
/// How far from the pointer the loupe sits.
const GAP: f32 = 24.0;
const LABEL: f32 = 12.0;

/// The pixels around one, row by row, `None` beyond the image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lens {
    pixels: Vec<Option<[u8; 4]>>,
}

impl Lens {
    /// The pixels of `image` around (`x`, `y`).
    pub fn sample(image: &RgbaImage, x: i64, y: i64) -> Self {
        let reach = i64::from(REACH);
        let pixels = (y - reach..=y + reach)
            .flat_map(|row| (x - reach..=x + reach).map(move |column| (column, row)))
            .map(|(column, row)| {
                let column = u32::try_from(column).ok()?;
                let row = u32::try_from(row).ok()?;
                image.get_pixel_checked(column, row).map(|pixel| pixel.0)
            })
            .collect();
        Self { pixels }
    }

    /// The pixel under the pointer.
    pub fn center(&self) -> Option<[u8; 4]> {
        self.pixels.get(self.pixels.len() / 2).copied().flatten()
    }
}

/// `#rrggbb`, with the alpha when it is not opaque.
pub fn hex([r, g, b, a]: [u8; 4]) -> String {
    kirie_annotate::Rgba { r, g, b, a }.to_hex()
}

/// Draws `lens` and `label` beside `at`, kept within `size`.
pub fn draw(frame: &mut Frame, lens: &Lens, label: &str, at: Point, size: Size, palette: Palette) {
    #[allow(clippy::cast_precision_loss)]
    let side = (2 * REACH + 1) as f32 * CELL;
    let card = Size::new(side + space::S * 2.0, side + space::S * 3.0 + LABEL);
    let x = if at.x + GAP + card.width <= size.width {
        at.x + GAP
    } else {
        at.x - GAP - card.width
    };
    let y = if at.y + GAP + card.height <= size.height {
        at.y + GAP
    } else {
        at.y - GAP - card.height
    };
    let (x, y) = (x.round(), y.round());
    let origin = Point::new(x, y);
    let outline = Path::rounded_rectangle(origin, card, radius::CONTROL.into());
    frame.fill(&outline, palette.surface[0]);
    frame.stroke(
        &outline,
        Stroke::default().with_color(palette.border).with_width(1.0),
    );
    let grid = Point::new(x + space::S, y + space::S);
    let columns = 2 * REACH as usize + 1;
    for (index, pixel) in lens.pixels.iter().enumerate() {
        #[allow(clippy::cast_precision_loss)]
        let cell = Point::new(
            grid.x + (index % columns) as f32 * CELL,
            grid.y + (index / columns) as f32 * CELL,
        );
        let color = pixel.map_or(Color::from_rgb8(0x10, 0x11, 0x14), |[r, g, b, _]| {
            Color::from_rgb8(r, g, b)
        });
        frame.fill_rectangle(cell, Size::new(CELL, CELL), color);
    }
    #[allow(clippy::cast_precision_loss)]
    let middle = Point::new(grid.x + REACH as f32 * CELL, grid.y + REACH as f32 * CELL);
    let ring = Path::rectangle(middle, Size::new(CELL, CELL));
    frame.stroke(
        &ring,
        Stroke::default().with_color(Color::BLACK).with_width(3.0),
    );
    frame.stroke(
        &ring,
        Stroke::default().with_color(Color::WHITE).with_width(1.5),
    );
    frame.fill_text(Text {
        content: label.to_owned(),
        position: Point::new(x + card.width / 2.0, grid.y + side + space::S + LABEL / 2.0),
        color: palette.text,
        size: Pixels(LABEL),
        align_x: alignment::Horizontal::Center.into(),
        align_y: alignment::Vertical::Center,
        ..Text::default()
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_lens_is_centered_on_the_pixel_and_empty_beyond_the_image() {
        #[allow(clippy::cast_possible_truncation)]
        let image = RgbaImage::from_fn(20, 20, |x, y| image::Rgba([x as u8, y as u8, 0, 255]));
        let lens = Lens::sample(&image, 10, 3);
        let side = 2 * REACH as usize + 1;
        assert_eq!(lens.pixels.len(), side * side);
        assert_eq!(lens.center(), Some([10, 3, 0, 255]));
        assert_eq!(lens.pixels[0], None, "above the top edge");
        assert_eq!(lens.pixels[side * REACH as usize], Some([3, 3, 0, 255]));
        assert_eq!(Lens::sample(&image, -1, 0).center(), None);
        assert_eq!(hex([0xe4, 0x57, 0x2e, 255]), "#e4572e");
        assert_eq!(hex([0, 0, 0, 0x80]), "#00000080");
    }
}
