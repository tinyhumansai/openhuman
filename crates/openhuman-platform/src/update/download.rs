//! Verified download of a release asset.
//!
//! Each core archive is published beside a `.sha256` sidecar holding its
//! SHA-256 digest (`scripts/release/package-cli-tarball.sh`). The staging path
//! fetches that sidecar before the archive, streams the archive to disk under a
//! size cap while hashing it, and refuses anything whose digest differs. A
//! missing or malformed sidecar is a refusal too: the check fails closed.
//!
//! The release does not sign the core archive itself. The only pinned key in
//! the repository is the Tauri updater's minisign key, which covers the desktop
//! bundles, so there is no signature to verify here.

use std::io::Write;

use sha2::{Digest, Sha256};

/// Upper bound on an update body when the release metadata declared no size,
/// as with a caller-supplied URL on `update.apply`.
pub(super) const MAX_UPDATE_BYTES: u64 = 512 * 1024 * 1024;

/// Upper bound on a `.sha256` sidecar. The real file is 65 bytes.
const MAX_CHECKSUM_BYTES: u64 = 1024;

/// URL of the `.sha256` sidecar published beside `download_url`.
///
/// The suffix is appended to the URL path, so a query string stays intact.
pub(super) fn checksum_url(download_url: &str) -> Result<String, String> {
    let mut url = url::Url::parse(download_url)
        .map_err(|e| format!("invalid download URL for checksum lookup: {e}"))?;
    let path = format!("{}.sha256", url.path());
    url.set_path(&path);
    Ok(url.to_string())
}

/// Parse a sidecar body into a digest.
///
/// Accepts the bare hex digest the release writes and the `sha256sum` form
/// (`<hex>  <file name>`); only the first token is read.
pub(super) fn parse_checksum(body: &str) -> Result<[u8; 32], String> {
    let token = body.split_whitespace().next().unwrap_or("");
    let bytes = hex::decode(token).map_err(|_| "checksum sidecar is not hex".to_string())?;
    <[u8; 32]>::try_from(bytes.as_slice())
        .map_err(|_| format!("checksum sidecar holds {} bytes, not 32", bytes.len()))
}

/// Fetch and parse the `.sha256` sidecar for `download_url`.
pub(super) async fn fetch_expected_sha256(
    client: &reqwest::Client,
    download_url: &str,
) -> Result<[u8; 32], String> {
    let url = checksum_url(download_url)?;
    log::debug!("[update] fetching checksum sidecar {url}");
    let mut response = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("failed to fetch checksum sidecar: {e}"))?;
    if !response.status().is_success() {
        return Err(format!(
            "checksum sidecar unavailable (status {}); refusing an unverifiable update",
            response.status()
        ));
    }
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("failed to read checksum sidecar: {e}"))?
    {
        if (body.len() + chunk.len()) as u64 > MAX_CHECKSUM_BYTES {
            return Err("checksum sidecar is larger than a digest".to_string());
        }
        body.extend_from_slice(&chunk);
    }
    let text = std::str::from_utf8(&body).map_err(|_| "checksum sidecar is not text")?;
    parse_checksum(text)
}

/// Refuse a response whose declared length already exceeds `max_bytes`.
pub(super) fn check_declared_length(
    response: &reqwest::Response,
    max_bytes: u64,
) -> Result<(), String> {
    match response.content_length() {
        Some(len) if len > max_bytes => Err(oversize_message(max_bytes)),
        _ => Ok(()),
    }
}

/// Stream `response` into `file` (a freshly created temp file), hashing as it
/// goes.
///
/// Stops reading once more than `max_bytes` arrive, without writing the
/// excess; the returned count then exceeds `max_bytes` and `verify` refuses
/// it. Returns the byte count and the SHA-256 digest of what was read. The
/// caller creates the file and removes it on error.
pub(super) async fn stream_to_file(
    mut response: reqwest::Response,
    file: &mut std::fs::File,
    max_bytes: u64,
) -> Result<(u64, [u8; 32]), String> {
    let mut hasher = Sha256::new();
    let mut read: u64 = 0;
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|e| format!("failed to read update body: {e}"))?
    {
        read += chunk.len() as u64;
        if read > max_bytes {
            break;
        }
        hasher.update(&chunk);
        file.write_all(&chunk)
            .map_err(|e| format!("failed to write update binary: {e}"))?;
    }
    file.flush()
        .map_err(|e| format!("failed to flush update binary: {e}"))?;
    Ok((read, hasher.finalize().into()))
}

/// Accept a streamed body only if it fits the cap, matches the declared size
/// when there is one, and hashes to the sidecar's digest.
pub(super) fn verify(
    read: u64,
    digest: &[u8; 32],
    max_bytes: u64,
    expected_size: Option<u64>,
    expected_sha256: &[u8; 32],
) -> Result<(), String> {
    if read > max_bytes {
        return Err(oversize_message(max_bytes));
    }
    if let Some(size) = expected_size.filter(|size| *size != read) {
        return Err(format!(
            "update body is {read} bytes, the release declared {size}"
        ));
    }
    if digest != expected_sha256 {
        return Err(format!(
            "update checksum mismatch: expected {}, got {}",
            hex::encode(expected_sha256),
            hex::encode(digest)
        ));
    }
    Ok(())
}

fn oversize_message(max_bytes: u64) -> String {
    format!("update body exceeds the declared size of {max_bytes} bytes")
}

#[cfg(test)]
#[path = "download_tests.rs"]
mod tests;
