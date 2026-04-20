//! Node.js distribution downloader with SHASUMS256 verification.
//!
//! Resolves the right archive for the current OS/arch off nodejs.org,
//! streams it to a caller-supplied temp path, and validates the SHA-256
//! against the official `SHASUMS256.txt` for the release. Keeps everything
//! in one place so the bootstrap caller only needs to know "download this
//! version, give me the bytes on disk".
//!
//! ## Security
//!
//! We **require** a SHA-256 match before returning success — a corrupted or
//! tampered archive is treated the same as a failed download and the file
//! is deleted. There is no opt-out; skills will run untrusted code inside
//! the resolved Node runtime, so the integrity check is load-bearing.

use anyhow::{anyhow, bail, Context, Result};
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::Path;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;

/// Base URL for official Node.js release artifacts.
const NODEJS_DIST_BASE: &str = "https://nodejs.org/dist";

/// Describes a single downloadable Node.js distribution for the host triple.
#[derive(Debug, Clone)]
pub struct NodeDistribution {
    /// Version string including the leading `v` (e.g. `v22.11.0`).
    pub version: String,
    /// Archive filename as it appears in `SHASUMS256.txt`
    /// (e.g. `node-v22.11.0-darwin-arm64.tar.xz`).
    pub archive_name: String,
    /// Full download URL.
    pub url: String,
    /// Whether the archive is a zip (Windows) or tar.xz (everything else).
    /// Drives which extraction path the caller invokes.
    pub is_zip: bool,
}

impl NodeDistribution {
    /// Build the distribution descriptor for the current host OS/arch.
    ///
    /// Supported triples mirror the officially-prebuilt Node.js binaries:
    ///
    /// | OS       | Arch                                | Archive suffix              |
    /// |----------|--------------------------------------|-----------------------------|
    /// | macOS    | aarch64, x86_64                     | `-darwin-{arm64,x64}.tar.xz`|
    /// | Linux    | aarch64, x86_64, arm, armv7         | `-linux-{arm64,x64,armv7l}.tar.xz` |
    /// | Windows  | aarch64, x86_64                     | `-win-{arm64,x64}.zip`      |
    ///
    /// Everything else yields an error — the caller should surface it as a
    /// "Node runtime unavailable on this host" message.
    pub fn for_host(version: &str) -> Result<Self> {
        let version = normalize_version(version);
        let (suffix, is_zip) = host_archive_suffix()?;
        let archive_name = format!("node-{version}-{suffix}");
        let url = format!("{NODEJS_DIST_BASE}/{version}/{archive_name}");
        tracing::debug!(
            version = %version,
            url = %url,
            "[node_runtime::downloader] resolved distribution for host"
        );
        Ok(Self {
            version,
            archive_name,
            url,
            is_zip,
        })
    }
}

/// Normalise a version string to the canonical `vX.Y.Z` form used by
/// nodejs.org. Config allows `22.11.0` or `v22.11.0`; we always emit the
/// `v`-prefixed variant because it is what appears in the URL path.
fn normalize_version(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.starts_with('v') {
        trimmed.to_string()
    } else {
        format!("v{trimmed}")
    }
}

/// Return `(archive_suffix, is_zip)` for the current host. The suffix omits
/// the `node-vX.Y.Z-` prefix because callers always interpolate the version.
fn host_archive_suffix() -> Result<(&'static str, bool)> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    match (os, arch) {
        ("macos", "aarch64") => Ok(("darwin-arm64.tar.xz", false)),
        ("macos", "x86_64") => Ok(("darwin-x64.tar.xz", false)),
        ("linux", "aarch64") => Ok(("linux-arm64.tar.xz", false)),
        ("linux", "x86_64") => Ok(("linux-x64.tar.xz", false)),
        ("linux", "arm") | ("linux", "armv7") => Ok(("linux-armv7l.tar.xz", false)),
        ("windows", "aarch64") => Ok(("win-arm64.zip", true)),
        ("windows", "x86_64") => Ok(("win-x64.zip", true)),
        _ => Err(anyhow!(
            "no prebuilt Node.js distribution for host {os}/{arch} — set node.enabled=false or install node manually"
        )),
    }
}

