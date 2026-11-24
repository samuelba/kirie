//! The colors offered for drawing: each style's shades of the standard colors, which can be
//! changed, and custom colors kept for reuse, saved in `~/.config/kirie/colors.ron`.

use std::path::PathBuf;

use kirie_annotate::{PALETTE, Rgba, StyleId};
use serde::{Deserialize, Serialize};

/// Custom colors beyond this many drop the oldest.
pub const MAX_CUSTOM: usize = 8;

type Defaults = [Rgba; PALETTE.len()];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swatches {
    /// Each style's standard colors, in the order of [`StyleId::ALL`].
    defaults: [Defaults; StyleId::ALL.len()],
    custom: Vec<Rgba>,
}

/// Which swatch a change is for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Slot {
    Default(usize),
    Custom(usize),
}

impl Default for Swatches {
    fn default() -> Self {
        Self {
            defaults: StyleId::ALL.map(StyleId::palette),
            custom: Vec::new(),
        }
    }
}

/// The file's form: colors as `#rrggbb`, so it is easy to edit by hand. `defaults` are the
/// clean style's, as before the other styles had their own.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    defaults: Vec<String>,
    sketch: Vec<String>,
    ink: Vec<String>,
    custom: Vec<String>,
}

fn index(style: StyleId) -> usize {
    match style {
        StyleId::Clean => 0,
        StyleId::Sketch => 1,
        StyleId::Ink => 2,
    }
}

impl Swatches {
    pub fn defaults(&self, style: StyleId) -> &[Rgba] {
        &self.defaults[index(style)]
    }

    pub fn custom(&self) -> &[Rgba] {
        &self.custom
    }

    /// The color new marks in `style` start with.
    pub fn first(&self, style: StyleId) -> Rgba {
        self.defaults(style)[0]
    }

    pub fn get(&self, style: StyleId, slot: Slot) -> Option<Rgba> {
        match slot {
            Slot::Default(at) => self.defaults(style).get(at).copied(),
            Slot::Custom(at) => self.custom.get(at).copied(),
        }
    }

    /// Each standard color as `from` shades it, paired with how `to` shades it; custom colors
    /// stay as they are.
    pub fn recolor(&self, from: StyleId, to: StyleId) -> Vec<(Rgba, Rgba)> {
        self.defaults(from)
            .iter()
            .zip(self.defaults(to))
            .filter(|(old, new)| old != new)
            .map(|(old, new)| (*old, *new))
            .collect()
    }

    /// Keeps `color` as the newest custom color, unless `style` offers it already.
    pub fn add(&mut self, style: StyleId, color: Rgba) {
        if self.defaults(style).contains(&color) || self.custom.contains(&color) {
            return;
        }
        self.custom.insert(0, color);
        self.custom.truncate(MAX_CUSTOM);
    }

    /// Changes a swatch of `style`. A custom color that would repeat another swatch goes away
    /// instead.
    pub fn replace(&mut self, style: StyleId, slot: Slot, color: Rgba) {
        match slot {
            Slot::Default(at) => {
                if let Some(old) = self.defaults[index(style)].get_mut(at) {
                    *old = color;
                    self.custom.retain(|&custom| custom != color);
                }
            }
            Slot::Custom(at) if at < self.custom.len() => {
                let repeated = self.defaults(style).contains(&color)
                    || self
                        .custom
                        .iter()
                        .enumerate()
                        .any(|(other, &custom)| other != at && custom == color);
                if repeated {
                    self.custom.remove(at);
                } else {
                    self.custom[at] = color;
                }
            }
            Slot::Custom(_) => {}
        }
    }

    pub fn remove(&mut self, slot: Slot) {
        if let Slot::Custom(at) = slot
            && at < self.custom.len()
        {
            self.custom.remove(at);
        }
    }

    pub fn reset_defaults(&mut self) {
        self.defaults = Self::default().defaults;
    }

