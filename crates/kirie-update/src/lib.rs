//! Looking up the latest release on GitHub, and updating an install made by `install.sh`.

mod install;
mod verify;

pub use install::{Installed, install, install_blocking};

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::time::{Duration, SystemTime};

use semver::Version;
use serde::Deserialize;

pub const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");
const REPOSITORY: &str = env!("CARGO_PKG_REPOSITORY");
const GITHUB: &str = "https://github.com/";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const USER_AGENT: &str = concat!("kirie/", env!("CARGO_PKG_VERSION"));
/// The marker `install.sh` writes: `method=script` and `version=<installed version>`.
const MARKER: &str = "kirie/install-method";
/// How often a running editor or tray icon looks for a newer release.
pub const CHECK_EVERY: Duration = Duration::from_secs(24 * 60 * 60);

/// Whether a look for a newer release is due, by the wall clock: a timer alone runs late after
/// the machine slept, and a day of it can take a week.
pub fn due(last: Option<SystemTime>, now: SystemTime) -> bool {
    last.is_none_or(|last| {
        now.duration_since(last)
            .is_ok_and(|elapsed| elapsed >= CHECK_EVERY)
    })
}

/// A published release that is newer than the running build.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Release {
    pub version: Version,
    /// The release page, with its notes.
    pub url: String,
}

/// The fields of GitHub's release object that matter here.
#[derive(Debug, Deserialize)]
struct Latest {
    tag_name: String,
    html_url: String,
}

/// Asks GitHub for the latest release; `None` when the running build is up to date.
pub async fn check() -> Result<Option<Release>, String> {
    off_the_runtime(check_blocking).await
}

/// Runs blocking network work on tokio's blocking threads.
async fn off_the_runtime<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .unwrap_or_else(|_| Err("cancelled".to_owned()))
}

pub fn check_blocking() -> Result<Option<Release>, String> {
    let url = latest_release_api(REPOSITORY)
        .ok_or_else(|| format!("the repository {REPOSITORY} is not on GitHub"))?;
    let latest: Latest = AGENT
        .get(&url)
        .header("Accept", "application/vnd.github+json")
        .call()
        .and_then(|mut response| response.body_mut().read_json())
        .map_err(|error| format!("cannot reach GitHub: {error}"))?;
    newer_than(CURRENT_VERSION, latest)
}

/// One agent for every request, so the downloads of an update share their connections.
static AGENT: LazyLock<ureq::Agent> = LazyLock::new(|| {
    ureq::Agent::config_builder()
        .timeout_global(Some(REQUEST_TIMEOUT))
        .user_agent(USER_AGENT)
        .build()
        .into()
});

/// `https://github.com/OWNER/REPO/releases`, where the release pages and their files are.
fn releases_url() -> String {
    format!("{}/releases", REPOSITORY.trim_end_matches('/'))
}

/// The GitHub API endpoint for a `https://github.com/OWNER/REPO` repository.
fn latest_release_api(repository: &str) -> Option<String> {
    let path = repository.strip_prefix(GITHUB)?.trim_end_matches('/');
    let (owner, repo) = path.split_once('/')?;
    let valid = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
    };
    (valid(owner) && valid(repo))
        .then(|| format!("https://api.github.com/repos/{owner}/{repo}/releases/latest"))
}

fn newer_than(current: &str, latest: Latest) -> Result<Option<Release>, String> {
    let bare = latest
        .tag_name
        .strip_prefix('v')
        .unwrap_or(&latest.tag_name);
    let version = Version::parse(bare)
        .map_err(|_| format!("the release tag {:?} is not a version", latest.tag_name))?;
    if !latest.html_url.starts_with(&format!("{}/", releases_url())) {
        return Err(format!(
            "the release page {:?} is not in this repository",
            latest.html_url
        ));
    }
    let current = Version::parse(current).map_err(|error| error.to_string())?;
    Ok((version > current).then(|| Release {
        version,
        url: latest.html_url,
    }))
}

/// How the running binary was installed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InstallKind {
    /// By `install.sh`. `installed` is the version the script last put in place: after an
    /// update it is newer than the running one, until Kirie starts again.
    Script { installed: Option<Version> },
    /// Built from source, or put in place by hand: only a link to the release helps.
    Other,
}

/// How the binary at `running` was installed. `install.sh` puts it in `XDG_BIN_HOME` or
/// `~/.local/bin`, as `directories` finds those.
pub fn install_kind(running: Option<&Path>) -> InstallKind {
    let Some(dirs) = directories::BaseDirs::new() else {
        return InstallKind::Other;
    };
    let Some(bin_dir) = dirs.executable_dir() else {
        return InstallKind::Other;
    };
    let marker = std::fs::read_to_string(dirs.data_dir().join(MARKER)).ok();
    kind_of(marker.as_deref(), running, &script_binary(bin_dir))
}