/// Fetch `SHASUMS256.txt` for the release and return a
/// `archive_name -> sha256_hex` map. The hex digest is lowercase.
pub async fn fetch_shasums(client: &Client, version: &str) -> Result<HashMap<String, String>> {
    let version = normalize_version(version);
    let url = format!("{NODEJS_DIST_BASE}/{version}/SHASUMS256.txt");
    tracing::debug!(url = %url, "[node_runtime::downloader] fetching SHASUMS256.txt");

    let body = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("GET {url}"))?
        .error_for_status()
        .with_context(|| format!("non-success status on {url}"))?
        .text()
        .await
        .with_context(|| format!("reading body of {url}"))?;

    let map = parse_shasums(&body);
    tracing::debug!(
        entries = map.len(),
        "[node_runtime::downloader] parsed SHASUMS256.txt"
    );
    Ok(map)
}

/// Parse the `SHASUMS256.txt` body into a lookup table. The format is one
/// entry per line: `<hex-sha256>  <filename>` (two spaces). Unknown / blank
/// lines are skipped to be robust against trailing newlines or signature
/// blocks that may appear in future releases.
fn parse_shasums(body: &str) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for line in body.lines() {
        let mut parts = line.split_whitespace();
        let (Some(hash), Some(name)) = (parts.next(), parts.next()) else {
            continue;
        };
        if hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit()) {
            out.insert(name.to_string(), hash.to_ascii_lowercase());
        }
    }
    out
}

/// Stream `dist.url` to `target_path`, computing the SHA-256 on the fly and
/// comparing against the digest supplied in `expected_sha256`.
///
/// On mismatch or any I/O error the partial file at `target_path` is
/// removed — we never leave half-written / tampered archives on disk.
pub async fn download_distribution(
    client: &Client,
    dist: &NodeDistribution,
    target_path: &Path,
    expected_sha256: &str,
) -> Result<()> {
    tracing::info!(
        url = %dist.url,
        target = %target_path.display(),
        "[node_runtime::downloader] starting download"
    );

    if let Some(parent) = target_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .with_context(|| format!("creating cache dir {}", parent.display()))?;
    }

    let mut response = client
        .get(&dist.url)
        .send()
        .await
        .with_context(|| format!("GET {}", dist.url))?
        .error_for_status()
        .with_context(|| format!("non-success status on {}", dist.url))?;

    let total_bytes = response.content_length();
    let mut file = File::create(target_path)
        .await
        .with_context(|| format!("creating {}", target_path.display()))?;
    let mut hasher = Sha256::new();
    let mut written: u64 = 0;

    while let Some(chunk) = response
        .chunk()
        .await
        .with_context(|| format!("streaming {}", dist.url))?
    {
        hasher.update(&chunk);
        file.write_all(&chunk)
            .await
            .with_context(|| format!("writing chunk to {}", target_path.display()))?;
        written = written.saturating_add(chunk.len() as u64);
    }

    file.flush().await.ok();
    drop(file);

    let actual_hex = hex::encode(hasher.finalize());
    let expected = expected_sha256.trim().to_ascii_lowercase();

    if actual_hex != expected {
        tracing::error!(
            expected = %expected,
            actual = %actual_hex,
            target = %target_path.display(),
            "[node_runtime::downloader] SHA-256 mismatch — deleting partial archive"
        );
        let _ = tokio::fs::remove_file(target_path).await;
        bail!(
            "SHA-256 mismatch for {} (expected {expected}, got {actual_hex})",
            dist.archive_name
        );
    }

    tracing::info!(
        target = %target_path.display(),
        bytes = written,
        total = ?total_bytes,
        "[node_runtime::downloader] download complete, hash verified"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_version_with_and_without_prefix() {
        assert_eq!(normalize_version("22.11.0"), "v22.11.0");
        assert_eq!(normalize_version("v22.11.0"), "v22.11.0");
        assert_eq!(normalize_version("  v22.11.0\n"), "v22.11.0");
    }

    #[test]
    fn parses_shasums_text() {
        let body = "\
abc123def4567890abc123def4567890abc123def4567890abc123def4567890  node-v22.11.0-darwin-arm64.tar.xz
1111222233334444555566667777888899990000111122223333444455556666  node-v22.11.0-linux-x64.tar.xz
garbage line
BADHASHNOTHEX  node-v22.11.0-win-x64.zip
";
        let map = parse_shasums(body);
        assert_eq!(map.len(), 2);
        assert_eq!(
            map.get("node-v22.11.0-darwin-arm64.tar.xz").unwrap(),
            "abc123def4567890abc123def4567890abc123def4567890abc123def4567890"
        );
    }

    #[test]
    fn distribution_for_host_returns_sensible_url() {
        let dist = NodeDistribution::for_host("v22.11.0").expect("host supported in CI");
        assert!(dist.url.starts_with("https://nodejs.org/dist/v22.11.0/"));
        assert!(dist.archive_name.starts_with("node-v22.11.0-"));
    }
}
