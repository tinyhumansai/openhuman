//! OpenHuman's half of attachment handling.
//!
//! The pipeline itself — marker parsing, `data:` URI decoding, MIME detection,
//! size and count limits, the rendered payload — lives in
//! [`tinyagents_harness::multimodal`]. What is left here is everything that
//! depends on *this* host rather than on any host:
//!
//! | Kept here | Why it cannot be generic |
//! | --- | --- |
//! | [`ChatMessage`] adapters | the durable transcript record is OpenHuman's |
//! | Config mapping | `MultimodalConfig` is a `config.toml` schema type |
//! | The `reqwest::Client` | the runtime proxy and its timeouts are host policy |
//! | [`DocumentsTextExtractor`] | PDF text comes from the `tinydocs` module, behind the `documents` gate and a host-chosen deadline |
//! | The attachments stash | `<workspace>/attachments`, its size cap, and its TTL |
//!
//! The stash is the least obvious of these and the most load-bearing. A
//! persisted message must never carry a raw `[IMAGE:data:…]` URI: a multi-MB
//! base64 blob floods the prompt-injection scan, the memory auto-save (N chunks
//! to embed), and the cross-thread JSONL index. So at ingress
//! [`stash_image_attachments`] replaces each image marker with a compact
//! `[Image: image #att:<id>]` placeholder and writes the bytes to disk, and at
//! dispatch [`rehydrate_image_placeholders`] puts them back — but only for a
//! vision-capable model, since a text-only one gains nothing from an image it
//! cannot see.
//!
//! Disk-backed rather than an in-memory FIFO so attachments survive process
//! restarts and long delegation chains: a sub-agent spawned several hops after
//! ingress still resolves the image by id.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;

use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::config::{
    build_runtime_proxy_client_with_timeouts, MultimodalConfig, MultimodalFileConfig,
};

use tinyagents_harness::multimodal::{
    self as mm,
    config::{FileLimits, ImageLimits},
    markers, mime as mm_mime,
    payload::sha256_prefix,
    resolve::{resolve_file, resolve_image, TextExtractor},
};

pub use tinyagents_harness::multimodal::{FilePayload, MultimodalError};

/// Hard upper bound on how long the `tinydocs` module may spend extracting a
/// PDF's text layer before the attempt is abandoned and the file degrades to a
/// metadata-only reference.
///
/// The crate's [`TextExtractor`] carries no deadline by design: the cost is set
/// by the document rather than by anything validated beforehand, and only a
/// host knows how long an attachment is worth waiting for. PDFs that choke the
/// parser — extremely large, encrypted, malformed — must not stall a chat turn.
#[cfg(feature = "documents")]
const PDF_EXTRACTION_TIMEOUT: Duration = Duration::from_secs(60);

/// The result of resolving every marker in a turn's messages.
#[derive(Debug, Clone)]
pub struct PreparedMessages {
    /// The messages with every marker resolved into a payload.
    pub messages: Vec<ChatMessage>,
    /// Whether the turn carried any image markers.
    pub contains_images: bool,
    /// Whether the turn carried any file markers.
    pub contains_files: bool,
}

// ── Config mapping ───────────────────────────────────────────────────────
//
// Follows the `session_config_from` precedent in `openhuman::tinyagents::config`:
// the crate owns the struct, the host maps its schema into it. The clamping
// stays crate-side, so these are plain field copies — if they ever grow a rule,
// that rule belongs in the crate instead.

/// Map the host image config onto the crate's limits.
fn image_limits(config: &MultimodalConfig) -> ImageLimits {
    ImageLimits {
        max_images: config.max_images,
        max_image_size_mb: config.max_image_size_mb,
        allow_remote_fetch: config.allow_remote_fetch,
    }
}

/// Map the host file config onto the crate's limits.
///
/// `max_files` is copied verbatim, including the `0` hard-disable sentinel that
/// [`MultimodalFileConfig::for_untrusted_channel_input`] sets — the crate
/// checks it before its own clamp.
fn file_limits(config: &MultimodalFileConfig) -> FileLimits {
    FileLimits {
        max_files: config.max_files,
        max_file_size_mb: config.max_file_size_mb,
        max_extracted_text_chars: config.max_extracted_text_chars,
        allow_remote_fetch: config.allow_remote_fetch,
        allowed_mime_types: config.allowed_mime_types.clone(),
    }
}

