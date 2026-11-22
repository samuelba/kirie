//! Text turned into glyph outlines with the bundled fonts, so it renders the same in every
//! painter and on every machine, whatever fonts are installed.

use cosmic_text::{
    Attrs, Buffer, Command, Family, FontSystem, Metrics, Shaping, SwashCache, fontdb,
};
use kurbo::{BezPath, Point, Rect, Shape as _};

use crate::primitive::Font;

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter.ttf");
const CAVEAT: &[u8] = include_bytes!("../../../assets/fonts/Caveat.ttf");
const LINE_HEIGHT: f32 = 1.25;

/// Lays out and outlines text. Building one loads the fonts, so keep it around.
pub struct Typesetter {
    fonts: FontSystem,
    cache: SwashCache,
}

impl std::fmt::Debug for Typesetter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Typesetter").finish_non_exhaustive()
    }
}

impl Default for Typesetter {
    fn default() -> Self {
        Self::new()
    }
}

impl Typesetter {
    pub fn new() -> Self {
        let mut db = fontdb::Database::new();
        db.load_font_data(INTER.to_vec());
        db.load_font_data(CAVEAT.to_vec());
        Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), db),
            cache: SwashCache::new(),
        }
    }

    /// The glyph outlines of `text` at `size`. Unless `centered`, `at` is the top left of the
    /// first line; centred text is centred on `at`.
    #[allow(clippy::cast_possible_truncation)]
    pub fn outline(
        &mut self,
        text: &str,
        font: Font,
        size: f64,
        at: Point,
        centered: bool,
    ) -> BezPath {
        let size = size.max(1.0) as f32;
        let mut buffer = Buffer::new(&mut self.fonts, Metrics::new(size, size * LINE_HEIGHT));
        buffer.set_size(&mut self.fonts, None, None);
        buffer.set_text(
            &mut self.fonts,
            text,
            &Attrs::new().family(Family::Name(font.family())),
            Shaping::Advanced,
            None,
        );
        buffer.shape_until_scroll(&mut self.fonts, false);

        let mut path = BezPath::new();
        for run in buffer.layout_runs() {
            for glyph in run.glyphs {
                let key = glyph.physical((0.0, 0.0), 1.0).cache_key;
                let x = f64::from(glyph.x + glyph.x_offset * glyph.font_size);
                let y = f64::from(run.line_y + glyph.y - glyph.y_offset * glyph.font_size);
                let Some(commands) = self.cache.get_outline_commands(&mut self.fonts, key) else {
                    continue;
                };
                append(&mut path, commands, Point::new(x, y));
            }
        }
        let offset = if centered {
            let bounds = path.bounding_box();
            at - bounds.center()
        } else {
            at.to_vec2()
        };
        path.apply_affine(kurbo::Affine::translate(offset));
        path
    }

    /// The area `text` covers when hung from `at`.
    pub fn bounds(&mut self, text: &str, font: Font, size: f64, at: Point) -> Rect {
        self.outline(text, font, size, at, false).bounding_box()
    }
}

/// Adds glyph commands (y up, relative to the pen) at `origin` (y down).
fn append(path: &mut BezPath, commands: &[Command], origin: Point) {
    let at = |x: f32, y: f32| Point::new(origin.x + f64::from(x), origin.y - f64::from(y));
    for command in commands {
        match *command {
            Command::MoveTo(p) => path.move_to(at(p.x, p.y)),
            Command::LineTo(p) => path.line_to(at(p.x, p.y)),
            Command::QuadTo(c, p) => path.quad_to(at(c.x, c.y), at(p.x, p.y)),
            Command::CurveTo(c1, c2, p) => {
                path.curve_to(at(c1.x, c1.y), at(c2.x, c2.y), at(p.x, p.y));
            }
            Command::Close => path.close_path(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_hangs_below_its_anchor() {
        let mut typesetter = Typesetter::new();
        let bounds = typesetter.bounds("Kirie", Font::Sans, 20.0, Point::new(100.0, 50.0));
        assert!(bounds.x0 >= 100.0 && bounds.x0 < 104.0, "{bounds:?}");
        assert!(bounds.y0 >= 50.0 && bounds.y1 < 75.0, "{bounds:?}");
        assert!(bounds.width() > 30.0);
    }

    #[test]
    fn centered_text_centers_on_its_anchor() {
        let mut typesetter = Typesetter::new();
        let path = typesetter.outline("12", Font::Hand, 30.0, Point::new(40.0, 40.0), true);
        let center = path.bounding_box().center();
        assert!((center.x - 40.0).abs() < 0.01 && (center.y - 40.0).abs() < 0.01);
    }

    #[test]
    fn lines_stack_and_sizes_scale() {
        let mut typesetter = Typesetter::new();
        let one = typesetter.bounds("Ag", Font::Sans, 20.0, Point::ORIGIN);
        let two = typesetter.bounds("Ag\nAg", Font::Sans, 20.0, Point::ORIGIN);
        assert!(two.height() > one.height() * 1.8);
        let big = typesetter.bounds("Ag", Font::Sans, 40.0, Point::ORIGIN);
        assert!(big.width() > one.width() * 1.8);
        assert!(
            typesetter
                .outline("", Font::Sans, 20.0, Point::ORIGIN, false)
                .is_empty()
        );
    }
}
