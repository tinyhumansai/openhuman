use crate::openhuman::config::{
    build_runtime_proxy_client_with_timeouts, MultimodalConfig, MultimodalFileConfig,
};
use crate::openhuman::inference::provider::ChatMessage;
use base64::{engine::general_purpose::STANDARD, Engine as _};
use flate2::read::GzDecoder;
use reqwest::Client;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

const IMAGE_MARKER_PREFIX: &str = "[IMAGE:";
const ALLOWED_IMAGE_MIME_TYPES: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/webp",
    "image/gif",
    "image/bmp",
];

/// File-attachment marker prefix. Counterpart to [`IMAGE_MARKER_PREFIX`].
/// Resolution rules mirror images: local paths, optional http(s) URLs
/// gated by [`MultimodalFileConfig::allow_remote_fetch`], and renderer-owned
/// `data:` URIs. Inline `application/gzip` data URIs are decompressed before
/// validation when they carry an `original_mime=...` parameter.
const FILE_MARKER_PREFIX: &str = "[FILE:";

/// Hard upper bound on how long [`pdf_extract::extract_text_from_mem`]
/// may run before the worker is abandoned and the file degrades to a
/// metadata-only reference. PDFs known to choke the parser (extremely
/// large, encrypted, malformed) must not stall a chat turn.
const PDF_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// Worst-case length budget reserved for the rendered truncation
/// suffix. The actual emitted suffix is `"\n[…truncated {N} chars]"`
/// where `N` is the dynamic dropped-character count. The reservation
/// uses the longest plausible value (`max_extracted_text_chars` is
/// clamped to 200_000, so `N` has up to 6 digits) so the truncated
/// payload never overshoots `max_extracted_text_chars` even after the
/// suffix is appended.
const TEXT_TRUNCATION_SUFFIX_BUDGET: &str = "\n[…truncated 999999 chars]";

#[derive(Debug, Clone)]
pub struct PreparedMessages {
    pub messages: Vec<ChatMessage>,
    pub contains_images: bool,
    pub contains_files: bool,
}

/// Resolved representation of a `[FILE:…]` marker. Extractable formats
/// inline their text payload; binary-only formats surface as metadata
/// only so the agent can mention them without seeing raw bytes.
#[derive(Debug, Clone)]
pub enum FilePayload {
    Extracted {
        name: String,
        mime: String,
        size_bytes: usize,
        text: String,
        truncated_chars: usize,
    },
    Reference {
        name: String,
        mime: String,
        size_bytes: usize,
        sha256_prefix: String,
    },
}

#[derive(Debug, thiserror::Error)]
pub enum MultimodalError {
    #[error("multimodal image limit exceeded: max_images={max_images}, found={found}")]
    TooManyImages { max_images: usize, found: usize },

    #[error("multimodal image size limit exceeded for '{input}': {size_bytes} bytes > {max_bytes} bytes")]
    ImageTooLarge {
        input: String,
        size_bytes: usize,
        max_bytes: usize,
    },

    #[error("multimodal image MIME type is not allowed for '{input}': {mime}")]
    UnsupportedMime { input: String, mime: String },

    #[error("multimodal remote image fetch is disabled for '{input}'")]
    RemoteFetchDisabled { input: String },

    #[error("multimodal image source not found or unreadable: '{input}'")]
    ImageSourceNotFound { input: String },

    #[error("invalid multimodal image marker '{input}': {reason}")]
    InvalidMarker { input: String, reason: String },

    #[error("failed to download remote image '{input}': {reason}")]
    RemoteFetchFailed { input: String, reason: String },

    #[error("failed to read local image '{input}': {reason}")]
    LocalReadFailed { input: String, reason: String },

    #[error("multimodal file limit exceeded: max_files={max_files}, found={found}")]
    TooManyFiles { max_files: usize, found: usize },

    #[error(
        "multimodal file size limit exceeded for '{input}': {size_bytes} bytes > {max_bytes} bytes"
    )]
    FileTooLarge {
        input: String,
        size_bytes: usize,
        max_bytes: usize,
    },

    #[error(
        "multimodal file MIME type '{mime}' for '{input}' is not allowed; supported: {supported}"
    )]
    UnsupportedFileMime {
        input: String,
        mime: String,
        supported: String,
    },

    #[error("multimodal file source not found or unreadable: '{input}'")]
    FileSourceNotFound { input: String },

    #[error("multimodal remote file fetch is disabled for '{input}'")]
    RemoteFileFetchDisabled { input: String },

    #[error("failed to download remote file '{input}': {reason}")]
    RemoteFileFetchFailed { input: String, reason: String },

    #[error("failed to read local file '{input}': {reason}")]
    LocalFileReadFailed { input: String, reason: String },

    #[error("invalid multimodal file marker '{input}': {reason}")]
    InvalidFileMarker { input: String, reason: String },
}

pub fn parse_image_markers(content: &str) -> (String, Vec<String>) {
    let mut refs = Vec::new();
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = content[cursor..].find(IMAGE_MARKER_PREFIX) {
        let start = cursor + rel_start;
        cleaned.push_str(&content[cursor..start]);

        let marker_start = start + IMAGE_MARKER_PREFIX.len();
        let Some(rel_end) = content[marker_start..].find(']') else {
            cleaned.push_str(&content[start..]);
            cursor = content.len();
            break;
        };

        let end = marker_start + rel_end;
        let candidate = content[marker_start..end].trim();

        if candidate.is_empty() {
            cleaned.push_str(&content[start..=end]);
        } else {
            refs.push(candidate.to_string());
        }

        cursor = end + 1;
    }

    if cursor < content.len() {
        cleaned.push_str(&content[cursor..]);
    }

    (cleaned.trim().to_string(), refs)
}

/// Count `[IMAGE:…]` markers in the **latest** user message only.
///
/// Earlier versions summed markers across every user-role message in
/// the history, which made the per-turn `max_images` cap drift upward
/// over a long conversation: a thread that attached three images on
/// turn 1 already counted them again on turn 2 even when the new user
/// message had no attachments at all. Looking only at the most recent
/// user message matches the user's intent ("how many am I attaching
/// THIS turn") and keeps the cap stable.
pub fn count_image_markers(messages: &[ChatMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| parse_image_markers(&m.content).1.len())
        .unwrap_or(0)
}

pub fn contains_image_markers(messages: &[ChatMessage]) -> bool {
    count_image_markers(messages) > 0
}

fn latest_user_message(messages: &[ChatMessage]) -> Option<&ChatMessage> {
    messages.iter().rev().find(|m| m.role == "user")
}