/// The HTTP client remote references are fetched with.
///
/// Built per call rather than cached because the runtime proxy configuration
/// can change under a running core, and a stale client would keep dialling the
/// old one.
fn remote_client() -> Client {
    build_runtime_proxy_client_with_timeouts("provider.ollama", 30, 10)
}

// ── Text extraction ──────────────────────────────────────────────────────

/// The host's [`TextExtractor`]: PDF text through the `tinydocs` module,
#[cfg_attr(feature = "documents", doc = "bounded by [`PDF_EXTRACTION_TIMEOUT`].")]
#[cfg_attr(
    not(feature = "documents"),
    doc = "bounded by the module's own deadline."
)]
///
/// Claims `application/pdf` and nothing else. Every other binary format
/// surfaces as a [`FilePayload::Reference`] without the module ever seeing the
/// bytes — which matters more than it looks, because the module runs
/// out-of-process and a speculative call would cost a bus round trip and a copy
/// of the file per attachment.
pub struct DocumentsTextExtractor;

#[async_trait]
impl TextExtractor for DocumentsTextExtractor {
    fn handles(&self, mime: &str) -> bool {
        mime == "application/pdf"
    }

    /// Extract a PDF's text layer.
    ///
    /// The parsing runs in the module, which owns its own blocking pool, so
    /// there is no `spawn_blocking` here — but the deadline stays.
    ///
    /// The bytes ride a bus stream rather than a JSON frame: a `.pdf` is
    /// bounded only by what the multimodal config accepted, which is far past
    /// what a frame holds.
    ///
    /// Every failure — an unavailable module, a damaged document, the deadline
    /// — is reported the same way, and the caller degrades the file to a
    /// [`FilePayload::Reference`] rather than surfacing it (which avoids Sentry
    /// noise on broken PDFs).
    #[cfg(feature = "documents")]
    async fn extract(&self, _mime: &str, bytes: &[u8]) -> Result<String, String> {
        use crate::openhuman::modules::documents;

        match tokio::time::timeout(PDF_EXTRACTION_TIMEOUT, async {
            let config = crate::openhuman::config::Config::load_or_init()
                .await
                .map_err(|error| format!("config unavailable for pdf extraction: {error}"))?;
            documents::extract_text(&config, bytes)
                .await
                .map_err(|error| error.to_string())
        })
        .await
        {
            Ok(Ok(text)) => Ok(text),
            Ok(Err(error)) => Err(error.to_string()),
            Err(_) => Err(format!(
                "pdf extraction exceeded {}s timeout",
                PDF_EXTRACTION_TIMEOUT.as_secs()
            )),
        }
    }

    /// Disabled variant when the `documents` feature is off: no PDF parser is
    /// compiled in, so signal failure and let the caller degrade the file to a
    /// [`FilePayload::Reference`] — the same path a parse error or a blown
    /// deadline takes.
    #[cfg(not(feature = "documents"))]
    async fn extract(&self, _mime: &str, _bytes: &[u8]) -> Result<String, String> {
        log::debug!(
            "[multimodal] pdf text extraction skipped: built without the `documents` feature"
        );
        Err("pdf text extraction disabled (built without the `documents` feature)".to_string())
    }
}

// ── Marker helpers over `ChatMessage` ────────────────────────────────────

/// Strip every `[IMAGE:…]` marker and return `(cleaned_text, refs_in_order)`.
pub fn parse_image_markers(content: &str) -> (String, Vec<String>) {
    markers::parse_image_markers(content)
}

/// Strip every `[FILE:…]` marker and return `(cleaned_text, refs_in_order)`.
pub fn parse_file_markers(content: &str) -> (String, Vec<String>) {
    markers::parse_file_markers(content)
}

/// The base64 payload Ollama's `images` array expects, or `None`.
pub fn extract_ollama_image_payload(image_ref: &str) -> Option<String> {
    markers::extract_ollama_image_payload(image_ref)
}

