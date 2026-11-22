//! `kirie self-update`: installs the latest release after checking that its checksums are signed
//! by the release key, the same way the install script does when minisign is installed.

use std::collections::BTreeMap;
use std::io::{Read as _, Write as _};
use std::process::{Command, Stdio};

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use sha2::{Digest, Sha256};

const REPO: &str = "samuelba/kirie";
/// The release key's public half: the second line of packaging/minisign.pub. CI checks that the
/// two match. Empty until the key exists, and then nothing can be updated.
const RELEASE_KEY: &str = "";
/// Downloads larger than this are refused.
const MAX_DOWNLOAD: u64 = 16 * 1024 * 1024;

pub fn run() -> anyhow::Result<()> {
    anyhow::ensure!(
        !RELEASE_KEY.is_empty(),
        "this build of Kirie has no release key, so it cannot check an update; reinstall with the install script"
    );
    let marker = directories::BaseDirs::new()
        .map(|dirs| dirs.data_dir().join("kirie/install-method"))
        .ok_or_else(|| anyhow::anyhow!("cannot find the data folder"))?;
    let method = std::fs::read_to_string(&marker).unwrap_or_default();
    anyhow::ensure!(
        method.lines().any(|line| line == "method=script"),
        "Kirie was not installed with the install script; update it the way it was installed"
    );

    let current = Version::parse(env!("CARGO_PKG_VERSION"))?;
    let latest = latest()?;
    if latest <= current {
        println!("Kirie {current} is the latest version.");
        return Ok(());
    }
    println!("Updating Kirie {current} → {latest}");
    let base = format!("https://github.com/{REPO}/releases/download/v{latest}");
    let sums = text(&format!("{base}/SHA256SUMS"))?;
    let signature = text(&format!("{base}/SHA256SUMS.minisig"))?;
    let sums =
        Sums::verified(RELEASE_KEY, &sums, &signature, &latest).map_err(anyhow::Error::msg)?;
    let installer = bytes(&format!("{base}/install.sh"))?;
    sums.check("install.sh", &installer)
        .map_err(anyhow::Error::msg)?;
    let arch = match std::env::consts::ARCH {
        "x86_64" => "x86_64",
        "aarch64" => "aarch64",
        other => anyhow::bail!("there is no Kirie build for {other}"),
    };
    let tarball = sums
        .hash(&format!("kirie-{latest}-{arch}-linux.tar.gz"))
        .map_err(anyhow::Error::msg)?;

    let mut shell = Command::new("sh")
        .args(["-s", "--", "--force"])
        .env("KIRIE_VERSION", latest.to_string())
        .env("KIRIE_SHA256", tarball)
        .stdin(Stdio::piped())
        .spawn()?;
    shell
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("cannot start the installer"))?
        .write_all(&installer)?;
    anyhow::ensure!(shell.wait()?.success(), "the installer failed");
    Ok(())
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .user_agent(concat!("kirie/", env!("CARGO_PKG_VERSION")))
        .build()
        .into()
}

fn latest() -> anyhow::Result<Version> {
    #[derive(serde::Deserialize)]
    struct Release {
        tag_name: String,
    }
    let release: Release = agent()
        .get(format!(
            "https://api.github.com/repos/{REPO}/releases/latest"
        ))
        .header("Accept", "application/vnd.github+json")
        .call()?
        .body_mut()
        .read_json()?;
    Ok(Version::parse(release.tag_name.trim_start_matches('v'))?)
}

fn bytes(url: &str) -> anyhow::Result<Vec<u8>> {
    let mut body = Vec::new();
    agent()
        .get(url)
        .call()?
        .into_body()
        .into_reader()
        .take(MAX_DOWNLOAD)
        .read_to_end(&mut body)?;
    Ok(body)
}

fn text(url: &str) -> anyhow::Result<String> {
    Ok(String::from_utf8(bytes(url)?)?)
}

/// The checksums of one release, after their signature was checked. The signature's trusted
/// comment names the version, so an older release's signed checksums cannot stand in for a
/// newer one's.
#[derive(Debug)]
struct Sums(BTreeMap<String, String>);

