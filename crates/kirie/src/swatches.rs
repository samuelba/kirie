//! The colors offered for drawing: the defaults, which can be changed, and custom colors
//! kept for reuse, saved in `~/.config/kirie/colors.ron`.

use std::path::{Path, PathBuf};

use kirie_annotate::{PALETTE, Rgba};
use serde::{Deserialize, Serialize};

/// Custom colors beyond this many drop the oldest.
pub const MAX_CUSTOM: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Swatches {
    defaults: [Rgba; PALETTE.len()],
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
            defaults: PALETTE,
            custom: Vec::new(),
        }
    }
}

/// The file's form: colors as `#rrggbb`, so it is easy to edit by hand.
#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Stored {
    defaults: Vec<String>,
    custom: Vec<String>,
}

impl Swatches {
    pub fn defaults(&self) -> &[Rgba] {
        &self.defaults
    }

    pub fn custom(&self) -> &[Rgba] {
        &self.custom
    }

    /// The color new marks start with.
    pub fn first(&self) -> Rgba {
        self.defaults[0]
    }

    pub fn get(&self, slot: Slot) -> Option<Rgba> {
        match slot {
            Slot::Default(index) => self.defaults.get(index).copied(),
            Slot::Custom(index) => self.custom.get(index).copied(),
        }
    }

    /// Keeps `color` as the newest custom color, unless it is offered already.
    pub fn add(&mut self, color: Rgba) {
        if self.defaults.contains(&color) || self.custom.contains(&color) {
            return;
        }
        self.custom.insert(0, color);
        self.custom.truncate(MAX_CUSTOM);
    }

    /// Changes a swatch. A custom color that would repeat another swatch goes away instead.
    pub fn replace(&mut self, slot: Slot, color: Rgba) {
        match slot {
            Slot::Default(index) => {
                if let Some(old) = self.defaults.get_mut(index) {
                    *old = color;
                    self.custom.retain(|&custom| custom != color);
                }
            }
            Slot::Custom(index) if index < self.custom.len() => {
                let repeated = self.defaults.contains(&color)
                    || self
                        .custom
                        .iter()
                        .enumerate()
                        .any(|(other, &custom)| other != index && custom == color);
                if repeated {
                    self.custom.remove(index);
                } else {
                    self.custom[index] = color;
                }
            }
            Slot::Custom(_) => {}
        }
    }

    pub fn remove(&mut self, slot: Slot) {
        if let Slot::Custom(index) = slot
            && index < self.custom.len()
        {
            self.custom.remove(index);
        }
    }

    pub fn reset_defaults(&mut self) {
        self.defaults = PALETTE;
    }

