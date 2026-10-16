//! Sharing artifacts through managed cloud file storage.
//!
//! [`StorageArtifactPublisher`] is OpenHuman's implementation of the
//! `tiny-artifacts` [`ArtifactPublisher`] port: it uploads one version of an
//! artifact through the same backend routes the `storage_*` tools use and
//! returns its link — a private presigned link by default, the stored file's
//! public URL when asked. The crate records the link on that version.
//!
//! What is uploaded is exactly the version asked for: a text version's own
//! content (so any version can be shared), or a file version's file. The
//! same guards as the storage tools apply: nothing is shared under read-only
//! autonomy, and the publisher only exists when the integration client does
//! (a backend is configured and the user is signed in).

use std::path::Path;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tiny_artifacts::{
    ArtifactKind, ArtifactPublisher, ArtifactRecord, ArtifactVersion, PublishOptions, PublishedLink,
};

use super::tools::helpers::{file_path, mime_for_path, FILES_PATH};
use super::types::{LinkResponse, UploadResponse};
use crate::integrations::IntegrationClient;
use openhuman_embed::__host::config::Config;
use openhuman_embed::__host::security::approval::ApprovalTurnContext;
use openhuman_embed::__host::security::SecurityPolicy;

/// Lifetime of a private link when the caller does not choose: the backend's
/// maximum, 7 days.
const DEFAULT_LINK_SECONDS: u64 = 604_800;

/// Shares artifact versions through the backend's file storage.
pub struct StorageArtifactPublisher {
    client: Arc<IntegrationClient>,
    security: Arc<SecurityPolicy>,
    /// The turn the backend requests are reported to. A publisher is built
    /// once per tool set, not per call, so it reports outside any chat.
    approval: ApprovalTurnContext,
}

impl StorageArtifactPublisher {
    /// A publisher over `client`, refusing to share when `security` is
    /// read-only.
    pub fn new(client: Arc<IntegrationClient>, security: Arc<SecurityPolicy>) -> Self {
        Self {
            client,
            security,
            approval: ApprovalTurnContext::default(),
        }
    }
}

/// The publisher for `config`, or `None` when there is no integration client
/// (no backend URL configured, or not signed in) — the same condition under
/// which the `storage_*` tools are absent.
pub fn artifact_publisher(config: &Config) -> Option<Arc<dyn ArtifactPublisher>> {
    let client = crate::integrations::build_client(config)?;
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    ));
    Some(Arc::new(StorageArtifactPublisher::new(client, security)))
}

/// The bytes, file name and media type a version uploads as.
async fn upload_body(
    record: &ArtifactRecord,
    version: &ArtifactVersion,
) -> anyhow::Result<(Vec<u8>, String, &'static str)> {
    if record.holds_file() {
        let path = version
            .file
            .as_ref()
            .map_or(version.body.as_str(), |f| f.path.as_str());
        let path = Path::new(path);
        // The record was checked when written; check again on what the path
        // resolves to now, so nothing protected leaves the machine.
        let resolved = tokio::fs::canonicalize(path)
            .await
            .map_err(|e| anyhow::anyhow!("cannot read the artifact's file: {e}"))?;
        if let Some(reason) = openhuman_embed::__host::artifact_publisher::refused_path(&resolved) {
            anyhow::bail!("[policy-blocked] cannot share this artifact: {reason}");
        }
        let bytes = tokio::fs::read(&resolved)
            .await
            .map_err(|e| anyhow::anyhow!("cannot read the artifact's file: {e}"))?;
        // Named after the artifact: the file read may be a kept version copy
        // (`v2.pptx`), whose name means nothing to whoever opens the link.
        let name = match path.extension().and_then(|e| e.to_str()) {
            Some(ext) => format!("{}.{ext}", file_stem(&record.title)),
            None => file_stem(&record.title),
        };
        let mime = mime_for_path(path);
        return Ok((bytes, name, mime));
    }
    let ext = match record.kind {
        ArtifactKind::Markdown => "md",
        ArtifactKind::Html => "html",
        _ => "txt",
    };
    let name = format!("{}.{ext}", file_stem(&record.title));
    let mime = match record.kind {
        ArtifactKind::Markdown => "text/markdown",
        ArtifactKind::Html => "text/html",
        _ => "text/plain",
    };
    Ok((version.body.clone().into_bytes(), name, mime))
}

/// A file-name stem from a title: path separators and control characters
/// dropped, never empty.
fn file_stem(title: &str) -> String {
    let stem: String = title
        .chars()
        .filter(|c| !matches!(c, '/' | '\\' | '\0') && !c.is_control())
        .collect::<String>()
        .trim()
        .trim_matches('.')
        .to_string();
    if stem.is_empty() {
        "artifact".to_string()
    } else {
        stem
    }
}

fn millis(rfc3339: Option<&str>) -> Option<u64> {
    let at = chrono::DateTime::parse_from_rfc3339(rfc3339?).ok()?;
    u64::try_from(at.timestamp_millis()).ok()
}

