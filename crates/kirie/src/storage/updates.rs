//! Which releases the user wants to hear about: Kirie may look for them, and the one found was
//! not skipped.

use std::path::Path;

use semver::Version;

const SKIPPED_FILE: &str = "update-skipped";

/// Whether the user wants to hear about `version`: Kirie may look for releases and this one
/// was not skipped.
pub fn wanted(version: &Version) -> bool {
    crate::storage::settings::Settings::load().check_updates && !is_skipped(version)
}

/// Whether the user chose to skip exactly this version.
pub fn is_skipped(version: &Version) -> bool {
    crate::storage::store::state_path(SKIPPED_FILE).is_some_and(|path| skipped_at(&path, version))
}

fn skipped_at(path: &Path, version: &Version) -> bool {
    std::fs::read_to_string(path).is_ok_and(|skipped| skipped.trim() == version.to_string())
}

/// Remembers that the user does not want to hear about `version` again.
pub fn skip(version: &Version) {
    if let Some(path) = crate::storage::store::state_path(SKIPPED_FILE) {
        skip_at(&path, version);
    }
}

fn skip_at(path: &Path, version: &Version) {
    if let Err(error) = crate::storage::store::write(path, &version.to_string()) {
        tracing::warn!(path = %path.display(), %error, "cannot remember the skipped release");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_version_stays_skipped_until_a_newer_one() {
        let dir = std::env::temp_dir().join(format!("kirie-release-{}", std::process::id()));
        let path = dir.join("state").join(SKIPPED_FILE);
        let v030 = Version::new(0, 3, 0);
        assert!(!skipped_at(&path, &v030));
        skip_at(&path, &v030);
        assert!(skipped_at(&path, &v030));
        assert!(!skipped_at(&path, &Version::new(0, 4, 0)));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