    /// The saved colors, or the built-in ones when there are none or they cannot be read.
    pub fn load() -> Self {
        let Some(path) = path() else {
            return Self::default();
        };
        match std::fs::read_to_string(&path) {
            Ok(text) => Self::parse(&text).unwrap_or_else(|error| {
                let backup = path.with_extension("ron.bak");
                tracing::warn!(
                    path = %path.display(),
                    backup = %backup.display(),
                    %error,
                    "unreadable colors; keeping them aside and starting from the built-in ones"
                );
                if let Err(error) = std::fs::rename(&path, &backup) {
                    tracing::warn!(%error, "cannot keep the unreadable colors aside");
                }
                Self::default()
            }),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Self::default(),
            Err(error) => {
                tracing::warn!(path = %path.display(), %error, "cannot read the colors");
                Self::default()
            }
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let path = path().ok_or("cannot find the config folder")?;
        write(&path, &self.to_ron()?)
            .map_err(|error| format!("cannot save colors to {}: {error}", path.display()))
    }

    fn parse(text: &str) -> Result<Self, ron::error::SpannedError> {
        let stored: Stored = ron::from_str(text)?;
        let color = |hex: &String| Rgba::from_hex(hex.trim());
        let mut swatches = Self::default();
        for (slot, hex) in swatches.defaults.iter_mut().zip(&stored.defaults) {
            if let Some(color) = color(hex) {
                *slot = color;
            }
        }
        for color in stored.custom.iter().filter_map(color).rev() {
            swatches.add(color);
        }
        Ok(swatches)
    }

    fn to_ron(&self) -> Result<String, String> {
        let hexes = |colors: &[Rgba]| colors.iter().map(|color| color.to_hex()).collect();
        let stored = Stored {
            defaults: hexes(&self.defaults),
            custom: hexes(&self.custom),
        };
        ron::ser::to_string_pretty(&stored, ron::ser::PrettyConfig::default())
            .map_err(|error| error.to_string())
    }
}

fn path() -> Option<PathBuf> {
    directories::BaseDirs::new().map(|dirs| dirs.config_dir().join("kirie/colors.ron"))
}

/// Writes through a temporary file so a crash never leaves half a file behind.
fn write(path: &Path, text: &str) -> std::io::Result<()> {
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder)?;
    }
    let partial = path.with_extension("ron.partial");
    std::fs::write(&partial, text)?;
    std::fs::rename(&partial, path)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEAL: Rgba = Rgba::rgb(0x14, 0xb8, 0xa6);
    const PINK: Rgba = Rgba::rgb(0xec, 0x48, 0x99);

    #[test]
    fn custom_colors_come_newest_first_without_repeats() {
        let mut swatches = Swatches::default();
        swatches.add(TEAL);
        swatches.add(PINK);
        swatches.add(TEAL);
        swatches.add(PALETTE[2]);
        assert_eq!(swatches.custom(), [PINK, TEAL]);
    }

    #[test]
    fn custom_colors_keep_only_the_newest() {
        let mut swatches = Swatches::default();
        for blue in 0..=u8::try_from(MAX_CUSTOM).unwrap() {
            swatches.add(Rgba::rgb(1, 2, blue));
        }
        assert_eq!(swatches.custom().len(), MAX_CUSTOM);
        assert_eq!(swatches.custom()[0], Rgba::rgb(1, 2, 8));
    }

    #[test]
    fn defaults_change_and_reset() {
        let mut swatches = Swatches::default();
        swatches.replace(Slot::Default(0), TEAL);
        assert_eq!(swatches.first(), TEAL);
        swatches.reset_defaults();
        assert_eq!(swatches.defaults(), PALETTE);
    }

    #[test]
    fn changing_a_swatch_never_repeats_another() {
        let mut swatches = Swatches::default();
        swatches.add(TEAL);
        swatches.add(PINK);
        swatches.replace(Slot::Default(0), TEAL);
        assert_eq!(swatches.custom(), [PINK]);
        swatches.replace(Slot::Custom(0), PALETTE[1]);
        assert!(swatches.custom().is_empty());
        swatches.add(PINK);
        swatches.replace(Slot::Custom(0), Rgba::rgb(1, 2, 3));
        assert_eq!(swatches.custom(), [Rgba::rgb(1, 2, 3)]);
    }

    #[test]
    fn only_custom_colors_are_removed() {
        let mut swatches = Swatches::default();
        swatches.add(TEAL);
        swatches.remove(Slot::Default(0));
        swatches.remove(Slot::Custom(5));
        assert_eq!(swatches.defaults(), PALETTE);
        assert_eq!(swatches.get(Slot::Custom(0)), Some(TEAL));
        swatches.remove(Slot::Custom(0));
        assert!(swatches.custom().is_empty());
    }

    #[test]
    fn colors_survive_a_round_trip() {
        let mut swatches = Swatches::default();
        swatches.replace(Slot::Default(3), PINK);
        swatches.add(TEAL);
        swatches.add(PINK.with_alpha(0x80));
        let text = swatches.to_ron().unwrap();
        assert!(text.contains("\"#14b8a6\""), "{text}");
        assert_eq!(Swatches::parse(&text).unwrap(), swatches);
    }

    #[test]
    fn a_partial_file_keeps_the_rest_built_in() {
        let swatches =
            Swatches::parse(r##"(defaults: ["#14b8a6", "nonsense", "#ec4899"])"##).unwrap();
        assert_eq!(swatches.first(), TEAL);
        assert_eq!(swatches.defaults()[1], PALETTE[1]);
        assert_eq!(swatches.defaults()[2], PINK);
        assert_eq!(swatches.defaults()[3..], PALETTE[3..]);
        assert!(swatches.custom().is_empty());
        assert!(Swatches::parse("not ron").is_err());
    }

    #[test]
    fn saving_replaces_the_file_whole() {
        let folder = std::env::temp_dir().join(format!("kirie-swatches-{}", std::process::id()));
        let path = folder.join("nested/colors.ron");
        write(&path, "one").unwrap();
        write(&path, "two").unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "two");
        assert!(!path.with_extension("ron.partial").exists());
        std::fs::remove_dir_all(folder).unwrap();
    }
}