/// Seconds a private link should last: what was asked (7 days by default,
/// within the backend's 60 s – 7 day range), never past the uploaded file's
/// own expiry — a link may not outlive its file — less a minute of margin.
pub(crate) fn link_seconds(
    requested: Option<u64>,
    file_expires_at_millis: Option<u64>,
    now_millis: u64,
) -> u64 {
    let wanted = requested
        .unwrap_or(DEFAULT_LINK_SECONDS)
        .clamp(60, DEFAULT_LINK_SECONDS);
    match file_expires_at_millis {
        Some(expires) => {
            let left = expires.saturating_sub(now_millis) / 1000;
            wanted.min(left.saturating_sub(60)).max(60)
        }
        None => wanted,
    }
}

/// Whether `err` is the backend refusing the request's values (a 400 or 422),
/// as opposed to a transport, authentication or server failure. The
/// integration client reports a status as "Backend returned <code> <reason>".
fn is_validation_rejection(err: &anyhow::Error) -> bool {
    let message = err.to_string();
    ["Backend returned 400 ", "Backend returned 422 "]
        .iter()
        .any(|prefix| message.contains(prefix))
}

impl StorageArtifactPublisher {
    /// A presigned link for `file_id` lasting `seconds`. If the backend
    /// rejects that lifetime (a 400/422), one retry asks for its default
    /// lifetime instead; any other failure is returned as it is.
    async fn private_link(&self, file_id: &str, seconds: u64) -> anyhow::Result<LinkResponse> {
        let route = file_path(file_id, "/link");
        let first = match self
            .client
            .post::<LinkResponse>(
                &self.approval,
                &route,
                &json!({ "expiresInSeconds": seconds }),
            )
            .await
        {
            Ok(link) => return Ok(link),
            Err(err) if is_validation_rejection(&err) => err,
            Err(err) => return Err(anyhow::anyhow!("link generation failed: {err}")),
        };
        tracing::warn!(
            file_id,
            seconds,
            err = %first,
            "[file_storage] link with a set lifetime refused; retrying with the default"
        );
        self.client
            .post::<LinkResponse>(&self.approval, &route, &json!({}))
            .await
            .map_err(|e| anyhow::anyhow!("link generation failed: {e}"))
    }

    /// Delete an upload nothing points at. Best-effort.
    async fn discard(&self, file_id: &str) {
        if let Err(err) = self
            .client
            .delete::<super::types::DeleteResponse>(&self.approval, &file_path(file_id, ""))
            .await
        {
            tracing::warn!(file_id, err = %err, "[file_storage] could not delete an unshared upload");
        }
    }
}

#[async_trait]
impl ArtifactPublisher for StorageArtifactPublisher {
    async fn publish(
        &self,
        record: &ArtifactRecord,
        version: &ArtifactVersion,
        options: &PublishOptions,
    ) -> anyhow::Result<PublishedLink> {
        if !self.security.can_act() {
            anyhow::bail!("[policy-blocked] Action blocked: autonomy is read-only");
        }
        let (bytes, filename, mime) = upload_body(record, version).await?;
        tracing::debug!(
            artifact_id = %record.id,
            version = version.version,
            bytes = bytes.len(),
            public = options.public,
            "[file_storage] sharing artifact version"
        );
        let part = reqwest::multipart::Part::bytes(bytes)
            .file_name(filename)
            .mime_str(mime)?;
        let visibility = if options.public { "public" } else { "private" };
        let form = reqwest::multipart::Form::new()
            .part("file", part)
            .text("visibility", visibility);
        let uploaded = self
            .client
            .upload_multipart::<UploadResponse>(&self.approval, FILES_PATH, form)
            .await
            .map_err(|e| anyhow::anyhow!("upload failed: {e}"))?;

        let link = if options.public {
            uploaded
                .public_url
                .clone()
                .map(|url| (url, uploaded.expires_at.clone()))
                .ok_or_else(|| anyhow::anyhow!("the stored file has no public URL"))
        } else {
            let seconds = link_seconds(
                options.expires_in_seconds,
                millis(uploaded.expires_at.as_deref()),
                tiny_artifacts::now_millis(),
            );
            self.private_link(&uploaded.file_id, seconds)
                .await
                .map(|link| (link.url, link.expires_at))
        };
        let (url, expires_at) = match link {
            Ok(link) => link,
            Err(err) => {
                // Nothing will point at the upload: free it rather than leave
                // a billed orphan.
                self.discard(&uploaded.file_id).await;
                return Err(err);
            }
        };
        tracing::info!(
            artifact_id = %record.id,
            version = version.version,
            file_id = %uploaded.file_id,
            public = options.public,
            "[file_storage] shared artifact version"
        );
        Ok(PublishedLink {
            url,
            expires_at_millis: millis(expires_at.as_deref()),
            published_at_millis: tiny_artifacts::now_millis(),
            public: options.public,
        })
    }
}

#[cfg(test)]
#[path = "artifact_publisher_tests.rs"]
mod tests;