impl Sums {
    fn verified(key: &str, sums: &str, signature: &str, version: &Version) -> Result<Self, String> {
        let key = PublicKey::from_base64(key.trim())
            .map_err(|error| format!("bad release key: {error}"))?;
        let signature = Signature::decode(signature)
            .map_err(|error| format!("the release's signature is unreadable: {error}"))?;
        key.verify(sums.as_bytes(), &signature, false)
            .map_err(|_| "the release's checksums are not signed by the release key".to_owned())?;
        let expected = format!("kirie {version}");
        if signature.trusted_comment() != expected {
            return Err(format!(
                "the release's signature is for {:?}, not {expected:?}",
                signature.trusted_comment()
            ));
        }
        Ok(Self(parse(sums)))
    }

    fn hash(&self, name: &str) -> Result<&str, String> {
        self.0
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("{name} is missing from the release's checksums"))
    }

    fn check(&self, name: &str, content: &[u8]) -> Result<(), String> {
        if sha256_hex(content) == self.hash(name)? {
            Ok(())
        } else {
            Err(format!("{name} does not match its checksum"))
        }
    }
}

/// `sha256sum` lines: the hash, a space, then a space (text) or `*` (binary) and the name.
fn parse(sums: &str) -> BTreeMap<String, String> {
    sums.lines()
        .filter_map(|line| {
            let (hash, name) = line.split_once(' ')?;
            let name = name.strip_prefix([' ', '*'])?;
            (hash.len() == 64 && hash.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .then(|| (name.to_owned(), hash.to_ascii_lowercase()))
        })
        .collect()
}

fn sha256_hex(content: &[u8]) -> String {
    use std::fmt::Write as _;
    Sha256::digest(content)
        .iter()
        .fold(String::with_capacity(64), |mut hex, byte| {
            let _ = write!(hex, "{byte:02x}");
            hex
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checksum_lines_parse() {
        let hash = "a".repeat(64);
        let sums = parse(&format!(
            "{hash}  install.sh\n{hash} *kirie.tar.gz\nnot a line\n{}  short\n",
            "b".repeat(10)
        ));
        assert_eq!(sums.len(), 2);
        assert_eq!(sums["kirie.tar.gz"], hash);
    }

    #[test]
    fn hashes_are_lowercase_hex() {
        assert_eq!(
            sha256_hex(b""),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    /// A throwaway key made for this test with `rsign generate`, not the release key.
    const TEST_KEY: &str = "RWQ44HVk6N45X/h2xTqmvAHkisNXBn3tzqFWGRLHkCvuPq9AIGhPzZ9v";
    const TEST_SUMS: &str =
        "56a79f3b115448072387c2480044bfa2cf8f90e4f5fddd8c943b4e051b81f80b  install.sh\n";
    const TEST_SIGNATURE: &str = "untrusted comment: signature from rsign secret key
RUQ44HVk6N45X2hjvEO86oy6oTjG22nXUlecvyVyhq7QoVlhVf3RlEfChcsxa/qdqMhWbWThX47q9XZV95VRcC3Bq14rz33eVwU=
trusted comment: kirie 1.2.3
Rdk4yywPgAzrCKhA270nNRIKDU1B52DdfsPXZzIirc1jaqlOqiQMGnHgeVYg2aLduiYMB+1+cyFOxYK7thu0DA==
";

    #[test]
    fn signed_checksums_are_accepted_for_their_version_only() {
        let sums =
            Sums::verified(TEST_KEY, TEST_SUMS, TEST_SIGNATURE, &Version::new(1, 2, 3)).unwrap();
        assert!(sums.check("install.sh", b"echo hi").is_ok());
        let replayed = Sums::verified(TEST_KEY, TEST_SUMS, TEST_SIGNATURE, &Version::new(1, 2, 4));
        assert!(replayed.unwrap_err().contains("kirie 1.2.3"));
        let tampered = TEST_SUMS.replace("install.sh", "install.sh.evil");
        assert!(
            Sums::verified(TEST_KEY, &tampered, TEST_SIGNATURE, &Version::new(1, 2, 3)).is_err()
        );
    }

    #[test]
    fn unsigned_checksums_are_refused() {
        let version = Version::new(1, 2, 3);
        assert!(Sums::verified("not a key", "", "", &version).is_err());
        let sums = Sums(parse(&format!("{}  install.sh", sha256_hex(b"echo hi"))));
        assert!(sums.check("install.sh", b"echo hi").is_ok());
        assert!(sums.check("install.sh", b"rm -rf /").is_err());
        assert!(sums.hash("missing").is_err());
    }
}
