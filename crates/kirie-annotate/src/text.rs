//! Text turned into glyph outlines with the bundled fonts, so it renders the same in every
//! painter and on every machine, whatever fonts are installed.

use std::collections::HashMap;

use cosmic_text::{
    Attrs, Buffer, Command, Family, FontSystem, Metrics, Shaping, SwashCache, fontdb,
};
use kurbo::{BezPath, Point, Shape as _, Size};

use crate::primitive::{Align, Font};

const INTER: &[u8] = include_bytes!("../../../assets/fonts/Inter.ttf");
const CAVEAT: &[u8] = include_bytes!("../../../assets/fonts/Caveat.ttf");
const SOURCE_CODE_PRO: &[u8] = include_bytes!("../../../assets/fonts/SourceCodePro.otf");
const LINE_HEIGHT: f32 = 1.25;

/// Lays out and outlines text. Building one loads the fonts, so keep it around.
pub struct Typesetter {
    fonts: FontSystem,
    cache: SwashCache,
    /// Text laid out at the origin, by text, font, size and alignment; redraws only move it.
    layouts: HashMap<(String, Font, u64, Align), Layout>,
}

/// Text set at the origin.
#[derive(Debug, Clone)]
struct Layout {
    outline: BezPath,
    /// The room the lines take: the widest line by every line's height.
    size: Size,
}

/// Layouts kept before the cache starts over.
const LAYOUTS: usize = 256;

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
        for font in [INTER, CAVEAT, SOURCE_CODE_PRO] {
            db.load_font_data(font.to_vec());
        }
        Self {
            fonts: FontSystem::new_with_locale_and_db("en-US".into(), db),
            cache: SwashCache::new(),
            layouts: HashMap::new(),
        }
    }

    /// The glyph outlines of `text` at `size`, its lines lined up by `align`. Unless
    /// `centered`, `at` is the top left of the text; centered text is centered on `at`.
    pub fn outline(
        &mut self,
        text: &str,
        font: Font,
        size: f64,
        at: Point,
        align: Align,
        centered: bool,
    ) -> BezPath {
        let mut path = self.layout(text, font, size, align).outline;
        let offset = if centered {
            at - path.bounding_box().center()
        } else {
            at.to_vec2()
        };
        path.apply_affine(kurbo::Affine::translate(offset));
        path
    }

    /// The room `text` takes: its widest line by the height of all its lines.
    pub fn measure(&mut self, text: &str, font: Font, size: f64) -> Size {
        self.layout(text, font, size, Align::Left).size
    }

    fn layout(&mut self, text: &str, font: Font, size: f64, align: Align) -> Layout {
        let key = (text.to_owned(), font, size.to_bits(), align);
        if let Some(layout) = self.layouts.get(&key) {
            return layout.clone();
        }
        let layout = self.lay_out(text, font, size, align);
        if self.layouts.len() >= LAYOUTS {
            self.layouts.clear();
        }
        self.layouts.insert(key, layout.clone());
        layout
    }

    /// `text` hung from the origin.
    #[allow(clippy::cast_possible_truncation)]
    fn lay_out(&mut self, text: &str, font: Font, size: f64, align: Align) -> Layout {
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

        let widest = buffer
            .layout_runs()
            .map(|run| run.line_w)
            .fold(0.0, f32::max);
        let height = buffer
            .layout_runs()
            .map(|run| run.line_top + run.line_height)
            .fold(0.0, f32::max);
        let mut path = BezPath::new();
        for run in buffer.layout_runs() {
            let indent = f64::from(widest - run.line_w) * align.share();
            for glyph in run.glyphs {
                let key = glyph.physical((0.0, 0.0), 1.0).cache_key;
                let x = indent + f64::from(glyph.x + glyph.x_offset * glyph.font_size);
                let y = f64::from(run.line_y + glyph.y - glyph.y_offset * glyph.font_size);
                let Some(commands) = self.cache.get_outline_commands(&mut self.fonts, key) else {
                    continue;
                };
                append(&mut path, commands, Point::new(x, y));
            }
        }
        Layout {
            outline: path,
            size: Size::new(f64::from(widest), f64::from(height)),
        }
    }
}

thread_local! {
    static MEASURER: std::cell::RefCell<Typesetter> = std::cell::RefCell::new(Typesetter::new());
}