/// Count `[IMAGE:…]` markers in the **latest** user message only.
///
/// Earlier versions summed markers across every user-role message in the
/// history, which made the per-turn `max_images` cap drift upward over a long
/// conversation: a thread that attached three images on turn 1 already counted
/// them again on turn 2 even when the new user message had no attachments at
/// all. Looking only at the most recent user message matches the user's intent
/// ("how many am I attaching THIS turn") and keeps the cap stable.
pub fn count_image_markers(messages: &[ChatMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| markers::parse_image_markers(&m.content).1.len())
        .unwrap_or(0)
}

/// Whether the latest user message carries any image marker.
pub fn contains_image_markers(messages: &[ChatMessage]) -> bool {
    count_image_markers(messages) > 0
}

/// Count `[FILE:…]` markers in the **latest** user message only — same
/// per-turn semantics as [`count_image_markers`].
pub fn count_file_markers(messages: &[ChatMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| markers::parse_file_markers(&m.content).1.len())
        .unwrap_or(0)
}

/// Whether the latest user message carries any file marker.
pub fn contains_file_markers(messages: &[ChatMessage]) -> bool {
    count_file_markers(messages) > 0
}

fn latest_user_message(messages: &[ChatMessage]) -> Option<&ChatMessage> {
    messages.iter().rev().find(|m| m.role == "user")
}

// ── Provider dispatch ────────────────────────────────────────────────────

