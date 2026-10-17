//! What the preview panel shows for an artifact: the newest version's content.
//!
//! Served by the core (`ai.read_artifact_body`), so the panel reads from
//! whichever core the app talks to, local or remote, and document conversion
//! runs where the artifact lives.
//!
//! * Text kinds (`text`, `markdown`, `html`) return their stored content.
//! * A file or image version is opened only when its path is inside the
//!   folders file artifacts may point into (the agent's working folder and the
//!   files folders), and only up to [`PREVIEW_MAX_BYTES`]. A UTF-8 file comes
//!   back as text; a DOCX, PPTX, XLSX or PDF is converted to Markdown through
//!   the TinyDocs module. Anything else comes back as its path, for the panel
//!   to show as a file.

use std::path::Path;

use serde::Serialize;
use tiny_artifacts::{ArtifactKind, ArtifactRecord};

use crate::config::Config;

/// Largest file the preview opens. A bigger one shows as a file.
pub const PREVIEW_MAX_BYTES: u64 = 10 * 1024 * 1024;

/// The newest version of an artifact, as the preview panel shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ArtifactBody {
    /// The record's kind: `text`, `markdown`, `html`, `image` or `file`.
    pub kind: String,
    pub title: String,
    /// The newest version number (0 for an artifact still being produced).
    pub version: u32,
    /// The content to show: the text, the converted Markdown, or the file's
    /// path when it is not shown as text.
    pub body: String,
    /// Whether `body` is content rather than a path.
    pub is_text: bool,
    /// Whether `body` is Markdown converted from a document, to render as
    /// Markdown. The artifact itself is still a file: it is not edited.
    pub converted: bool,
}

/// The preview of artifact `id` in `config`'s workspace.
///
/// # Errors
///
/// Returns an error when the artifact does not exist or its record cannot be
/// read.
pub async fn read_body(config: &Config, id: &str) -> Result<ArtifactBody, String> {
    let files = super::files::FileRoots::from_config(config);
    let mut roots =
        super::tiny_store::file_roots(&config.workspace_dir, &files, Some(&config.action_dir));
    let versioned = super::tiny_store::artifacts_service(
        &config.workspace_dir,
        files.clone(),
        Some(&config.action_dir),
        None,
    )
    .get(id)
    .await
    .map_err(|e| e.to_string())?;
    let record = match versioned {
        Some(record) => record,
        // An artifact made before versioning has only its legacy record: show
        // the file that record vouches for.
        None => {
            let record = legacy_record(&config.workspace_dir, &files, id).await?;
            if let Some(folder) = record
                .latest()
                .and_then(|v| v.file.as_ref())
                .and_then(|f| Path::new(&f.path).parent())
            {
                roots.push(folder.to_path_buf());
            }
            record
        }
    };
    let converter = DocumentConverter::for_config(config);
    Ok(body_of(
        &record,
        &super::tiny_store::guarded_roots(roots),
        &converter,
    )
    .await)
}

/// A one-version file record for an artifact that has only its legacy
/// [`ArtifactMeta`](super::types::ArtifactMeta), pointing at the file the
/// legacy store vouches for (ready, inside its folders, still on disk).
async fn legacy_record(
    workspace_dir: &Path,
    files: &super::files::FileRoots,
    id: &str,
) -> Result<ArtifactRecord, String> {
    let meta = super::store::get_artifact(workspace_dir, id)
        .await
        .map_err(|_| format!("artifact {id} not found"))?;
    let path = super::files::resolve_ready_file(workspace_dir, files, id).await?;
    let path = path.display().to_string();
    let at = u64::try_from(meta.created_at.timestamp_millis()).unwrap_or(0);
    let mut version = tiny_artifacts::ArtifactVersion::new(
        1,
        path.clone(),
        tiny_artifacts::ArtifactAuthor::Agent,
        super::tiny_store::AGENT_AUTHOR_ID,
        at,
    );
    version.file = Some(tiny_artifacts::FileRef {
        size_bytes: meta.size_bytes,
        media_type: None,
        path,
    });
    let mut record = ArtifactRecord::pending(id, meta.title, ArtifactKind::File, at);
    record.versions.push(version);
    Ok(record)
}