/// Where `install.sh` put the binary, with links resolved as the running binary's path has them.
fn script_binary(bin_dir: &Path) -> PathBuf {
    let binary = bin_dir.join("kirie");
    std::fs::canonicalize(&binary).unwrap_or(binary)
}

/// The install kind from the marker's text, the running binary and the one the script put in
/// place.
fn kind_of(marker: Option<&str>, running: Option<&Path>, binary: &Path) -> InstallKind {
    let value = |key: &str| {
        marker?
            .lines()
            .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
            .map(str::trim)
    };
    if value("method") != Some("script") || running != Some(binary) {
        return InstallKind::Other;
    }
    InstallKind::Script {
        installed: value("version").and_then(|version| Version::parse(version).ok()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn latest(tag: &str) -> Latest {
        Latest {
            tag_name: tag.into(),
            html_url: format!("{REPOSITORY}/releases/tag/{tag}"),
        }
    }

    #[test]
    fn only_a_higher_release_is_offered() {
        let release = newer_than("0.2.0", latest("v0.3.0")).unwrap().unwrap();
        assert_eq!(release.version, Version::new(0, 3, 0));
        assert_eq!(release.url, format!("{REPOSITORY}/releases/tag/v0.3.0"));
        assert_eq!(newer_than("0.3.0", latest("v0.3.0")).unwrap(), None);
        assert_eq!(newer_than("0.3.0", latest("v0.2.9")).unwrap(), None);
        assert!(newer_than("0.9.0", latest("0.10.0")).unwrap().is_some());
    }

    #[test]
    fn odd_tags_and_foreign_pages_are_refused() {
        assert!(newer_than("0.2.0", latest("nightly")).is_err());
        let elsewhere = Latest {
            tag_name: "v9.0.0".into(),
            html_url: "https://example.com/releases/tag/v9.0.0".into(),
        };
        assert!(newer_than("0.2.0", elsewhere).is_err());
    }

    #[test]
    fn the_api_url_comes_from_the_repository() {
        assert_eq!(
            latest_release_api("https://github.com/owner/repo/").as_deref(),
            Some("https://api.github.com/repos/owner/repo/releases/latest")
        );
        assert!(latest_release_api(REPOSITORY).is_some());
        assert_eq!(latest_release_api("https://gitlab.com/owner/repo"), None);
        assert_eq!(latest_release_api("https://github.com/owner"), None);
        assert_eq!(latest_release_api("https://github.com/owner/re?po"), None);
        assert!(Version::parse(CURRENT_VERSION).is_ok());
    }

    #[test]
    #[ignore = "talks to the GitHub API"]
    fn the_latest_release_can_be_fetched() {
        if let Some(release) = check_blocking().unwrap() {
            assert!(release.url.starts_with(REPOSITORY));
        }
    }

    #[test]
    fn a_check_is_due_a_day_after_the_last_by_the_clock() {
        let now = SystemTime::now();
        assert!(due(None, now));
        assert!(!due(Some(now), now));
        assert!(!due(Some(now - Duration::from_secs(60)), now));
        assert!(due(Some(now - CHECK_EVERY), now));
        assert!(
            !due(Some(now + Duration::from_secs(60)), now),
            "a clock set back waits"
        );
    }

    #[test]
    fn a_linked_bin_folder_is_resolved() {
        let dir = std::env::temp_dir().join(format!("kirie-bin-{}", std::process::id()));
        let real = dir.join("real");
        std::fs::create_dir_all(&real).unwrap();
        std::fs::write(real.join("kirie"), "").unwrap();
        let link = dir.join("link");
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let resolved = std::fs::canonicalize(real.join("kirie")).unwrap();
        assert_eq!(script_binary(&link), resolved);
        assert_eq!(
            kind_of(
                Some("method=script"),
                Some(&resolved),
                &script_binary(&link)
            ),
            InstallKind::Script { installed: None }
        );
        assert_eq!(script_binary(&dir.join("none")), dir.join("none/kirie"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn only_the_script_installed_binary_is_a_script_install() {
        let bin = Path::new("/home/me/.local/bin");
        let binary = bin.join("kirie");
        let marker = "method=script\nversion=0.2.0\n";
        assert_eq!(
            kind_of(Some(marker), Some(&binary), &binary),
            InstallKind::Script {
                installed: Some(Version::new(0, 2, 0))
            }
        );
        let elsewhere = Path::new("/home/me/src/kirie/target/release/kirie");
        assert_eq!(
            kind_of(Some(marker), Some(elsewhere), &binary),
            InstallKind::Other
        );
        assert_eq!(kind_of(None, Some(&binary), &binary), InstallKind::Other);
        assert_eq!(
            kind_of(Some("method=package"), Some(&binary), &binary),
            InstallKind::Other
        );
        assert_eq!(
            kind_of(Some("method=script"), Some(&binary), &binary),
            InstallKind::Script { installed: None }
        );
    }
}
