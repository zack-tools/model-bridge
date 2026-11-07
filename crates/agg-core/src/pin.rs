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

/// Verify both the platform archive and the executable extracted from it.
/// The executable must match the exact member of the checksum-pinned archive.
pub fn verify_native_binary(archive: &Path, binary: &Path) -> Result<String, String> {
    let archive_name = match (std::env::consts::OS, std::env::consts::ARCH) {
        ("linux", "x86_64") => LINUX_AMD64_ARCHIVE,
        ("macos", "aarch64") => "CLIProxyAPI_8.0.57_darwin_aarch64.tar.gz",
        ("macos", "x86_64") => "CLIProxyAPI_8.0.57_darwin_amd64.tar.gz",
        ("windows", "x86_64") => "CLIProxyAPI_8.0.57_windows_amd64.zip",
        ("windows", "aarch64") => "CLIProxyAPI_8.0.57_windows_aarch64.zip",
        _ => return Err("no verified native archive mapping for this platform".into()),
    };
    verify_archive(archive, archive_name)?;
    let binary_sha = sha256_file(binary)?;
    let archive_binary_sha = if archive_name.ends_with(".zip") {
        zip_binary_sha256(archive)?
    } else {
        let extracted = std::process::Command::new("tar")
            .arg("-xOf").arg(archive).arg("cli-proxy-api").output()
            .map_err(|e| format!("extract pinned binary: {e}"))?;
        if !extracted.status.success() { return Err("could not read binary from pinned archive".into()); }
        sha256_bytes(&extracted.stdout)
    };
    if archive_binary_sha != binary_sha { return Err("binary does not match pinned archive".into()); }
    Ok(binary_sha)
}

fn zip_binary_sha256(archive: &Path) -> Result<String, String> {
    use std::io::Read;
    let file = std::fs::File::open(archive).map_err(|e| format!("open archive: {e}"))?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| format!("read ZIP: {e}"))?;
    let mut matches = 0;
    let mut digest = None;
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index).map_err(|e| format!("read ZIP member: {e}"))?;
        if entry.name() != "cli-proxy-api.exe" { continue; }
        matches += 1;
        if !entry.is_file() || entry.size() > 512 * 1024 * 1024 { return Err("invalid ZIP binary".into()); }
        let mut hasher = Sha256::new();
        let mut buffer = [0u8; 32768];
        loop {
            let count = entry.read(&mut buffer).map_err(|e| format!("read ZIP binary: {e}"))?;
            if count == 0 { break; }
            hasher.update(&buffer[..count]);
        }
        digest = Some(hex_encode(&hasher.finalize()));
    }
    if matches != 1 { return Err("ZIP must contain exactly one cli-proxy-api.exe".into()); }
    digest.ok_or_else(|| "ZIP binary missing".into())
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
    fn zip_requires_exactly_one_expected_binary() {
        use std::io::Write;
        let mut nonce = [0u8; 8];
        getrandom::getrandom(&mut nonce).unwrap();
        let path = std::env::temp_dir().join(format!("agg-zip-{}.zip", sha256_bytes(&nonce)));
        let write_zip = |name: &str| {
            let mut writer = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
            writer.start_file(name, zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated)).unwrap();
            writer.write_all(b"fixture binary").unwrap();
            writer.finish().unwrap();
        };
        write_zip("cli-proxy-api.exe");
        assert_eq!(zip_binary_sha256(&path).unwrap(), sha256_bytes(b"fixture binary"));
        write_zip("other.exe");
        assert!(zip_binary_sha256(&path).is_err());
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    #[ignore = "requires separately downloaded pinned Windows x64 archive"]
    fn windows_x64_release_zip_matches_pinned_archive() {
        let path = std::env::var_os("APP_AGG_TEST_WINDOWS_ZIP").expect("archive path required");
        let path = Path::new(&path);
        verify_archive(path, "CLIProxyAPI_8.0.57_windows_amd64.zip").unwrap();
        assert_eq!(zip_binary_sha256(path).unwrap(), "49a597a0f8cea758dcf17e94d9468be62f4552b1a8f9c5d538045d8d2cdb7d80");
    }

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