    /// The saved colors, or the built-in ones when there are none or they cannot be read.
    pub fn load() -> Self {
        path()
            .and_then(|path| crate::storage::store::read::<Stored>(&path))
            .map(|stored| Self::from_stored(&stored))
            .unwrap_or_default()
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path().ok_or("cannot find the config folder")?;
        crate::storage::store::write(&path, &self.to_ron()?)
            .map_err(|error| format!("cannot save colors to {}: {error}", path.display()))
    }

    #[cfg(test)]
    fn parse(text: &str) -> Result<Self, ron::error::SpannedError> {
        ron::from_str(text).map(|stored| Self::from_stored(&stored))
    }

    fn from_stored(stored: &Stored) -> Self {
        let color = |hex: &String| Rgba::from_hex(hex.trim());
        let mut swatches = Self::default();
        for (style, kept) in StyleId::ALL.into_iter().zip(stored.by_style()) {
            for (slot, hex) in swatches.defaults[index(style)].iter_mut().zip(kept) {
                if let Some(color) = color(hex) {
                    *slot = color;
                }
            }
        }
        for color in stored.custom.iter().filter_map(color).rev() {
            if !swatches.custom.contains(&color) {
                swatches.custom.insert(0, color);
            }
        }
        swatches.custom.truncate(MAX_CUSTOM);
        swatches
    }

    fn to_ron(&self) -> Result<String, String> {
        let hexes = |colors: &[Rgba]| colors.iter().map(|color| color.to_hex()).collect();
        let [clean, sketch, ink] = StyleId::ALL.map(|style| hexes(self.defaults(style)));
        let stored = Stored {
            defaults: clean,
            sketch,
            ink,
            custom: hexes(&self.custom),
        };
        ron::ser::to_string_pretty(&stored, ron::ser::PrettyConfig::default())
            .map_err(|error| error.to_string())
    }
}

impl Stored {
    fn by_style(&self) -> [&Vec<String>; StyleId::ALL.len()] {
        StyleId::ALL.map(|style| match style {
            StyleId::Clean => &self.defaults,
            StyleId::Sketch => &self.sketch,
            StyleId::Ink => &self.ink,
        })
    }
}