/// The room `text` takes, as [`Typesetter::measure`] gives it, with a typesetter kept per
/// thread for code that has none of its own at hand.
pub fn measure(text: &str, font: Font, size: f64) -> Size {
    MEASURER.with_borrow_mut(|typesetter| typesetter.measure(text, font, size))
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

    fn ink(
        typesetter: &mut Typesetter,
        text: &str,
        font: Font,
        size: f64,
        at: Point,
    ) -> kurbo::Rect {
        typesetter
            .outline(text, font, size, at, Align::Left, false)
            .bounding_box()
    }

    #[test]
    fn text_hangs_below_its_anchor() {
        let mut typesetter = Typesetter::new();
        let bounds = ink(
            &mut typesetter,
            "Kirie",
            Font::Sans,
            20.0,
            Point::new(100.0, 50.0),
        );
        assert!(bounds.x0 >= 100.0 && bounds.x0 < 104.0, "{bounds:?}");
        assert!(bounds.y0 >= 50.0 && bounds.y1 < 75.0, "{bounds:?}");
        assert!(bounds.width() > 30.0);
    }

    #[test]
    fn centered_text_centers_on_its_anchor() {
        let mut typesetter = Typesetter::new();
        let path = typesetter.outline(
            "12",
            Font::Hand,
            30.0,
            Point::new(40.0, 40.0),
            Align::Left,
            true,
        );
        let center = path.bounding_box().center();
        assert!((center.x - 40.0).abs() < 0.01 && (center.y - 40.0).abs() < 0.01);
    }

    #[test]
    fn cached_outlines_move_with_their_anchor() {
        let mut typesetter = Typesetter::new();
        let first = ink(
            &mut typesetter,
            "Kirie",
            Font::Sans,
            20.0,
            Point::new(10.0, 10.0),
        );
        let again = ink(
            &mut typesetter,
            "Kirie",
            Font::Sans,
            20.0,
            Point::new(110.0, 60.0),
        );
        assert!((again.x0 - first.x0 - 100.0).abs() < 1e-9);
        assert!((again.y0 - first.y0 - 50.0).abs() < 1e-9);
        assert_eq!(typesetter.layouts.len(), 1);
    }

    #[test]
    fn lines_stack_and_sizes_scale() {
        let mut typesetter = Typesetter::new();
        let one = ink(&mut typesetter, "Ag", Font::Sans, 20.0, Point::ORIGIN);
        let two = ink(&mut typesetter, "Ag\nAg", Font::Sans, 20.0, Point::ORIGIN);
        assert!(two.height() > one.height() * 1.8);
        let big = ink(&mut typesetter, "Ag", Font::Sans, 40.0, Point::ORIGIN);
        assert!(big.width() > one.width() * 1.8);
        assert!(
            typesetter
                .outline("", Font::Sans, 20.0, Point::ORIGIN, Align::Left, false)
                .is_empty()
        );
    }

    #[test]
    fn lines_line_up_as_asked() {
        let mut typesetter = Typesetter::new();
        let text = "a long first line\nshort";
        let mut short_line = |align| {
            let path = typesetter.outline(text, Font::Sans, 20.0, Point::ORIGIN, align, false);
            let lower = path
                .segments()
                .filter(|segment| segment.bounding_box().y0 > 22.0)
                .map(|segment| segment.bounding_box())
                .reduce(|a, b| a.union(b))
                .unwrap();
            (lower.x0, lower.x1)
        };
        let (left, _) = short_line(Align::Left);
        let (_, right) = short_line(Align::Right);
        let (center_x0, center_x1) = short_line(Align::Center);
        let room = typesetter.measure(text, Font::Sans, 20.0).width;
        assert!(left < 3.0, "{left}");
        assert!((room - right).abs() < 3.0, "{right} in {room}");
        assert!((f64::midpoint(center_x0, center_x1) - room / 2.0).abs() < 3.0);
    }

    #[test]
    fn measures_cover_every_line_and_every_font() {
        let mut typesetter = Typesetter::new();
        let one = typesetter.measure("Kirie", Font::Sans, 20.0);
        assert!((one.height - 25.0).abs() < 1e-3, "{one:?}");
        let two = typesetter.measure("Kirie\nKirie", Font::Sans, 20.0);
        assert!((two.height - 50.0).abs() < 1e-3);
        assert!((two.width - one.width).abs() < 1e-3);
        let trailing = typesetter.measure("Kirie\n", Font::Sans, 20.0);
        assert!(
            trailing.height > one.height,
            "an empty last line still takes room"
        );
        for font in Font::ALL {
            let ink = ink(&mut typesetter, "Kirie", font, 20.0, Point::ORIGIN);
            let room = typesetter.measure("Kirie", font, 20.0);
            assert!(ink.width() > 10.0 && ink.x1 <= room.width + 2.0, "{font:?}");
        }
    }
}
