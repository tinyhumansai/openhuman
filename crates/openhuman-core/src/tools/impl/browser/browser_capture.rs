//! Pictures of the page the browser tool saves by itself (`[browser]
//! screenshot_dir`).
//!
//! A host that wants to show what a browser run did (a post, a report) needs
//! a picture of the pages it reached. Leaving that to the model fails: it
//! forgets, or its toolset has no way to write an image. So when
//! `screenshot_dir` is set the tool captures the page itself after it opens
//! one, after a click, key press, select or check, and before it closes a
//! session, and writes each picture as `<unix ms>-<tag>.png` (or `.jpeg`) into
//! that folder. The host collects them from there.
//!
//! A capture never fails the call it follows: whatever goes wrong is logged
//! and the picture is skipped.
//!
//! Passwords: a password the keychain typed (`browser_keychain`) is masked
//! in every reply, but a picture cannot be masked. Before a capture, while
//! any keychain password has been typed lately, the page's accessibility
//! tree and text are read and the capture is skipped when either shows one
//! of those passwords (a revealed password field, an echo of it). A masked
//! password field shows dots, not the password, and is captured. A fill or
//! type is never followed by a capture.

use std::path::{Path, PathBuf};

use tinycomputer_bus::browser::{
    ImageFormat, ReadRequest, ScreenshotRequest, SessionId, SnapshotRequest,
};

use crate::modules::browser::BrowserClient;

/// The largest picture saved: X takes a still image up to 5 MB. A larger
/// PNG is taken again as a JPEG; a larger JPEG is skipped.
pub(crate) const MAX_PICTURE_BYTES: usize = 5 * 1024 * 1024;
/// JPEG quality for a page whose PNG is too large.
const JPEG_QUALITY: u8 = 80;
/// The most pictures kept in the folder; the oldest go first. A host that
/// never collects them still holds a bounded folder.
pub(crate) const MAX_KEPT: usize = 24;

/// The direct actions a picture follows: the ones that change what the page
/// shows. Never `fill` or `type`, which leave typed text on the page.
const CAPTURED_AFTER: &[&str] = &["click", "double_click", "press", "select", "check"];

/// Whether a direct browser action of `kind` (its TinyComputer wire name) is
/// followed by a picture.
pub(super) fn follows(kind: &str) -> bool {
    CAPTURED_AFTER.contains(&kind)
}

/// The folder pictures go to, when `[browser] screenshot_dir` names an
/// absolute one.
pub(super) fn folder(client: &BrowserClient) -> Option<PathBuf> {
    let dir = client.config().browser.screenshot_dir.as_deref()?.trim();
    let path = Path::new(dir);
    (!dir.is_empty() && path.is_absolute()).then(|| path.to_path_buf())
}

/// Whether any of `texts` shows a password the keychain typed lately.
pub(super) fn shows_filled_password(texts: &[&str]) -> bool {
    texts
        .iter()
        .any(|text| super::keychain::redact_filled(text) != *text)
}

/// Captures the session's page into the configured folder, if one is set.
/// Never fails: a picture that cannot be taken is logged and skipped.
pub(super) async fn capture(client: &BrowserClient, session: &SessionId, after: &str) {
    let Some(dir) = folder(client) else {
        return;
    };
    match capture_into(client, session, &dir).await {
        Ok(Some(path)) => tracing::debug!(
            after,
            file = %path.file_name().unwrap_or_default().to_string_lossy(),
            "[browser-capture] saved a picture of the page"
        ),
        Ok(None) => {}
        Err(error) => tracing::warn!(
            after,
            error = %super::keychain::redact_filled(&error.to_string()),
            "[browser-capture] could not save a picture of the page"
        ),
    }
}

async fn capture_into(
    client: &BrowserClient,
    session: &SessionId,
    dir: &Path,
) -> anyhow::Result<Option<PathBuf>> {
    if super::keychain::any_filled() {
        let tree = client
            .snapshot(
                session,
                SnapshotRequest {
                    compact: false,
                    max_chars: 200_000,
                    ..SnapshotRequest::default()
                },
            )
            .await?;
        let page = client
            .read_page(
                session,
                ReadRequest {
                    max_chars: 200_000,
                    ..ReadRequest::default()
                },
            )
            .await?;
        if shows_filled_password(&[&tree.tree, &page.content]) {
            tracing::info!(
                "[browser-capture] skipped: the page shows a password the keychain typed"
            );
            return Ok(None);
        }
    }
    let mut format = ImageFormat::Png;
    let mut bytes = client
        .screenshot(session, ScreenshotRequest::default())
        .await?;
    if bytes.len() > MAX_PICTURE_BYTES {
        format = ImageFormat::Jpeg;
        bytes = client
            .screenshot(
                session,
                ScreenshotRequest {
                    format,
                    quality: Some(JPEG_QUALITY),
                    ..ScreenshotRequest::default()
                },
            )
            .await?;
    }
    if bytes.len() > MAX_PICTURE_BYTES {
        tracing::info!(
            bytes = bytes.len(),
            "[browser-capture] skipped: the picture is over 5 MB even as a JPEG"
        );
        return Ok(None);
    }
    let extension = match format {
        ImageFormat::Jpeg => "jpeg",
        _ => "png",
    };
    save(dir, &bytes, extension).map(Some)
}

/// The next picture's name: the time in milliseconds, so names sort in the
/// order the pictures were taken, then a random tag that keeps two taken in
/// the same millisecond apart.
pub(super) fn file_name(unix_ms: u128, tag: &str, extension: &str) -> String {
    format!("{unix_ms:013}-{tag}.{extension}")
}

/// Writes `bytes` as the folder's next picture (whole, or not at all), then
/// drops the oldest beyond [`MAX_KEPT`].
pub(super) fn save(dir: &Path, bytes: &[u8], extension: &str) -> anyhow::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let unix_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or_default();
    let tag = uuid::Uuid::new_v4().simple().to_string();
    let name = file_name(unix_ms, &tag[..8], extension);
    let path = dir.join(&name);
    // Written beside it under a name no reader takes for a picture, then
    // renamed, so a reader never sees half an image.
    let partial = dir.join(format!(".{name}.part"));
    std::fs::write(&partial, bytes)?;
    std::fs::rename(&partial, &path)?;
    prune(dir, MAX_KEPT);
    Ok(path)
}

/// Drops the oldest pictures (by name) so that at most `keep` remain.
pub(super) fn prune(dir: &Path, keep: usize) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut pictures: Vec<PathBuf> = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.is_file()
                && path
                    .extension()
                    .and_then(|ext| ext.to_str())
                    .is_some_and(|ext| matches!(ext, "png" | "jpeg"))
                && !path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with('.'))
        })
        .collect();
    if pictures.len() <= keep {
        return;
    }
    pictures.sort();
    for old in &pictures[..pictures.len() - keep] {
        let _ = std::fs::remove_file(old);
    }
}

#[cfg(test)]
#[path = "browser_capture_tests.rs"]
mod tests;