pub fn extract_ollama_image_payload(image_ref: &str) -> Option<String> {
    if image_ref.starts_with("data:") {
        let comma_idx = image_ref.find(',')?;
        let (_, payload) = image_ref.split_at(comma_idx + 1);
        let payload = payload.trim();
        if payload.is_empty() {
            None
        } else {
            Some(payload.to_string())
        }
    } else {
        Some(image_ref.trim().to_string()).filter(|value| !value.is_empty())
    }
}

/// Strip every `[FILE:…]` marker from `content` and return the cleaned
/// text alongside the raw source references in order. Mirrors
/// [`parse_image_markers`] so the two pipelines stay symmetrical.
pub fn parse_file_markers(content: &str) -> (String, Vec<String>) {
    let mut refs = Vec::new();
    let mut cleaned = String::with_capacity(content.len());
    let mut cursor = 0usize;

    while let Some(rel_start) = content[cursor..].find(FILE_MARKER_PREFIX) {
        let start = cursor + rel_start;
        cleaned.push_str(&content[cursor..start]);

        let marker_start = start + FILE_MARKER_PREFIX.len();
        let Some(rel_end) = content[marker_start..].find(']') else {
            cleaned.push_str(&content[start..]);
            cursor = content.len();
            break;
        };

        let end = marker_start + rel_end;
        let candidate = content[marker_start..end].trim();

        if candidate.is_empty() {
            cleaned.push_str(&content[start..=end]);
        } else {
            refs.push(candidate.to_string());
        }

        cursor = end + 1;
    }

    if cursor < content.len() {
        cleaned.push_str(&content[cursor..]);
    }

    (cleaned.trim().to_string(), refs)
}

/// Count `[FILE:…]` markers in the **latest** user message only — same
/// per-turn semantics as [`count_image_markers`]. See that function's
/// rustdoc for the reasoning.
pub fn count_file_markers(messages: &[ChatMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| parse_file_markers(&m.content).1.len())
        .unwrap_or(0)
}

pub fn contains_file_markers(messages: &[ChatMessage]) -> bool {
    count_file_markers(messages) > 0
}

pub async fn prepare_messages_for_provider(
    messages: &[ChatMessage],
    image_config: &MultimodalConfig,
    file_config: &MultimodalFileConfig,
) -> anyhow::Result<PreparedMessages> {
    let (max_images, max_image_size_mb) = image_config.effective_limits();
    let max_image_bytes = max_image_size_mb.saturating_mul(1024 * 1024);

    let (max_files, max_file_size_mb, max_extracted_text_chars) = file_config.effective_limits();
    let max_file_bytes = max_file_size_mb.saturating_mul(1024 * 1024);

    let found_images = count_image_markers(messages);
    if found_images > max_images {
        return Err(MultimodalError::TooManyImages {
            max_images,
            found: found_images,
        }
        .into());
    }

    let found_files = count_file_markers(messages);
    // Hard-zero gate: `MultimodalFileConfig::for_untrusted_channel_input()`
    // (and the triage arm) sets `max_files: 0` as a sentinel meaning
    // "reject every `[FILE:…]` marker before any disk read." The clamp
    // inside `effective_limits` lifts 0 → 1, so without this pre-check a
    // single attacker-supplied `[FILE:/etc/passwd]` would slip through
    // (`1 > 1` is false). Honour the raw value here so the channel /
    // triage hardening is actually enforced.
    if file_config.max_files == 0 && found_files > 0 {
        return Err(MultimodalError::TooManyFiles {
            max_files: 0,
            found: found_files,
        }
        .into());
    }
    if found_files > max_files {
        return Err(MultimodalError::TooManyFiles {
            max_files,
            found: found_files,
        }
        .into());
    }

    tracing::debug!(
        target: "multimodal",
        found_images,
        found_files,
        "[multimodal] preparing messages"
    );

    if found_images == 0 && found_files == 0 {
        return Ok(PreparedMessages {
            messages: messages.to_vec(),
            contains_images: false,
            contains_files: false,
        });
    }

    let remote_client = build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10);

    let mut normalized_messages = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role != "user" {
            normalized_messages.push(message.clone());
            continue;
        }

        let (text_after_images, image_refs) = parse_image_markers(&message.content);
        let (cleaned_text, file_refs) = parse_file_markers(&text_after_images);

        if image_refs.is_empty() && file_refs.is_empty() {
            normalized_messages.push(message.clone());
            continue;
        }

        let mut normalized_image_refs = Vec::with_capacity(image_refs.len());
        for reference in image_refs {
            let data_uri = normalize_image_reference(
                &reference,
                image_config,
                max_image_bytes,
                &remote_client,
            )
            .await?;
            normalized_image_refs.push(data_uri);
        }

        let mut file_payloads = Vec::with_capacity(file_refs.len());
        for reference in file_refs {
            let payload = normalize_file_reference(
                &reference,
                file_config,
                max_file_bytes,
                max_extracted_text_chars,
                &remote_client,
            )
            .await?;
            file_payloads.push(payload);
        }

        let content =
            compose_multimodal_message(&cleaned_text, &normalized_image_refs, &file_payloads);
        normalized_messages.push(ChatMessage {
            id: message.id.clone(),
            role: message.role.clone(),
            content,
            extra_metadata: message.extra_metadata.clone(),
        });
    }

    Ok(PreparedMessages {
        messages: normalized_messages,
        contains_images: found_images > 0,
        contains_files: found_files > 0,
    })
}