/// Resolve every marker in `messages` into a provider-ready payload.
///
/// Counts are checked against the raw markers before any read happens: a cap
/// enforced after the fetch is not a cap.
pub async fn prepare_messages_for_provider(
    messages: &[ChatMessage],
    image_config: &MultimodalConfig,
    file_config: &MultimodalFileConfig,
) -> anyhow::Result<PreparedMessages> {
    let images = image_limits(image_config);
    let files = file_limits(file_config);

    let (max_images, _) = images.effective();
    let max_image_bytes = images.max_image_bytes();

    let (max_files, _, max_extracted_text_chars) = files.effective();
    let max_file_bytes = files.max_file_bytes();

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
    // (and the triage arm) sets `max_files: 0` as a sentinel meaning "reject
    // every `[FILE:…]` marker before any disk read." The crate's clamp lifts
    // 0 → 1, so without this pre-check a single attacker-supplied
    // `[FILE:/etc/passwd]` would slip through (`1 > 1` is false). Honour the
    // raw value here so the channel / triage hardening is actually enforced.
    if files.files_disabled() && found_files > 0 {
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

    let client = remote_client();

    let mut normalized_messages = Vec::with_capacity(messages.len());
    for message in messages {
        if message.role != "user" {
            normalized_messages.push(message.clone());
            continue;
        }

        let (text_after_images, image_refs) = markers::parse_image_markers(&message.content);
        let (cleaned_text, file_refs) = markers::parse_file_markers(&text_after_images);

        if image_refs.is_empty() && file_refs.is_empty() {
            normalized_messages.push(message.clone());
            continue;
        }

        let mut normalized_image_refs = Vec::with_capacity(image_refs.len());
        for reference in image_refs {
            normalized_image_refs
                .push(resolve_image(&reference, &images, max_image_bytes, &client).await?);
        }

        let mut file_payloads = Vec::with_capacity(file_refs.len());
        for reference in file_refs {
            file_payloads.push(
                resolve_file(
                    &reference,
                    &files,
                    max_file_bytes,
                    max_extracted_text_chars,
                    &client,
                    &DocumentsTextExtractor,
                )
                .await?,
            );
        }

        let content =
            mm::compose_multimodal_message(&cleaned_text, &normalized_image_refs, &file_payloads);
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

// ── Ingress ──────────────────────────────────────────────────────────────

/// Ingress-time file extraction.
///
/// Replaces every `[FILE:…]` marker in a raw user message with its
/// extracted-text block, or a content-less `[FILE-ATTACHED: …]` placeholder
/// when extraction fails. `[IMAGE:…]` markers are deliberately left untouched
/// here — they are handled at provider dispatch, where vision needs the inline
/// data URI.
///
/// Run this at channel ingress, BEFORE the message is persisted to history,
/// auto-saved to the memory store, appended to the cross-thread JSONL, or
/// scanned for prompt injection — so the multi-MB base64 data URI never
/// survives past the front door. Idempotent: a message with no `[FILE:` marker
/// (already-rewritten `[FILE-EXTRACTED]` text included) is returned unchanged.
pub async fn inline_file_attachments(message: &str, file_config: &MultimodalFileConfig) -> String {
    if !message.contains(markers::FILE_MARKER_PREFIX) {
        return message.to_string();
    }
    let (cleaned, file_refs) = markers::parse_file_markers(message);
    if file_refs.is_empty() {
        return message.to_string();
    }

    let files = file_limits(file_config);
    let (max_files, _, max_extracted_text_chars) = files.effective();
    let max_file_bytes = files.max_file_bytes();
    // Enforce the per-turn file cap at ingress: rewriting the markers removes
    // the count check `prepare_messages_for_provider` would otherwise do.
    // `files_disabled()` is the hard-disable sentinel — read nothing. Over-cap
    // refs degrade to a content-less placeholder rather than being read.
    let read_cap = if files.files_disabled() { 0 } else { max_files };
    let client = remote_client();

    let mut payloads = Vec::with_capacity(file_refs.len());
    for (idx, reference) in file_refs.iter().enumerate() {
        if idx >= read_cap {
            payloads.push(FilePayload::placeholder(
                "attachment (over file limit)",
                "skipped",
            ));
            continue;
        }
        match resolve_file(
            reference,
            &files,
            max_file_bytes,
            max_extracted_text_chars,
            &client,
            &DocumentsTextExtractor,
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
                payloads.push(FilePayload::placeholder("attachment", "unavailable"));
            }
        }
    }

    let rewritten = mm::compose_multimodal_message(&cleaned, &[], &payloads);
    tracing::info!(
        target: "multimodal",
        files = payloads.len(),
        before_chars = message.chars().count(),
        after_chars = rewritten.chars().count(),
        "[multimodal::files][ingress] inlined file attachments — data URI replaced with extracted text/placeholder before persistence"
    );
    rewritten
}

/// Ingress-time image stashing. Replaces every `[IMAGE:data:…]` marker with a
/// `[Image: image #att:<id>]` placeholder and stashes the decoded canonical
/// data URI, so the multi-MB base64 never persists. Idempotent (no `[IMAGE:`
/// ⇒ no-op).
pub async fn stash_image_attachments(message: &str, image_config: &MultimodalConfig) -> String {
    if !message.contains(markers::IMAGE_MARKER_PREFIX) {
        return message.to_string();
    }
    let (cleaned, image_refs) = markers::parse_image_markers(message);
    if image_refs.is_empty() {
        return message.to_string();
    }

    let images = image_limits(image_config);
    let (max_images, _) = images.effective();
    let max_image_bytes = images.max_image_bytes();
    let client = remote_client();

    let mut placeholders = Vec::with_capacity(image_refs.len());
    for (idx, reference) in image_refs.iter().enumerate() {
        // Enforce the per-turn image cap at ingress: over-cap markers degrade
        // to a text placeholder and are never read or stashed (rewriting the
        // markers removes the count check `prepare_messages_for_provider` would
        // otherwise apply, and bounds stash growth per message).
        if idx >= max_images {
            placeholders.push("[Image: (over image limit)]".to_string());
            continue;
        }
        match resolve_image(reference, &images, max_image_bytes, &client).await {
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
                placeholders.push(markers::image_placeholder(&id));
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

// ── Placeholders over `ChatMessage` ──────────────────────────────────────

/// Extract the `[Image: … #att:<id>]` sidecar placeholder tokens from `text`,
/// in order. Used to forward a user's attached images into a delegated vision
/// sub-agent's prompt so its turn rehydrates them (the orchestrator itself, on
/// a non-vision tier, keeps the placeholder as text and never sees the image).
pub fn extract_image_placeholders_in_text(text: &str) -> Vec<String> {
    markers::extract_image_placeholders_in_text(text)
}

/// True if any message carries an `[Image: … #att:<id>]` sidecar placeholder.
pub fn has_image_placeholders(messages: &[ChatMessage]) -> bool {
    messages
        .iter()
        .any(|m| markers::text_has_image_placeholders(&m.content))
}

/// Rehydrate `[Image: … #att:<id>]` placeholders back into inline
/// `[IMAGE:<path>]` markers pointing at the on-disk attachment, returning a
/// provider-only copy. Resolution re-reads the file at dispatch. Placeholders
/// whose id is absent (file evicted/swept, or written by a different workspace)
/// keep their text. Call ONLY for vision-capable models.
pub fn rehydrate_image_placeholders(messages: &[ChatMessage]) -> Vec<ChatMessage> {
    let index = build_attachment_index();
    messages
        .iter()
        .map(|m| {
            if !markers::text_has_image_placeholders(&m.content) {
                return m.clone();
            }
            ChatMessage {
                id: m.id.clone(),
                role: m.role.clone(),
                content: markers::rehydrate_placeholders_in_text(&m.content, &index),
                extra_metadata: m.extra_metadata.clone(),
            }
        })
        .collect()
}

// ── The on-disk attachment stash ─────────────────────────────────────────

/// Soft cap on the on-disk attachments directory. After each write, oldest
/// files (by mtime) are evicted until the total is back under this bound.
const ATTACHMENTS_MAX_BYTES: u64 = 256 * 1024 * 1024;
/// Age after which an attachment is considered stale and removed by the startup
/// sweep ([`sweep_stale_attachments`]).
const ATTACHMENTS_TTL: Duration = Duration::from_secs(24 * 60 * 60);

/// Process-global on-disk attachments directory. Installed once at core startup
/// via [`init_attachments_dir`].
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
/// [`init_attachments_dir`]). The persistence-pollution fix and rehydration
/// both hold either way.
fn attachments_dir() -> PathBuf {
    ATTACHMENTS_DIR
        .get()
        .cloned()
        .unwrap_or_else(fallback_attachments_dir)
}

/// Per-user fallback attachments dir used only when [`init_attachments_dir`]
/// was never called. Uses the OS user cache dir (e.g. `~/Library/Caches/…`,
/// `~/.cache/…`) so persisted image bytes aren't dropped into a world-readable
/// shared `temp_dir()` on multi-user hosts. Only falls back to `temp_dir()`
/// when no user cache dir can be resolved at all.
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
    let parsed = mm::data_uri::parse_data_uri(data_uri)
        .map_err(|reason| anyhow::anyhow!("cannot decode stashed data URI: {reason}"))?;
    let ext = mm_mime::image_ext_from_mime(&parsed.mime).unwrap_or("img");
    let dir = attachments_dir();
    tokio::fs::create_dir_all(&dir).await?;
    let final_path = dir.join(format!("{id}.{ext}"));
    if tokio::fs::try_exists(&final_path).await.unwrap_or(false) {
        // Content-addressing deduplicates identical images, but a new message
        // can legitimately reference a file whose previous reference is older
        // than the startup TTL. Refresh its mtime so the immediately following
        // sweep cannot reclaim an attachment that was just reused.
        let touch_path = final_path.clone();
        tokio::task::spawn_blocking(move || -> std::io::Result<()> {
            std::fs::File::open(touch_path)?
                .set_times(std::fs::FileTimes::new().set_modified(std::time::SystemTime::now()))
        })
        .await??;
        return Ok(final_path); // content-addressed: already persisted
    }
    let tmp_path = dir.join(format!(".{id}.{ext}.tmp"));
    tokio::fs::write(&tmp_path, &parsed.bytes).await?;
    tokio::fs::rename(&tmp_path, &final_path).await?;
    enforce_attachments_cap(&dir).await;
    Ok(final_path)
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
/// [`ATTACHMENTS_MAX_BYTES`]. Best-effort.
async fn enforce_attachments_cap(dir: &Path) {
    let mut files: Vec<(PathBuf, std::time::SystemTime, u64)> = Vec::new();
    let mut total: u64 = 0;
    let Ok(mut rd) = tokio::fs::read_dir(dir).await else {
        return;
    };
    while let Ok(Some(entry)) = rd.next_entry().await {
        // Skip in-flight `.<id>.<ext>.tmp` writes (mirrors
        // `build_attachment_index`) so concurrent atomic writes aren't evicted
        // out from under a rename.
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

#[cfg(test)]
#[path = "multimodal_tests.rs"]
mod tests;