/// Converts a document's bytes to Markdown. `None` when conversion is not
/// available (the documents component is off, or not in this build).
pub(crate) struct DocumentConverter<'a> {
    #[cfg_attr(not(feature = "documents"), allow(dead_code))]
    config: Option<&'a Config>,
}

impl<'a> DocumentConverter<'a> {
    fn for_config(config: &'a Config) -> Self {
        Self {
            config: config.components.documents.then_some(config),
        }
    }

    /// A converter that converts nothing.
    #[cfg(test)]
    pub(crate) fn none() -> Self {
        Self { config: None }
    }

    #[cfg(feature = "documents")]
    async fn convert(&self, extension: &str, bytes: &[u8]) -> Option<String> {
        use tinydocs_bus::DocumentFormat;
        let config = self.config?;
        let format = match extension {
            "docx" => DocumentFormat::Docx,
            "pptx" => DocumentFormat::Pptx,
            "xlsx" => DocumentFormat::Xlsx,
            "pdf" => DocumentFormat::Pdf,
            _ => return None,
        };
        match crate::modules::documents::convert_markdown(config, format, bytes).await {
            Ok(markdown) => Some(markdown),
            Err(err) => {
                tracing::debug!(
                    target: "artifacts",
                    extension,
                    err = ?err,
                    "[artifacts] preview conversion failed; showing the file"
                );
                None
            }
        }
    }

    #[cfg(not(feature = "documents"))]
    async fn convert(&self, _extension: &str, _bytes: &[u8]) -> Option<String> {
        None
    }
}

/// The preview of `record`'s newest version, opening files only inside
/// `roots`.
pub(crate) async fn body_of(
    record: &ArtifactRecord,
    roots: &tiny_artifacts::FileRoots,
    converter: &DocumentConverter<'_>,
) -> ArtifactBody {
    let latest = record.latest();
    let stored = latest.map(|v| v.body.clone()).unwrap_or_default();
    let mut preview = ArtifactBody {
        kind: kind_name(record.kind).to_string(),
        title: record.title.clone(),
        version: latest.map_or(0, |v| v.version),
        body: stored.clone(),
        is_text: !record.holds_file(),
        converted: false,
    };
    if !record.holds_file() {
        return preview;
    }
    let path = latest
        .and_then(|v| v.file.as_ref())
        .map_or(stored, |f| f.path.clone());
    preview.body = path.clone();
    // Inside the trusted folders, an existing regular file: anything else is
    // shown by its path only.
    let Ok(file) = roots.resolve(&path).await else {
        return preview;
    };
    if file.size_bytes > PREVIEW_MAX_BYTES {
        return preview;
    }
    let Ok(bytes) = tokio::fs::read(Path::new(&file.path)).await else {
        return preview;
    };
    let probe = &bytes[..bytes.len().min(8192)];
    if !probe.contains(&0) && std::str::from_utf8(probe).is_ok() {
        if let Ok(text) = String::from_utf8(bytes) {
            preview.body = text;
            preview.is_text = true;
        }
        return preview;
    }
    let Some(extension) = file.extension() else {
        return preview;
    };
    if let Some(markdown) = converter
        .convert(&extension, &bytes)
        .await
        .filter(|markdown| !markdown.trim().is_empty())
    {
        preview.body = markdown;
        preview.is_text = true;
        preview.converted = true;
    }
    preview
}

fn kind_name(kind: ArtifactKind) -> &'static str {
    match kind {
        ArtifactKind::Text => "text",
        ArtifactKind::Markdown => "markdown",
        ArtifactKind::Html => "html",
        ArtifactKind::Image => "image",
        ArtifactKind::File => "file",
    }
}

#[cfg(test)]
#[path = "preview_tests.rs"]
mod tests;