/// Ingress-time file extraction (PDF-attach fix).
///
/// Replaces every `[FILE:data:…]` marker in a raw user message with its
/// extracted-text block (`[FILE-EXTRACTED: name="…"]…text…[/FILE-EXTRACTED]`),
/// or a content-less `[FILE-ATTACHED: …]` placeholder when extraction fails.
/// `[IMAGE:…]` markers are deliberately left untouched here — they are handled
/// at provider dispatch (vision needs the inline data URI).
///
/// Run this at channel ingress, BEFORE the message is persisted to history,
/// auto-saved to the memory store, appended to the cross-thread JSONL, or
/// scanned for prompt injection — so the multi-MB base64 data URI never
/// survives past the front door. Idempotent: a message with no `[FILE:` marker
/// (already-rewritten `[FILE-EXTRACTED]` text included) is returned unchanged.
pub async fn inline_file_attachments(message: &str, file_config: &MultimodalFileConfig) -> String {
    if !message.contains(FILE_MARKER_PREFIX) {
        return message.to_string();
    }
    let (cleaned, file_refs) = parse_file_markers(message);
    if file_refs.is_empty() {
        return message.to_string();
    }
    let (max_files, max_file_size_mb, max_extracted_text_chars) = file_config.effective_limits();
    let max_file_bytes = max_file_size_mb.saturating_mul(1024 * 1024);
    // Enforce the per-turn file cap at ingress (rewriting the markers removes
    // the count check that `prepare_messages_for_provider` would otherwise do).
    // `max_files == 0` is the hard-disable sentinel: read nothing. Over-cap refs
    // degrade to a content-less placeholder rather than being normalized/read.
    let read_cap = if file_config.max_files == 0 {
        0
    } else {
        max_files
    };
    let client = build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10);

    let mut payloads = Vec::with_capacity(file_refs.len());
    for (idx, reference) in file_refs.iter().enumerate() {
        if idx >= read_cap {
            payloads.push(FilePayload::Reference {
                name: "attachment (over file limit)".to_string(),
                mime: "application/octet-stream".to_string(),
                size_bytes: 0,
                sha256_prefix: "skipped".to_string(),
            });
            continue;
        }
        match normalize_file_reference(
            reference,
            file_config,
            max_file_bytes,
            max_extracted_text_chars,
            &client,
        )
        .await
        {
            Ok(payload) => payloads.push(payload),
            Err(err) => {
                tracing::warn!(
                    target: "multimodal",
                    reason = %err,
                    "[multimodal::files][ingress] file marker could not be normalized; emitting bare placeholder"
                );
                payloads.push(FilePayload::Reference {
                    name: "attachment".to_string(),
                    mime: "application/octet-stream".to_string(),
                    size_bytes: 0,
                    sha256_prefix: "unavailable".to_string(),
                });
            }
        }
    }

    let rewritten = compose_multimodal_message(&cleaned, &[], &payloads);
    tracing::info!(
        target: "multimodal",
        files = payloads.len(),
        before_chars = message.chars().count(),
        after_chars = rewritten.chars().count(),
        "[multimodal::files][ingress] inlined file attachments — data URI replaced with extracted text/placeholder before persistence"
    );
    rewritten
}

// ── Image sidecar (disk-backed) ───────────────────────────────────────────
//
// Persisted messages must never carry a raw `[IMAGE:data:…]` data URI: like the
// PDF blob it floods the injection scan, the memory auto-save (N-chunk embed →
// Voyage 400), and the cross-thread JSONL index. So at ingress we replace each
// image marker with a compact `[Image: image #att:<id>]` placeholder and write
// the decoded image bytes out-of-band to `<workspace>/attachments/<id>.<ext>`.
// At provider dispatch we rehydrate the placeholder back into an inline
// `[IMAGE:<path>]` marker — but ONLY for vision-capable models; non-vision
// models keep the text placeholder (no multi-MB payload, no error).
//
// Disk-backed (not the old in-memory FIFO) so attachments survive process
// restarts and long delegation chains: a sub-agent spawned several hops after
// ingress still resolves the image by id. `normalize_local_image` re-reads the
// file at dispatch, so no decode/MIME logic is duplicated here.

/// Placeholder token left in the persisted message in place of a raw image data
/// URI. Mixed-case so it never collides with the inline `[IMAGE:` parser.
const IMAGE_PLACEHOLDER_PREFIX: &str = "[Image:";
/// Separator before the stash content-hash id inside a placeholder.
const IMAGE_STASH_REF: &str = "#att:";

/// Soft cap on the on-disk attachments directory. After each write, oldest
/// files (by mtime) are evicted until the total is back under this bound.
const ATTACHMENTS_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Age after which an attachment is considered stale and removed by the
/// startup sweep ([`sweep_stale_attachments`]).
const ATTACHMENTS_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Process-global on-disk attachments directory. Installed once at core startup
/// via [`init_attachments_dir`]; mirrors the `OnceLock` pattern the in-memory
/// stash used before.
static ATTACHMENTS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Install the on-disk attachments directory (`<workspace>/attachments`). Call
/// once at core startup. Idempotent — first writer wins. Best-effort fires a
/// stale-file sweep when called inside a Tokio runtime.
pub fn init_attachments_dir(dir: PathBuf) {
    if ATTACHMENTS_DIR.set(dir).is_err() {
        return;
    }
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async {
            sweep_stale_attachments().await;
        });
    }
}

/// Resolve the attachments dir, falling back to a **per-user private** dir when
/// unset (CLI / direct invocation / tests that never called
/// [`init_attachments_dir`]). The persistence-pollution fix and rehydration both
/// hold either way.
fn attachments_dir() -> PathBuf {
    ATTACHMENTS_DIR
        .get()
        .cloned()
        .unwrap_or_else(fallback_attachments_dir)
}

/// Per-user fallback attachments dir used only when [`init_attachments_dir`] was
/// never called. Uses the OS user cache dir (e.g. `~/Library/Caches/...`,
/// `~/.cache/...`) so persisted image bytes aren't dropped into a world-readable
/// shared `temp_dir()` on multi-user hosts. Only falls back to `temp_dir()` when
/// no user cache dir can be resolved at all.
fn fallback_attachments_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.cache_dir().join("openhuman-attachments"))
        .unwrap_or_else(|| std::env::temp_dir().join("openhuman-attachments"))
}

/// Persist a canonical image data URI to `<dir>/<id>.<ext>`, content-addressed
/// by `id`. Atomic (temp file + rename); deduped (skips the write when the
/// target already exists). Returns the written path. After writing, enforces
/// [`ATTACHMENTS_MAX_BYTES`].
async fn write_attachment(id: &str, data_uri: &str) -> anyhow::Result<PathBuf> {
    let parsed = parse_data_uri(data_uri)
        .map_err(|reason| anyhow::anyhow!("cannot decode stashed data URI: {reason}"))?;
    let ext = ext_from_mime(&parsed.mime).unwrap_or("img");
    let dir = attachments_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let final_path = dir.join(format!("{id}.{ext}"));
    if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
        return Ok(final_path); // content-addressed: already persisted
    }
    let tmp_path = dir.join(format!(".{id}.{ext}.tmp"));
    tokio::fs::write(&tmp_path, &parsed.bytes).await?;
    tokio::fs::rename(&tmp_path, &final_path).await?;
    enforce_attachments_cap(&dir).await;
    Ok(final_path)
}

/// File extension for a known image MIME (inverse of [`mime_from_extension`]).
fn ext_from_mime(mime: &str) -> Option<&'static str> {
    match mime {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/webp" => Some("webp"),
        "image/gif" => Some("gif"),
        "image/bmp" => Some("bmp"),
        _ => None,
    }
}

