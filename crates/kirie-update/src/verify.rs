//! The release's checksums, trusted once their signature checks out against the release key.

use std::collections::BTreeMap;

use minisign_verify::{PublicKey, Signature};
use semver::Version;
use sha2::{Digest, Sha256};

/// The release key's public half: the second line of packaging/minisign.pub. CI checks that the
/// two match.
const RELEASE_KEY: &str = "RWQnRRohusKyNk1BV+AejdPK/q7tZpn//TYfCMQyIpBQACHZapjioxdi";

/// The checksums of one release, after their signature was checked. The signature's trusted
/// comment names the version, so an older release's signed checksums cannot stand in for a
/// newer one's.
#[derive(Debug)]
pub struct Sums(BTreeMap<String, String>);

impl Sums {
    pub fn verified(sums: &str, signature: &str, version: &Version) -> Result<Self, String> {
        Self::verified_with(RELEASE_KEY, sums, signature, version)
    }

    fn verified_with(
        key: &str,
        sums: &str,
        signature: &str,
        version: &Version,
    ) -> Result<Self, String> {
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

    pub fn hash(&self, name: &str) -> Result<&str, String> {
        self.0
            .get(name)
            .map(String::as_str)
            .ok_or_else(|| format!("{name} is missing from the release's checksums"))
    }

    pub fn check(&self, name: &str, content: &[u8]) -> Result<(), String> {
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
    fn release_key_parses() {
        assert!(PublicKey::from_base64(RELEASE_KEY).is_ok());
    }

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
        let sums = Sums::verified_with(TEST_KEY, TEST_SUMS, TEST_SIGNATURE, &Version::new(1, 2, 3))
            .unwrap();
        assert!(sums.check("install.sh", b"echo hi").is_ok());
        let replayed =
            Sums::verified_with(TEST_KEY, TEST_SUMS, TEST_SIGNATURE, &Version::new(1, 2, 4));
        assert!(replayed.unwrap_err().contains("kirie 1.2.3"));
        let tampered = TEST_SUMS.replace("install.sh", "install.sh.evil");
        assert!(
            Sums::verified_with(TEST_KEY, &tampered, TEST_SIGNATURE, &Version::new(1, 2, 3))
                .is_err()
        );
    }

    #[test]
    fn unsigned_checksums_are_refused() {
        let version = Version::new(1, 2, 3);
        assert!(Sums::verified_with("not a key", "", "", &version).is_err());
        assert!(Sums::verified(TEST_SUMS, TEST_SIGNATURE, &version).is_err());
        let sums = Sums(parse(&format!("{}  install.sh", sha256_hex(b"echo hi"))));
        assert!(sums.check("install.sh", b"echo hi").is_ok());
        assert!(sums.check("install.sh", b"rm -rf /").is_err());
        assert!(sums.hash("missing").is_err());
    }
}
