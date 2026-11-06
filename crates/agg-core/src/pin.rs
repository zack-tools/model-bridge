//! Pinned CLIProxyAPI release metadata compiled into the crate.

use sha2::{Digest, Sha256};
use std::path::Path;

pub const PIN_JSON: &str = include_str!("../../../vendor/cliproxyapi/PIN.json");
pub const CHECKSUMS: &str = include_str!("../../../vendor/cliproxyapi/checksums.txt");

pub const CORE_VERSION: &str = "8.0.57";
pub const CORE_GIT_SHA: &str = "c2f402b45643f0740d70b9010fd7d8d0aab7ef65";
pub const LINUX_AMD64_ARCHIVE: &str = "CLIProxyAPI_8.0.57_linux_amd64.tar.gz";
pub const LINUX_AMD64_SHA256: &str =
    "1e7b31c4bdbfeb91947fbb686933c36234ffa84c1beb781c7c020c035763545e";

#[derive(Debug, Clone, serde::Serialize)]
pub struct PinnedCore {
    pub version: &'static str,
    pub git_sha: &'static str,
    pub linux_amd64_archive: &'static str,
    pub linux_amd64_sha256: &'static str,
}

pub fn pinned_core() -> PinnedCore {
    PinnedCore {
        version: CORE_VERSION,
        git_sha: CORE_GIT_SHA,
        linux_amd64_archive: LINUX_AMD64_ARCHIVE,
        linux_amd64_sha256: LINUX_AMD64_SHA256,
    }
}

pub fn sha256_bytes(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex_encode(&hasher.finalize())
}

pub fn sha256_file(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(sha256_bytes(&bytes))
}

pub fn expected_sha256(archive_name: &str) -> Option<&'static str> {
    for line in CHECKSUMS.lines() {
        let mut parts = line.split_whitespace();
        let sha = parts.next()?;
        let name = parts.next()?;
        if name == archive_name {
            return Some(sha);
        }
    }
    None
}

pub fn verify_archive(path: &Path, archive_name: &str) -> Result<String, String> {
    let expected = expected_sha256(archive_name)
        .ok_or_else(|| format!("archive {archive_name} is not in the pinned checksum list"))?;
    let actual = sha256_file(path)?;
    if !actual.eq_ignore_ascii_case(expected) {
        return Err(format!(
            "checksum mismatch for {archive_name}: expected {expected} actual {actual}"
        ));
    }
    Ok(actual)
}

fn hex_encode(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_json_matches_constants() {
        let v: serde_json::Value = serde_json::from_str(PIN_JSON).unwrap();
        assert_eq!(v["version"], CORE_VERSION);
        assert_eq!(v["git_sha"], CORE_GIT_SHA);
        assert_eq!(v["evaluated_here"], LINUX_AMD64_ARCHIVE);
        assert_eq!(
            expected_sha256(LINUX_AMD64_ARCHIVE),
            Some(LINUX_AMD64_SHA256)
        );
        assert!(PIN_JSON.contains("NewFileTokenStore"));
    }
}