/// Build an `id → path` index from a single read of the attachments dir. Skips
/// in-flight `.tmp` files. Sync (called from the sync rehydrate path).
fn build_attachment_index() -> HashMap<String, PathBuf> {
    let dir = attachments_dir();
    let mut map = HashMap::new();
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return map;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with('.') {
            continue; // skip `.<id>.<ext>.tmp` in-flight writes
        }
        if let Some(stem) = name.split('.').next() {
            if !stem.is_empty() {
                map.insert(stem.to_string(), entry.path());
            }
        }
    }
    map
}

/// Evict oldest attachments (by mtime) until the dir is under
/// [`ATTACHMENTS_MAX_BYTES`]. Best-effort; replaces the old heap FIFO.
async fn enforce_attachments_cap(dir: &Path) {
    let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
    let mut total: u64 = 0;
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        // Skip in-flight `.<id>.<ext>.tmp` writes (mirrors build_attachment_index)
        // so concurrent atomic writes aren't evicted out from under a rename.
        if entry.file_name().to_string_lossy().starts_with('.') {
            continue;
        }
        if let Ok(meta) = entry.metadata().await {
            if meta.is_file() {
                let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
                total = total.saturating_add(meta.len());
                files.push((entry.path(), mtime, meta.len()));
            }
        }
    }
    if total <= ATTACHMENTS_MAX_BYTES {
        return;
    }
    files.sort_by_key(|(_, mtime, _)| *mtime); // oldest first
    for (path, _, len) in files {
        if total <= ATTACHMENTS_MAX_BYTES {
            break;
        }
        if tokio::fs::remove_file(&path).await.is_ok() {
            total = total.saturating_sub(len);
            tracing::debug!(
                target: "multimodal",
                path = %path.display(),
                "[multimodal::images][gc] evicted attachment over size cap"
            );
        }
    }
}

/// Delete attachments older than [`ATTACHMENTS_TTL`]. Best-effort startup sweep
/// fired by [`init_attachments_dir`].
pub async fn sweep_stale_attachments() {
    let dir = attachments_dir();
    let Ok(mut rd) = tokio::fs::read_dir(&dir).await else {
        return;
    };
    let now = std::time::SystemTime::now();
    let mut reclaimed = 0u64;
    while let Ok(Some(entry)) = rd.next_entry().await {
        let Ok(meta) = entry.metadata().await else {
            continue;
        };
        if !meta.is_file() {
            continue;
        }
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        let age = now.duration_since(mtime).unwrap_or(Duration::ZERO);
        if age > ATTACHMENTS_TTL && tokio::fs::remove_file(entry.path()).await.is_ok() {
            reclaimed = reclaimed.saturating_add(meta.len());
        }
    }
    if reclaimed > 0 {
        tracing::info!(
            target: "multimodal",
            reclaimed_bytes = reclaimed,
            "[multimodal::images][gc] startup sweep removed stale attachments"
        );
    }
}

/// Ingress-time image stashing. Replaces every `[IMAGE:data:…]` marker with a
/// `[Image: image #att:<id>]` placeholder and stashes the decoded canonical data
/// URI, so the multi-MB base64 never persists. Idempotent (no `[IMAGE:` ⇒ no-op).
pub async fn stash_image_attachments(message: &str, image_config: &MultimodalConfig) -> String {
    if !message.contains(IMAGE_MARKER_PREFIX) {
        return message.to_string();
    }
    let (cleaned, image_refs) = parse_image_markers(message);
    if image_refs.is_empty() {
        return message.to_string();
    }
    let (max_images, max_image_size_mb) = image_config.effective_limits();
    let max_image_bytes = max_image_size_mb.saturating_mul(1024 * 1024);
    let client = build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10);

    let mut placeholders = Vec::with_capacity(image_refs.len());
    for (idx, reference) in image_refs.iter().enumerate() {
        // Enforce the per-turn image cap at ingress: over-cap markers degrade to
        // a text placeholder and are never normalized/read or stashed (rewriting
        // the markers removes the count check `prepare_messages_for_provider`
        // would otherwise apply, and bounds stash growth per message).
        if idx >= max_images {
            placeholders.push("[Image: (over image limit)]".to_string());
            continue;
        }
        match normalize_image_reference(reference, image_config, max_image_bytes, &client).await {
            Ok(data_uri) => {
                let id = sha256_prefix(data_uri.as_bytes());
                match write_attachment(&id, &data_uri).await {
                    Ok(path) => tracing::debug!(
                        target: "multimodal",
                        id = %id,
                        path = %path.display(),
                        "[multimodal::images][stash] persisted attachment to disk"
                    ),
                    Err(err) => tracing::warn!(
                        target: "multimodal",
                        id = %id,
                        reason = %err,
                        "[multimodal::images][stash] failed to persist attachment; placeholder will not rehydrate"
                    ),
                }
                placeholders.push(format!("[Image: image {IMAGE_STASH_REF}{id}]"));
            }
            Err(err) => {
                tracing::warn!(
                    target: "multimodal",
                    reason = %err,
                    "[multimodal::images][ingress] image could not be normalized; emitting bare placeholder"
                );
                placeholders.push("[Image: (could not be processed)]".to_string());
            }
        }
    }

    let mut out = cleaned.trim().to_string();
    for p in &placeholders {
        if !out.is_empty() {
            out.push('\n');
        }
        out.push_str(p);
    }
    tracing::info!(
        target: "multimodal",
        images = placeholders.len(),
        before_chars = message.chars().count(),
        after_chars = out.chars().count(),
        "[multimodal::images][ingress] stashed image attachments — data URI replaced with placeholder before persistence"
    );
    out
}

/// Extract the `[Image: … #att:<id>]` sidecar placeholder tokens from `text`,
/// in order. Used to forward a user's attached images into a delegated vision
/// sub-agent's prompt so its turn rehydrates them (the orchestrator itself, on
/// a non-vision tier, keeps the placeholder as text and never sees the image).
pub fn extract_image_placeholders_in_text(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find(IMAGE_PLACEHOLDER_PREFIX) {
        let start = cursor + rel;
        let Some(rel_end) = text[start..].find(']') else {
            break;
        };
        let end = start + rel_end + 1;
        let token = &text[start..end];
        if token.contains(IMAGE_STASH_REF) {
            out.push(token.to_string());
        }
        cursor = end;
    }
    out
}

/// True if any message carries an `[Image: … #att:<id>]` sidecar placeholder.
pub fn has_image_placeholders(messages: &[ChatMessage]) -> bool {
    messages.iter().any(|m| {
        m.content.contains(IMAGE_PLACEHOLDER_PREFIX) && m.content.contains(IMAGE_STASH_REF)
    })
}

