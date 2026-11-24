//! Updating an install made by `install.sh`, by running the new release's own `install.sh`. The
//! script and the tarball it installs are checked against the release's signed checksums first.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use semver::Version;

use crate::verify::Sums;
use crate::{AGENT, off_the_runtime, releases_url};

/// Downloads larger than this are refused.
const MAX_DOWNLOAD: u64 = 16 * 1024 * 1024;
const INSTALLER: &str = "install.sh";
const SUMS: &str = "SHA256SUMS";
/// The GNOME Shell extension's metadata, whose version changes with every release.
const METADATA: &str = "metadata.json";

/// What an update changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Installed {
    /// The GNOME Shell extension's code changed, which GNOME loads only at the next log-in.
    pub extension_changed: bool,
}

/// Downloads `install.sh` from the release and runs it for that version. `extension` is the
/// GNOME Shell extension's folder, watched for whether the update changed its code.
pub async fn install(version: Version, extension: Option<PathBuf>) -> Result<Installed, String> {
    off_the_runtime(move || install_blocking(&version, extension.as_deref(), false)).await
}

/// Like [`install`]; `shown` lets the installer print to the terminal instead of keeping what it
/// says for the error.
pub fn install_blocking(
    version: &Version,
    extension: Option<&Path>,
    shown: bool,
) -> Result<Installed, String> {
    let base = format!("{}/download/v{version}", releases_url());
    let download = |name: &str| fetch(&format!("{base}/{name}"), name);
    let text =
        |name: &str| String::from_utf8(download(name)?).map_err(|_| format!("{name} is not text"));
    let sums = Sums::verified(&text(SUMS)?, &text(&format!("{SUMS}.minisig"))?, version)?;
    let installer = download(INSTALLER)?;
    sums.check(INSTALLER, &installer)?;
    let tarball = sums.hash(&tarball(version)?)?;

    let before = extension.and_then(extension_code);
    // The script comes in on stdin, so it never sits in a shared temporary folder.
    let output = || {
        if shown {
            Stdio::inherit()
        } else {
            Stdio::piped()
        }
    };
    let mut child = installer_command(version, tarball)
        .stdin(Stdio::piped())
        .stdout(output())
        .stderr(output())
        .spawn()
        .map_err(|error| format!("cannot run sh: {error}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        stdin
            .write_all(&installer)
            .map_err(|error| format!("cannot run the installer: {error}"))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| format!("cannot run the installer: {error}"))?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    tracing::info!(%stdout, %stderr, status = %output.status, "ran install.sh");
    if !output.status.success() {
        return Err(failure_reason(&stderr, output.status));
    }
    let after = extension.and_then(extension_code);
    Ok(Installed {
        extension_changed: after.is_some() && before != after,
    })
}

/// `sh` reading `install.sh` from stdin to install `version`, whose tarball hashes to `tarball`.
/// Variables that would make the script install from a folder, skipping the download and its
/// check, or uninstall instead, are not passed on.
fn installer_command(version: &Version, tarball: &str) -> Command {
    let mut command = Command::new("sh");
    command
        .args(["-s", "--", "--force"])
        .env("KIRIE_VERSION", version.to_string())
        .env("KIRIE_SHA256", tarball)
        .env_remove("KIRIE_FROM")
        .env_remove("KIRIE_UNINSTALL");
    command
}

fn fetch(url: &str, name: &str) -> Result<Vec<u8>, String> {
    let mut body = Vec::new();
    AGENT
        .get(url)
        .call()
        .map_err(|error| format!("cannot download {name}: {error}"))?
        .into_body()
        .into_reader()
        .take(MAX_DOWNLOAD)
        .read_to_end(&mut body)
        .map_err(|error| format!("cannot download {name}: {error}"))?;
    Ok(body)
}

/// The tarball `install.sh` downloads for this machine.
fn tarball(version: &Version) -> Result<String, String> {
    match std::env::consts::ARCH {
        arch @ ("x86_64" | "aarch64") => Ok(format!("kirie-{version}-{arch}-linux.tar.gz")),
        other => Err(format!("there is no Kirie build for {other}")),
    }
}

/// The installer's own explanation, without its prefix, or else how it ended.
fn failure_reason(stderr: &str, status: std::process::ExitStatus) -> String {
    stderr
        .lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map_or_else(
            || format!("the installer stopped ({status})"),
            |line| {
                line.strip_prefix("kirie install: ")
                    .unwrap_or(line)
                    .to_owned()
            },
        )
}

/// The extension's files apart from `metadata.json`, whose version changes with every release.
fn extension_code(dir: &Path) -> Option<BTreeMap<PathBuf, Vec<u8>>> {
    fn collect(dir: &Path, root: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                collect(&path, root, files);
            } else if let Ok(relative) = path.strip_prefix(root)
                && relative != Path::new(METADATA)
                && let Ok(content) = std::fs::read(&path)
            {
                files.insert(relative.to_owned(), content);
            }
        }
    }
    if !dir.is_dir() {
        return None;
    }
    let mut files = BTreeMap::new();
    collect(dir, dir, &mut files);
    Some(files)
}

#[cfg(test)]
mod tests {
    use std::ffi::OsStr;
    use std::os::unix::process::ExitStatusExt as _;

    use super::*;

    #[test]
    fn a_failure_is_told_in_the_installers_words() {
        let failed = std::process::ExitStatus::from_raw(256);
        assert_eq!(
            failure_reason(
                "downloading\nkirie install: the download does not match its checksum\n\n",
                failed
            ),
            "the download does not match its checksum"
        );
        assert!(failure_reason("", failed).starts_with("the installer stopped"));
    }

    #[test]
    fn the_extension_code_leaves_out_its_metadata() {
        let dir = std::env::temp_dir().join(format!("kirie-extension-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("schemas")).unwrap();
        std::fs::write(dir.join("metadata.json"), "1").unwrap();
        std::fs::write(dir.join("extension.js"), "a").unwrap();
        std::fs::write(dir.join("schemas/x.xml"), "b").unwrap();
        let before = extension_code(&dir).unwrap();
        assert_eq!(before.len(), 2);
        std::fs::write(dir.join("metadata.json"), "2").unwrap();
        assert_eq!(extension_code(&dir).unwrap(), before);
        std::fs::write(dir.join("extension.js"), "c").unwrap();
        assert_ne!(extension_code(&dir).unwrap(), before);
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(extension_code(&dir).is_none());
    }

    #[test]
    fn the_installer_never_inherits_a_way_around_the_check() {
        let command = installer_command(&Version::new(1, 2, 3), "abc");
        let envs: BTreeMap<_, _> = command.get_envs().collect();
        let env = |name: &str| envs.get(OsStr::new(name)).copied();
        assert_eq!(env("KIRIE_FROM"), Some(None));
        assert_eq!(env("KIRIE_UNINSTALL"), Some(None));
        assert_eq!(env("KIRIE_VERSION"), Some(Some(OsStr::new("1.2.3"))));
        assert_eq!(env("KIRIE_SHA256"), Some(Some(OsStr::new("abc"))));
    }

    #[test]
    fn the_tarball_names_the_machine() {
        let name = tarball(&Version::new(1, 2, 3)).unwrap();
        assert!(name.starts_with("kirie-1.2.3-") && name.ends_with("-linux.tar.gz"));
    }
}
