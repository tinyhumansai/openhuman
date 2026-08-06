//! Persist generated media to the agent's action directory.
//!
//! GMI returns expiring signed URLs; we download the bytes and write them under
//! a `generated-media/` root inside `action_dir` so final answers can reference
//! a stable local file path (per the `image_generation` contract). The action
//! directory is the agent's canonical read/write root.

use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::types::MediaItem;

/// Subdirectory (under `action_dir`) where generated artifacts are stored.
const GENERATED_MEDIA_DIR: &str = "generated-media";

/// A downloaded artifact and where it landed on disk.
#[derive(Debug, Clone)]
pub struct PersistedArtifact {
    pub kind: String,
    pub path: PathBuf,
    pub source_url: String,
    pub thumbnail_url: Option<String>,
}

/// Pick a file extension from the artifact kind + content type / URL.
fn extension_for(kind: &str, content_type: Option<&str>, url: &str) -> String {
    if let Some(ct) = content_type {
        let ct = ct.to_ascii_lowercase();
        if ct.contains("png") {
            return "png".to_string();
        }
        if ct.contains("webp") {
            return "webp".to_string();
        }
        if ct.contains("jpeg") || ct.contains("jpg") {
            return "jpg".to_string();
        }
        if ct.contains("mp4") {
            return "mp4".to_string();
        }
        if ct.contains("webm") {
            return "webm".to_string();
        }
    }
    // Fall back to the URL path suffix, then a per-kind default.
    let lower = url.split('?').next().unwrap_or(url).to_ascii_lowercase();
    for ext in ["png", "webp", "jpg", "jpeg", "mp4", "webm"] {
        if lower.ends_with(&format!(".{ext}")) {
            return if ext == "jpeg" {
                "jpg".to_string()
            } else {
                ext.to_string()
            };
        }
    }
    if kind.eq_ignore_ascii_case("video") {
        "mp4".to_string()
    } else {
        "png".to_string()
    }
}

/// Download a single media URL into `dir`, returning the written path.
async fn download_one(
    http: &reqwest::Client,
    dir: &Path,
    item: &MediaItem,
    request_id: &str,
    index: usize,
) -> Result<PersistedArtifact> {
    tracing::info!(
        "[media_generation] downloading {} artifact {} for request={}",
        item.kind,
        index,
        request_id
    );
    let resp = http
        .get(&item.url)
        .send()
        .await
        .with_context(|| format!("failed to fetch generated media from {}", item.url))?
        .error_for_status()
        .with_context(|| format!("generated media URL returned an error: {}", item.url))?;

    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let ext = extension_for(&item.kind, content_type.as_deref(), &item.url);

    let bytes = resp
        .bytes()
        .await
        .with_context(|| format!("failed to read generated media body from {}", item.url))?;

    // Sanitize the request id for use in a filename (it is a UUID from GMI, but
    // be defensive against path separators).
    let safe_id: String = request_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    let filename = format!("{safe_id}-{index}.{ext}");
    let path = dir.join(&filename);
    tokio::fs::write(&path, &bytes)
        .await
        .with_context(|| format!("failed to write generated media to {}", path.display()))?;

    Ok(PersistedArtifact {
        kind: item.kind.clone(),
        path,
        source_url: item.url.clone(),
        thumbnail_url: item.thumbnail_url.clone(),
    })
}

/// Download + persist all media items for a request under
/// `{action_dir}/generated-media/`. Returns the written artifacts.
pub async fn persist_media(
    action_dir: &Path,
    request_id: &str,
    items: &[MediaItem],
) -> Result<Vec<PersistedArtifact>> {
    if items.is_empty() {
        return Ok(Vec::new());
    }
    let dir = action_dir.join(GENERATED_MEDIA_DIR);
    tokio::fs::create_dir_all(&dir)
        .await
        .with_context(|| format!("failed to create generated-media dir {}", dir.display()))?;

    let http = reqwest::Client::new();
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.iter().enumerate() {
        out.push(download_one(&http, &dir, item, request_id, i).await?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_prefers_content_type() {
        assert_eq!(
            extension_for("image", Some("image/png"), "https://x/y"),
            "png"
        );
        assert_eq!(
            extension_for("image", Some("image/webp"), "https://x/y"),
            "webp"
        );
        assert_eq!(
            extension_for("video", Some("video/mp4"), "https://x/y"),
            "mp4"
        );
    }

    #[test]
    fn extension_falls_back_to_url_then_kind() {
        assert_eq!(
            extension_for("image", None, "https://x/y/a.webp?sig=1"),
            "webp"
        );
        assert_eq!(extension_for("video", None, "https://x/y/clip"), "mp4");
        assert_eq!(extension_for("image", None, "https://x/y/clip"), "png");
    }
}