/// Rehydrate `[Image: … #att:<id>]` placeholders back into inline
/// `[IMAGE:<path>]` markers pointing at the on-disk attachment, returning a
/// provider-only copy. `normalize_image_reference` re-reads the file at
/// dispatch. Placeholders whose id is absent (file evicted/swept, or written by
/// a different workspace) keep their text. Call ONLY for vision-capable models.
pub fn rehydrate_image_placeholders(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let index = build_attachment_index();
    messages
        .iter()
        .map(|m| {
            if !(m.content.contains(IMAGE_PLACEHOLDER_PREFIX)
                && m.content.contains(IMAGE_STASH_REF))
            {
                return m.clone();
            }
            ChatMessage {
                id: m.id.clone(),
                role: m.role.clone(),
                content: rehydrate_placeholders_in_text(&m.content, &index),
                extra_metadata: m.extra_metadata.clone(),
            }
        })
        .collect()
}

/// Replace each `[Image: <name> #att:<id>]` placeholder in `text` with
/// `[IMAGE:<path>]` when the id resolves to an on-disk attachment in `index`;
/// leave it verbatim otherwise.
fn rehydrate_placeholders_in_text(text: &str, index: &HashMap<String, PathBuf>) -> String {
    let mut out = String::with_capacity(text.len());
    let mut cursor = 0;
    while let Some(rel) = text[cursor..].find(IMAGE_PLACEHOLDER_PREFIX) {
        let start = cursor + rel;
        out.push_str(&text[cursor..start]);
        let Some(rel_end) = text[start..].find(']') else {
            out.push_str(&text[start..]);
            cursor = text.len();
            break;
        };
        let end = start + rel_end + 1;
        let inner = &text[start..end];
        let replaced = inner.find(IMAGE_STASH_REF).and_then(|ai| {
            let id = inner[ai + IMAGE_STASH_REF.len()..]
                .trim_end_matches(']')
                .trim();
            index
                .get(id)
                .map(|path| format!("[IMAGE:{}]", path.display()))
        });
        out.push_str(replaced.as_deref().unwrap_or(inner));
        cursor = end;
    }
    out.push_str(&text[cursor..]);
    out
}

fn compose_multimodal_message(
    text: &str,
    data_uris: &[String],
    file_payloads: &[FilePayload],
) -> String {
    let mut content = String::new();
    let trimmed = text.trim();

    if !trimmed.is_empty() {
        content.push_str(trimmed);
        content.push_str("\n\n");
    }

    for (index, data_uri) in data_uris.iter().enumerate() {
        if index > 0 {
            content.push('\n');
        }
        content.push_str(IMAGE_MARKER_PREFIX);
        content.push_str(data_uri);
        content.push(']');
    }

    for payload in file_payloads {
        if !content.is_empty() && !content.ends_with('\n') {
            content.push('\n');
        }
        if !content.is_empty() {
            content.push('\n');
        }
        match payload {
            FilePayload::Extracted {
                name,
                mime,
                size_bytes,
                text,
                truncated_chars,
            } => {
                content.push_str(&format!(
                    "[FILE-EXTRACTED: name=\"{}\" size=\"{}\" mime=\"{}\"]\n",
                    escape_attr(name),
                    format_size(*size_bytes),
                    mime
                ));
                content.push_str(text);
                if *truncated_chars > 0 {
                    content.push_str(&format!("\n[…truncated {} chars]", truncated_chars));
                }
                content.push_str("\n[/FILE-EXTRACTED]");
            }
            FilePayload::Reference {
                name,
                mime,
                size_bytes,
                sha256_prefix,
            } => {
                content.push_str(&format!(
                    "[FILE-ATTACHED: name=\"{}\" size=\"{}\" mime=\"{}\" sha256_prefix=\"{}\"]",
                    escape_attr(name),
                    format_size(*size_bytes),
                    mime,
                    sha256_prefix
                ));
            }
        }
    }

    content
}

/// Strip characters that would break the attribute-style serialization
/// of a [`FilePayload`] header (`"` and newlines). Names are user-
/// supplied filenames so they must not be trusted to be quote-free.
fn escape_attr(value: &str) -> String {
    value.replace(['"', '\n', '\r'], "_")
}

fn format_size(size_bytes: usize) -> String {
    const KB: usize = 1024;
    const MB: usize = 1024 * 1024;
    if size_bytes >= MB {
        format!("{:.1} MB", size_bytes as f64 / MB as f64)
    } else if size_bytes >= KB {
        format!("{:.1} KB", size_bytes as f64 / KB as f64)
    } else {
        format!("{} B", size_bytes)
    }
}

async fn normalize_image_reference(
    source: &str,
    config: &MultimodalConfig,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<String> {
    if source.starts_with("data:") {
        return normalize_data_uri(source, max_bytes);
    }

    if source.starts_with("http://") || source.starts_with("https://") {
        if !config.allow_remote_fetch {
            return Err(MultimodalError::RemoteFetchDisabled {
                input: source.to_string(),
            }
            .into());
        }

        return normalize_remote_image(source, max_bytes, remote_client).await;
    }

    normalize_local_image(source, max_bytes).await
}

fn normalize_data_uri(source: &str, max_bytes: usize) -> anyhow::Result<String> {
    let parsed = parse_data_uri(source).map_err(|reason| MultimodalError::InvalidMarker {
        input: source.to_string(),
        reason,
    })?;

    let (mime, decoded) = if parsed.mime == "application/gzip" {
        let original_mime = data_uri_param(&parsed.params, "original_mime").ok_or_else(|| {
            MultimodalError::InvalidMarker {
                input: source.to_string(),
                reason: "compressed image data URI missing original_mime parameter".to_string(),
            }
        })?;
        (
            original_mime.to_ascii_lowercase(),
            gunzip_data_uri(source, &parsed.bytes, max_bytes)?,
        )
    } else {
        (parsed.mime, parsed.bytes)
    };

    validate_mime(source, &mime)?;
    validate_size(source, decoded.len(), max_bytes)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(decoded)))
}

struct ParsedDataUri {
    mime: String,
    params: Vec<(String, String)>,
    bytes: Vec<u8>,
}