fn path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("kirie/colors.ron"))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAL: Rgba = Rgba::rgb(0x14, 0xb8, 0xa6);
    const PINK: Rgba = Rgba::rgb(0xec, 0x48, 0x99);

    #[test]
    fn custom_colors_come_newest_first_without_repeats() {
        let mut swatches = Swatches::default();
        swatches.add(StyleId::Clean, TEAL);
        swatches.add(StyleId::Clean, PINK);
        swatches.add(StyleId::Clean, TEAL);
        swatches.add(StyleId::Clean, PALETTE[2]);
        assert_eq!(swatches.custom(), [PINK, TEAL]);
    }

    #[test]
    fn custom_colors_keep_only_the_newest() {
        let mut swatches = Swatches::default();
        for blue in 0..=u8::try_from(MAX_CUSTOM).unwrap() {
            swatches.add(StyleId::Clean, Rgba::rgb(1, 2, blue));
        }
        assert_eq!(swatches.custom().len(), MAX_CUSTOM);
        assert_eq!(swatches.custom()[0], Rgba::rgb(1, 2, 8));
    }

    #[test]
    fn defaults_change_and_reset() {
        let mut swatches = Swatches::default();
        swatches.replace(StyleId::Clean, Slot::Default(0), TEAL);
        assert_eq!(swatches.first(StyleId::Clean), TEAL);
        swatches.reset_defaults();
        assert_eq!(swatches.defaults(StyleId::Clean), PALETTE);
    }

    #[test]
    fn changing_a_swatch_never_repeats_another() {
        let mut swatches = Swatches::default();
        swatches.add(StyleId::Clean, TEAL);
        swatches.add(StyleId::Clean, PINK);
        swatches.replace(StyleId::Clean, Slot::Default(0), TEAL);
        assert_eq!(swatches.custom(), [PINK]);
        swatches.replace(StyleId::Clean, Slot::Custom(0), PALETTE[1]);
        assert!(swatches.custom().is_empty());
        swatches.add(StyleId::Clean, PINK);
        swatches.replace(StyleId::Clean, Slot::Custom(0), Rgba::rgb(1, 2, 3));
        assert_eq!(swatches.custom(), [Rgba::rgb(1, 2, 3)]);
    }

    #[test]
    fn only_custom_colors_are_removed() {
        let mut swatches = Swatches::default();
        swatches.add(StyleId::Clean, TEAL);
        swatches.remove(Slot::Default(0));
        swatches.remove(Slot::Custom(5));
        assert_eq!(swatches.defaults(StyleId::Clean), PALETTE);
        assert_eq!(swatches.get(StyleId::Clean, Slot::Custom(0)), Some(TEAL));
        swatches.remove(Slot::Custom(0));
        assert!(swatches.custom().is_empty());
    }

    #[test]
    fn colors_survive_a_round_trip() {
        let mut swatches = Swatches::default();
        swatches.replace(StyleId::Clean, Slot::Default(3), PINK);
        swatches.add(StyleId::Clean, TEAL);
        swatches.add(StyleId::Clean, PINK.with_alpha(0x80));
        let text = swatches.to_ron().unwrap();
        assert!(text.contains("\"#14b8a6\""), "{text}");
        assert_eq!(Swatches::parse(&text).unwrap(), swatches);
    }

    #[test]
    fn a_partial_file_keeps_the_rest_built_in() {
        let swatches =
            Swatches::parse(r##"(defaults: ["#14b8a6", "nonsense", "#ec4899"])"##).unwrap();
        assert_eq!(swatches.first(StyleId::Clean), TEAL);
        assert_eq!(swatches.defaults(StyleId::Clean)[1], PALETTE[1]);
        assert_eq!(swatches.defaults(StyleId::Clean)[2], PINK);
        assert_eq!(swatches.defaults(StyleId::Clean)[3..], PALETTE[3..]);
        assert!(swatches.custom().is_empty());
        assert!(Swatches::parse("not ron").is_err());
    }

    #[test]
    fn each_style_keeps_its_own_colors() {
        let mut swatches = Swatches::default();
        for style in StyleId::ALL {
            assert_eq!(swatches.defaults(style), style.palette());
        }
        swatches.replace(StyleId::Sketch, Slot::Default(1), TEAL);
        assert_eq!(swatches.defaults(StyleId::Sketch)[1], TEAL);
        assert_eq!(swatches.defaults(StyleId::Clean), PALETTE);
        let text = swatches.to_ron().unwrap();
        assert_eq!(Swatches::parse(&text).unwrap(), swatches);
        swatches.reset_defaults();
        assert_eq!(
            swatches.defaults(StyleId::Sketch),
            StyleId::Sketch.palette()
        );
    }

    #[test]
    fn a_new_style_reshades_only_the_standard_colors() {
        let mut swatches = Swatches::default();
        swatches.add(StyleId::Clean, TEAL);
        let recolor = swatches.recolor(StyleId::Clean, StyleId::Ink);
        assert_eq!(recolor.len(), PALETTE.len());
        assert_eq!(recolor[0], (PALETTE[0], StyleId::Ink.palette()[0]));
        assert!(recolor.iter().all(|(from, _)| *from != TEAL));
        assert!(swatches.recolor(StyleId::Ink, StyleId::Ink).is_empty());
    }

    #[test]
    fn older_files_keep_their_colors_for_the_clean_style() {
        let swatches =
            Swatches::parse(r##"(defaults: ["#14b8a6"], custom: ["#ec4899"])"##).unwrap();
        assert_eq!(swatches.first(StyleId::Clean), TEAL);
        assert_eq!(swatches.defaults(StyleId::Ink), StyleId::Ink.palette());
        assert_eq!(swatches.custom(), [PINK]);
    }
}
