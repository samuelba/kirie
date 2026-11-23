//! What the user set: the folder screenshots go to, how they are named, whether saving copies
//! them too, and whether Kirie looks for new releases. Kept in `~/.config/kirie/settings.ron`.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

const FILE: &str = "settings.ron";
/// How screenshots are named unless the user says otherwise.
pub const DEFAULT_NAME: &str = "Kirie_%Y-%m-%d_%H-%M-%S";
/// The longest file name a pattern may make, in bytes, leaving room for a counter and extension.
const LONGEST_NAME: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where screenshots are saved; `None` for `~/Pictures/Screenshots`.
    pub folder: Option<PathBuf>,
    /// The file name without extension, with `strftime` fields for the time it was taken.
    pub name: String,
    /// Saving puts the picture on the clipboard too.
    pub copy_on_save: bool,
    /// Kirie asks GitHub for the latest release when it starts and once a day.
    pub check_updates: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            folder: None,
            name: DEFAULT_NAME.to_owned(),
            copy_on_save: true,
            check_updates: true,
        }
    }
}

impl Settings {
    /// The settings saved last, or the usual ones; a name pattern that no longer works falls
    /// back to the usual one.
    pub fn load() -> Self {
        let loaded: Self = config_path()
            .and_then(|path| crate::store::read(&path))
            .unwrap_or_default();
        if name(&loaded.name, &jiff::Zoned::now()).is_ok() {
            loaded
        } else {
            Self {
                name: DEFAULT_NAME.to_owned(),
                ..loaded
            }
        }
    }

    /// Keeps the settings; tests keep nothing.
    pub fn save(&self) -> Result<(), String> {
        match config_path() {
            Some(path) => crate::store::save(&path, self),
            None if cfg!(test) => Ok(()),
            None => Err("cannot find the config folder".to_owned()),
        }
    }

    /// The folder screenshots are saved to.
    pub fn folder(&self) -> PathBuf {
        self.folder.clone().unwrap_or_else(crate::save::default_dir)
    }

    /// The name of a screenshot taken `now`, without extension.
    pub fn stem(&self, now: &jiff::Zoned) -> String {
        name(&self.name, now)
            .unwrap_or_else(|_| name(DEFAULT_NAME, now).unwrap_or_else(|_| "Kirie".to_owned()))
    }
}

/// `~/.config/kirie/settings.ron`. Tests get none, so they never read or change the user's.
fn config_path() -> Option<PathBuf> {
    if cfg!(test) {
        return None;
    }
    let dirs = directories::BaseDirs::new()?;
    Some(dirs.config_dir().join("kirie").join(FILE))
}

/// The file name `pattern` makes for a screenshot taken `now`, or why it makes none.
pub fn name(pattern: &str, now: &jiff::Zoned) -> Result<String, String> {
    if pattern.trim().is_empty() {
        return Err("The name cannot be empty".to_owned());
    }
    let made = jiff::fmt::strtime::format(pattern, now)
        .map_err(|_| "The name has a % that is not a known field".to_owned())?;
    let made = made.trim().to_owned();
    if made.contains(['/', '\0']) {
        return Err("The name cannot hold a /".to_owned());
    }
    if made.starts_with('.') {
        return Err("The name cannot start with a dot".to_owned());
    }
    if made.len() > LONGEST_NAME {
        return Err("The name is too long".to_owned());
    }
    Ok(made)
}

/// `folder` as it is shown: with the home folder as `~`.
pub fn shown(folder: &Path) -> String {
    crate::save::pretty(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn noon() -> jiff::Zoned {
        "2026-11-22T12:00:05[UTC]".parse().unwrap()
    }

    #[test]
    fn patterns_name_screenshots_by_their_time() {
        assert_eq!(
            name(DEFAULT_NAME, &noon()).unwrap(),
            "Kirie_2026-11-22_12-00-05"
        );
        assert_eq!(name("shot %H%M", &noon()).unwrap(), "shot 1200");
        assert_eq!(name("plain", &noon()).unwrap(), "plain");
    }

    #[test]
    fn patterns_that_make_no_file_name_are_refused() {
        for bad in [
            "",
            "   ",
            "%Q%",
            "a/b",
            "%Y/%m",
            ".hidden",
            &"x".repeat(300),
        ] {
            assert!(name(bad, &noon()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_broken_pattern_falls_back_to_the_usual_name() {
        let settings = Settings {
            name: "%Q%".to_owned(),
            ..Settings::default()
        };
        assert_eq!(settings.stem(&noon()), "Kirie_2026-11-22_12-00-05");
        assert!(Settings::default().copy_on_save);
        assert!(Settings::default().check_updates);
        assert!(Settings::default().folder().ends_with("Screenshots"));
        let elsewhere = Settings {
            folder: Some(PathBuf::from("/shots")),
            ..Settings::default()
        };
        assert_eq!(elsewhere.folder(), Path::new("/shots"));
    }

    #[test]
    fn settings_survive_a_round_trip_and_old_files_get_defaults() {
        let settings = Settings {
            folder: Some(PathBuf::from("/shots")),
            name: "x".to_owned(),
            copy_on_save: false,
            check_updates: false,
        };
        let text = ron::to_string(&settings).unwrap();
        assert_eq!(ron::from_str::<Settings>(&text).unwrap(), settings);
        let old: Settings = ron::from_str("(folder: None)").unwrap();
        assert_eq!(old, Settings::default());
    }
}