fn parse_data_uri(source: &str) -> Result<ParsedDataUri, String> {
    let Some(comma_idx) = source.find(',') else {
        return Err("expected data URI payload".to_string());
    };

    let header = &source[..comma_idx];
    let payload = source[comma_idx + 1..].trim();

    if !header.contains(";base64") {
        return Err("only base64 data URIs are supported".to_string());
    }

    let mut parts = header.trim_start_matches("data:").split(';');
    let mime = parts.next().unwrap_or_default().trim().to_ascii_lowercase();
    let params = parts
        .filter_map(|part| {
            if part.eq_ignore_ascii_case("base64") {
                return None;
            }
            let (key, value) = part.split_once('=')?;
            Some((
                key.trim().to_ascii_lowercase(),
                percent_decode(value.trim()).unwrap_or_else(|| value.trim().to_string()),
            ))
        })
        .collect::<Vec<_>>();

    let bytes = STANDARD
        .decode(payload)
        .map_err(|error| format!("invalid base64 payload: {error}"))?;

    Ok(ParsedDataUri {
        mime,
        params,
        bytes,
    })
}

fn data_uri_param(params: &[(String, String)], key: &str) -> Option<String> {
    params
        .iter()
        .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
        .map(|(_, value)| value.clone())
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if i + 2 >= bytes.len() {
                return None;
            }
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).ok()?;
            let byte = u8::from_str_radix(hex, 16).ok()?;
            out.push(byte);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).ok()
}

fn gunzip_data_uri(
    source: &str,
    bytes: &[u8],
    max_decompressed_bytes: usize,
) -> anyhow::Result<Vec<u8>> {
    let limit = max_decompressed_bytes.saturating_add(1) as u64;
    let mut decoder = GzDecoder::new(bytes).take(limit);
    let mut out = Vec::new();
    decoder
        .read_to_end(&mut out)
        .map_err(|error| MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: format!("invalid gzip payload: {error}"),
        })?;
    if out.len() > max_decompressed_bytes {
        return Err(MultimodalError::InvalidMarker {
            input: source.to_string(),
            reason: format!("decompressed payload exceeds {max_decompressed_bytes} bytes"),
        }
        .into());
    }
    Ok(out)
}

async fn normalize_remote_image(
    source: &str,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<String> {
    let response = remote_client.get(source).send().await.map_err(|error| {
        MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: format!("HTTP {status}"),
        }
        .into());
    }

    if let Some(content_length) = response.content_length() {
        let content_length = content_length as usize;
        validate_size(source, content_length, max_bytes)?;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    let bytes = response
        .bytes()
        .await
        .map_err(|error| MultimodalError::RemoteFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_size(source, bytes.len(), max_bytes)?;

    let mime = detect_mime(None, bytes.as_ref(), content_type.as_deref()).ok_or_else(|| {
        MultimodalError::UnsupportedMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
        }
    })?;

    validate_mime(source, &mime)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

async fn normalize_local_image(source: &str, max_bytes: usize) -> anyhow::Result<String> {
    let path = Path::new(source);
    if !path.exists() || !path.is_file() {
        return Err(MultimodalError::ImageSourceNotFound {
            input: source.to_string(),
        }
        .into());
    }

    let metadata =
        tokio::fs::metadata(path)
            .await
            .map_err(|error| MultimodalError::LocalReadFailed {
                input: source.to_string(),
                reason: error.to_string(),
            })?;

    validate_size(source, metadata.len() as usize, max_bytes)?;

    let bytes = tokio::fs::read(path)
        .await
        .map_err(|error| MultimodalError::LocalReadFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_size(source, bytes.len(), max_bytes)?;

    let mime =
        detect_mime(Some(path), &bytes, None).ok_or_else(|| MultimodalError::UnsupportedMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
        })?;

    validate_mime(source, &mime)?;

    Ok(format!("data:{mime};base64,{}", STANDARD.encode(bytes)))
}

fn validate_size(source: &str, size_bytes: usize, max_bytes: usize) -> anyhow::Result<()> {
    if size_bytes > max_bytes {
        return Err(MultimodalError::ImageTooLarge {
            input: source.to_string(),
            size_bytes,
            max_bytes,
        }
        .into());
    }

    Ok(())
}

fn validate_mime(source: &str, mime: &str) -> anyhow::Result<()> {
    if ALLOWED_IMAGE_MIME_TYPES.contains(&mime) {
        return Ok(());
    }

    Err(MultimodalError::UnsupportedMime {
        input: source.to_string(),
        mime: mime.to_string(),
    }
    .into())
}

fn detect_mime(
    path: Option<&Path>,
    bytes: &[u8],
    header_content_type: Option<&str>,
) -> Option<String> {
    if let Some(header_mime) = header_content_type.and_then(normalize_content_type) {
        return Some(header_mime);
    }

    if let Some(path) = path {
        if let Some(ext) = path.extension().and_then(|value| value.to_str()) {
            if let Some(mime) = mime_from_extension(ext) {
                return Some(mime.to_string());
            }
        }
    }

    mime_from_magic(bytes).map(ToString::to_string)
}

fn normalize_content_type(content_type: &str) -> Option<String> {
    let mime = content_type.split(';').next()?.trim().to_ascii_lowercase();
    if mime.is_empty() {
        None
    } else {
        Some(mime)
    }
}

fn mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "webp" => Some("image/webp"),
        "gif" => Some("image/gif"),
        "bmp" => Some("image/bmp"),
        _ => None,
    }
}

fn mime_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 8 && bytes.starts_with(&[0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n']) {
        return Some("image/png");
    }

    if bytes.len() >= 3 && bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        return Some("image/jpeg");
    }

    if bytes.len() >= 6 && (bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a")) {
        return Some("image/gif");
    }

    if bytes.len() >= 12 && bytes.starts_with(b"RIFF") && &bytes[8..12] == b"WEBP" {
        return Some("image/webp");
    }

    if bytes.len() >= 2 && bytes.starts_with(b"BM") {
        return Some("image/bmp");
    }

    None
}

// ── File-attachment pipeline ──────────────────────────────────────────
//
// File markers run through a parallel pipeline to image markers but
// with a different end-state: extractable formats inline their text
// payload, binary-only formats surface as metadata-only references.
// The agent never sees raw binary bytes for `[FILE:…]` markers — base64
// inlining is the image pipeline's exclusive contract.

async fn normalize_file_reference(
    source: &str,
    config: &MultimodalFileConfig,
    max_bytes: usize,
    max_extracted_text_chars: usize,
    remote_client: &Client,
) -> anyhow::Result<FilePayload> {
    if source.starts_with("data:") {
        let (bytes, name, mime) = normalize_file_data_uri(source, max_bytes)?;
        return file_payload_from_bytes(
            source,
            bytes,
            name,
            mime,
            config,
            max_extracted_text_chars,
        )
        .await;
    }

    let (bytes, path_hint, name, header_content_type) =
        if source.starts_with("http://") || source.starts_with("https://") {
            if !config.allow_remote_fetch {
                return Err(MultimodalError::RemoteFileFetchDisabled {
                    input: source.to_string(),
                }
                .into());
            }
            let (bytes, name, content_type) =
                fetch_remote_file(source, max_bytes, remote_client).await?;
            (bytes, None, name, content_type)
        } else {
            let (bytes, path, name) = read_local_file(source, max_bytes).await?;
            (bytes, Some(path), name, None)
        };

    let mime = detect_file_mime(path_hint.as_deref(), &bytes, header_content_type.as_deref())
        .ok_or_else(|| MultimodalError::UnsupportedFileMime {
            input: source.to_string(),
            mime: "unknown".to_string(),
            supported: config.allowed_mime_types.join(", "),
        })?;

    file_payload_from_bytes(source, bytes, name, mime, config, max_extracted_text_chars).await
}

fn normalize_file_data_uri(
    source: &str,
    max_bytes: usize,
) -> anyhow::Result<(Vec<u8>, String, String)> {
    let parsed = parse_data_uri(source).map_err(|reason| MultimodalError::InvalidFileMarker {
        input: source.to_string(),
        reason,
    })?;
    let name = data_uri_param(&parsed.params, "name").unwrap_or_else(|| "attachment".to_string());

    let (mime, bytes) = if parsed.mime == "application/gzip" {
        let original_mime = data_uri_param(&parsed.params, "original_mime").ok_or_else(|| {
            MultimodalError::InvalidFileMarker {
                input: source.to_string(),
                reason: "compressed file data URI missing original_mime parameter".to_string(),
            }
        })?;
        (
            original_mime.to_ascii_lowercase(),
            gunzip_data_uri(source, &parsed.bytes, max_bytes)?,
        )
    } else {
        (parsed.mime, parsed.bytes)
    };

    if bytes.len() > max_bytes {
        return Err(MultimodalError::FileTooLarge {
            input: source.to_string(),
            size_bytes: bytes.len(),
            max_bytes,
        }
        .into());
    }

    Ok((bytes, name, mime))
}

async fn file_payload_from_bytes(
    source: &str,
    bytes: Vec<u8>,
    name: String,
    mime: String,
    config: &MultimodalFileConfig,
    max_extracted_text_chars: usize,
) -> anyhow::Result<FilePayload> {
    if !config.is_mime_allowed(&mime) {
        return Err(MultimodalError::UnsupportedFileMime {
            input: source.to_string(),
            mime: mime.clone(),
            supported: config.allowed_mime_types.join(", "),
        }
        .into());
    }

    let size_bytes = bytes.len();

    tracing::debug!(
        target: "multimodal",
        file = %name,
        mime = %mime,
        size_bytes,
        "[multimodal::files] resolved file ref"
    );

    if is_extractable_text_mime(&mime) {
        match extract_utf8_text(&bytes) {
            Ok(raw) => {
                let (text, truncated_chars) = truncate_chars(raw, max_extracted_text_chars);
                if truncated_chars > 0 {
                    tracing::info!(
                        target: "multimodal",
                        file = %name,
                        truncated_chars,
                        max_extracted_text_chars,
                        "[multimodal::files] truncated extracted text"
                    );
                }
                return Ok(FilePayload::Extracted {
                    name,
                    mime,
                    size_bytes,
                    text,
                    truncated_chars,
                });
            }
            Err(reason) => {
                tracing::warn!(
                    target: "multimodal",
                    file = %name,
                    reason = %reason,
                    "[multimodal::files] utf-8 decode failed, degrading to reference"
                );
            }
        }
    }

    if mime == "application/pdf" {
        match extract_pdf_text(bytes.clone()).await {
            Ok(raw) => {
                let (text, truncated_chars) = truncate_chars(raw, max_extracted_text_chars);
                if truncated_chars > 0 {
                    tracing::info!(
                        target: "multimodal",
                        file = %name,
                        truncated_chars,
                        max_extracted_text_chars,
                        "[multimodal::files] truncated extracted text"
                    );
                }
                return Ok(FilePayload::Extracted {
                    name,
                    mime,
                    size_bytes,
                    text,
                    truncated_chars,
                });
            }
            Err(reason) => {
                tracing::warn!(
                    target: "multimodal",
                    file = %name,
                    reason = %reason,
                    "[multimodal::files] PDF extraction failed, degrading to reference"
                );
            }
        }
    }

    let sha256_prefix = sha256_prefix(&bytes);
    Ok(FilePayload::Reference {
        name,
        mime,
        size_bytes,
        sha256_prefix,
    })
}

async fn read_local_file(
    source: &str,
    max_bytes: usize,
) -> anyhow::Result<(Vec<u8>, std::path::PathBuf, String)> {
    let path = Path::new(source).to_path_buf();
    if !path.exists() || !path.is_file() {
        return Err(MultimodalError::FileSourceNotFound {
            input: source.to_string(),
        }
        .into());
    }

    let metadata =
        tokio::fs::metadata(&path)
            .await
            .map_err(|error| MultimodalError::LocalFileReadFailed {
                input: source.to_string(),
                reason: error.to_string(),
            })?;

    validate_file_size(source, metadata.len() as usize, max_bytes)?;

    let bytes =
        tokio::fs::read(&path)
            .await
            .map_err(|error| MultimodalError::LocalFileReadFailed {
                input: source.to_string(),
                reason: error.to_string(),
            })?;

    validate_file_size(source, bytes.len(), max_bytes)?;

    let name = path
        .file_name()
        .and_then(|value| value.to_str())
        .map(ToString::to_string)
        .unwrap_or_else(|| source.to_string());

    Ok((bytes, path, name))
}

async fn fetch_remote_file(
    source: &str,
    max_bytes: usize,
    remote_client: &Client,
) -> anyhow::Result<(Vec<u8>, String, Option<String>)> {
    let response = remote_client.get(source).send().await.map_err(|error| {
        MultimodalError::RemoteFileFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        }
    })?;

    let status = response.status();
    if !status.is_success() {
        return Err(MultimodalError::RemoteFileFetchFailed {
            input: source.to_string(),
            reason: format!("HTTP {status}"),
        }
        .into());
    }

    if let Some(content_length) = response.content_length() {
        let content_length = content_length as usize;
        validate_file_size(source, content_length, max_bytes)?;
    }

    let content_type = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(ToString::to_string);

    let bytes = response
        .bytes()
        .await
        .map_err(|error| MultimodalError::RemoteFileFetchFailed {
            input: source.to_string(),
            reason: error.to_string(),
        })?;

    validate_file_size(source, bytes.len(), max_bytes)?;

    let name = source
        .rsplit('/')
        .next()
        .filter(|segment| !segment.is_empty())
        .unwrap_or(source)
        .to_string();

    Ok((bytes.to_vec(), name, content_type))
}

fn validate_file_size(source: &str, size_bytes: usize, max_bytes: usize) -> anyhow::Result<()> {
    if size_bytes > max_bytes {
        return Err(MultimodalError::FileTooLarge {
            input: source.to_string(),
            size_bytes,
            max_bytes,
        }
        .into());
    }

    Ok(())
}

fn is_extractable_text_mime(mime: &str) -> bool {
    matches!(mime, "text/plain" | "text/csv" | "text/markdown")
}

/// Best-effort UTF-8 decode. Strict decode wins; on failure falls back
/// to `from_utf8_lossy` (replaces invalid sequences with U+FFFD). The
/// returned `Err` is reserved for future hard-fail modes — currently
/// the function never returns `Err`, but keeping the result type
/// preserves the option to surface lossy decoding to the caller.
fn extract_utf8_text(bytes: &[u8]) -> Result<String, String> {
    match std::str::from_utf8(bytes) {
        Ok(text) => Ok(text.to_string()),
        Err(_) => Ok(String::from_utf8_lossy(bytes).into_owned()),
    }
}

/// Run `pdf-extract` on a copy of `bytes` inside a `spawn_blocking`
/// worker, bounded by [`PDF_EXTRACTION_TIMEOUT`]. Returns the raw
/// extracted text on success; on timeout / panic / parse error the
/// caller degrades the file to [`FilePayload::Reference`] rather than
/// surface the failure to the user (avoids Sentry noise on broken PDFs).
async fn extract_pdf_text(bytes: Vec<u8>) -> Result<String, String> {
    let extraction = tokio::task::spawn_blocking(move || {
        pdf_extract::extract_text_from_mem(&bytes).map_err(|error| error.to_string())
    });

    match tokio::time::timeout(PDF_EXTRACTION_TIMEOUT, extraction).await {
        Ok(Ok(Ok(text))) => Ok(text),
        Ok(Ok(Err(reason))) => Err(reason),
        Ok(Err(join_error)) => Err(format!("pdf extraction worker panicked: {join_error}")),
        Err(_) => Err(format!(
            "pdf extraction exceeded {}s timeout",
            PDF_EXTRACTION_TIMEOUT.as_secs()
        )),
    }
}

/// Truncate `text` to at most `max_chars` Unicode scalar values, leaving
/// room for the rendered `"\n[…truncated {dropped} chars]"` suffix.
/// The reservation uses [`TEXT_TRUNCATION_SUFFIX_BUDGET`] — the
/// worst-case rendered length — so the final `text + suffix` payload
/// always stays inside `max_chars` regardless of the actual dropped
/// digit count. Returns the (possibly-trimmed) text and the count of
/// chars dropped (0 when no truncation happened).
fn truncate_chars(text: String, max_chars: usize) -> (String, usize) {
    let total = text.chars().count();
    if total <= max_chars {
        return (text, 0);
    }

    let suffix_chars = TEXT_TRUNCATION_SUFFIX_BUDGET.chars().count();
    let keep = max_chars.saturating_sub(suffix_chars);
    let truncated: String = text.chars().take(keep).collect();
    let dropped = total.saturating_sub(keep);
    (truncated, dropped)
}

fn sha256_prefix(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|byte| format!("{:02x}", byte)).collect();
    hex.chars().take(16).collect()
}

fn detect_file_mime(
    path: Option<&Path>,
    bytes: &[u8],
    header_content_type: Option<&str>,
) -> Option<String> {
    if let Some(header_mime) = header_content_type.and_then(normalize_content_type) {
        if file_mime_known(&header_mime) {
            return Some(header_mime);
        }
    }

    if let Some(path) = path {
        if let Some(ext) = path.extension().and_then(|value| value.to_str()) {
            if let Some(mime) = file_mime_from_extension(ext) {
                return Some(mime.to_string());
            }
        }
    }

    if let Some(mime) = file_mime_from_magic(bytes) {
        return Some(mime.to_string());
    }

    if looks_like_utf8_text(bytes) {
        return Some("text/plain".to_string());
    }

    None
}

fn file_mime_known(mime: &str) -> bool {
    file_mime_from_extension(mime).is_some()
        || matches!(
            mime,
            "application/pdf"
                | "text/plain"
                | "text/csv"
                | "text/markdown"
                | "application/zip"
                | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
                | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
                | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
                | "application/octet-stream"
        )
}

fn file_mime_from_extension(ext: &str) -> Option<&'static str> {
    match ext.to_ascii_lowercase().as_str() {
        "pdf" => Some("application/pdf"),
        "txt" => Some("text/plain"),
        "md" | "markdown" => Some("text/markdown"),
        "csv" => Some("text/csv"),
        "zip" => Some("application/zip"),
        "xlsx" => Some("application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"),
        "docx" => Some("application/vnd.openxmlformats-officedocument.wordprocessingml.document"),
        "pptx" => Some("application/vnd.openxmlformats-officedocument.presentationml.presentation"),
        _ => None,
    }
}

fn file_mime_from_magic(bytes: &[u8]) -> Option<&'static str> {
    if bytes.len() >= 5 && bytes.starts_with(b"%PDF-") {
        return Some("application/pdf");
    }

    // OOXML formats (xlsx/docx/pptx) and plain zip all share the
    // PK\x03\x04 ZIP local-file-header magic; without parsing the
    // central directory we cannot distinguish them, so callers must
    // rely on the file extension for OOXML vs zip discrimination.
    if bytes.len() >= 4 && bytes.starts_with(&[b'P', b'K', 0x03, 0x04]) {
        return Some("application/zip");
    }

    None
}

/// Crude UTF-8 sniff: bytes parse as UTF-8 and contain at least one
/// printable character. Used as a last-resort fallback so unlabeled
/// .log / .ini / source files are still recognised as text.
fn looks_like_utf8_text(bytes: &[u8]) -> bool {
    if bytes.is_empty() {
        return false;
    }
    match std::str::from_utf8(bytes) {
        Ok(text) => text
            .chars()
            .any(|c| !c.is_control() || matches!(c, '\n' | '\r' | '\t')),
        Err(_) => false,
    }
}

#[cfg(test)]
#[path = "multimodal_tests.rs"]
mod tests;
