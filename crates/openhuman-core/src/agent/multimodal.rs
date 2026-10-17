//! Legacy attachment marker compatibility and configuration adapters.
//!
//! New uploads are handled by `agent::attachments`: originals live beneath
//! the acting workspace and provider bytes are prepared only on request copies.
//! The older stash remains readable so existing conversations can resume.

use crate::core::runtime::DispatchCtx;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::OnceLock;
use std::time::Duration;

use async_trait::async_trait;
use reqwest::Client;

use crate::config::{
    build_runtime_proxy_client_with_timeouts, MultimodalConfig, MultimodalFileConfig,
};
use tinyagents_session::transcript::{TranscriptMessage, TranscriptPart};

use tinyagents_harness::multimodal::{
    self as mm,
    config::{FileLimits, ImageLimits},
    markers,
    payload::sha256_prefix,
    resolve::{resolve_file, resolve_image, TextExtractor},
    BlobAttachmentStash,
};

use tinyagents_harness::multimodal::{FilePayload, MultimodalError};

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
    pub messages: Vec<TranscriptMessage>,
    /// Whether the turn carried any image markers.
    pub contains_images: bool,
    /// Whether the turn carried any file markers.
    pub contains_files: bool,
}

// ── Config mapping ───────────────────────────────────────────────────────
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
/// Claims `application/pdf` and nothing else. The native module runs in the
/// core process; a wait deadline does not cancel its blocking parser work.
pub struct DocumentsTextExtractor<'a> {
    /// The context the extraction reads its config under.
    #[cfg_attr(not(feature = "documents"), allow(dead_code))]
    pub ctx: &'a DispatchCtx,
}

#[async_trait]
impl TextExtractor for DocumentsTextExtractor<'_> {
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
        use crate::modules::documents;

        match tokio::time::timeout(PDF_EXTRACTION_TIMEOUT, async {
            let config = crate::config::ops::load_current_or_init(self.ctx)
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

// ── Marker helpers over `TranscriptMessage` ────────────────────────────────────

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
pub fn count_image_markers(messages: &[TranscriptMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| markers::parse_image_markers(&m.content).1.len())
        .unwrap_or(0)
}

/// Count `[FILE:…]` markers in the **latest** user message only — same
/// per-turn semantics as [`count_image_markers`].
pub fn count_file_markers(messages: &[TranscriptMessage]) -> usize {
    latest_user_message(messages)
        .map(|m| markers::parse_file_markers(&m.content).1.len())
        .unwrap_or(0)
}

fn latest_user_message(messages: &[TranscriptMessage]) -> Option<&TranscriptMessage> {
    messages.iter().rev().find(|m| m.role == "user")
}

// ── Provider dispatch ────────────────────────────────────────────────────

/// Resolve every marker in `messages` into a provider-ready payload.
///
/// Counts are checked against the raw markers before any read happens: a cap
/// enforced after the fetch is not a cap.
pub async fn prepare_messages_for_provider(
    ctx: &DispatchCtx,
    messages: &[TranscriptMessage],
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
            // A `data:` image is an inline byte payload, not a fetchable URL.
            // Keep the wire contract strict before handing it to TinyAgents:
            // accepting a non-base64 form silently turned malformed image
            // input into a provider-visible marker after the resolver update.
            if reference.trim_start().starts_with("data:") && !data_uri_uses_base64(&reference) {
                return Err(anyhow::anyhow!("only base64 data URIs are supported"));
            }
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
                    &DocumentsTextExtractor { ctx },
                )
                .await?,
            );
        }

        let content =
            mm::compose_multimodal_message(&cleaned_text, &normalized_image_refs, &file_payloads);
        normalized_messages.push(TranscriptMessage {
            content,
            cache_breakpoints: Vec::new(),
            ..message.clone()
        });
    }

    Ok(PreparedMessages {
        messages: normalized_messages,
        contains_images: found_images > 0,
        contains_files: found_files > 0,
    })
}

/// Whether a `data:` URI declares a base64 payload before its first comma.
///
/// Callers first establish that the value starts with `data:`; ordinary image
/// paths and remote URLs intentionally do not pass through this check.
fn data_uri_uses_base64(reference: &str) -> bool {
    reference
        .split_once(',')
        .map(|(header, _)| {
            header
                .split(';')
                .any(|parameter| parameter.trim().eq_ignore_ascii_case("base64"))
        })
        .unwrap_or(false)
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
pub async fn inline_file_attachments(
    ctx: &DispatchCtx,
    message: &str,
    file_config: &MultimodalFileConfig,
) -> String {
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
            &DocumentsTextExtractor { ctx },
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
                let written = match stash() {
                    Some(stash) => stash.write(&id, &data_uri).await,
                    None => Err(anyhow::anyhow!("no attachment stash for this tenant")),
                };
                match written {
                    Ok(stashed) => tracing::debug!(
                        target: "multimodal",
                        id = %id,
                        len = stashed.len,
                        "[multimodal::images][stash] persisted attachment to the blob store"
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

// ── Placeholders over `TranscriptMessage` ──────────────────────────────────────

/// Extract the `[Image: … #att:<id>]` sidecar placeholder tokens from `text`,
/// in order. Used to forward a user's attached images into a delegated vision
/// sub-agent's prompt so its turn rehydrates them (the orchestrator itself, on
/// a non-vision tier, keeps the placeholder as text and never sees the image).
pub fn extract_image_placeholders_in_text(text: &str) -> Vec<String> {
    markers::extract_image_placeholders_in_text(text)
}

/// True if any message carries an `[Image: … #att:<id>]` sidecar placeholder.
pub fn has_image_placeholders(messages: &[TranscriptMessage]) -> bool {
    messages.iter().any(row_has_image_placeholders)
}

/// Whether a row's text (its `content`, or any text part of a user row with
/// typed image parts) carries a sidecar placeholder.
fn row_has_image_placeholders(row: &TranscriptMessage) -> bool {
    markers::text_has_image_placeholders(&row.content)
        || row.parts.as_deref().is_some_and(|parts| {
            parts.iter().any(|part| {
                matches!(part, TranscriptPart::Text { text } if markers::text_has_image_placeholders(text))
            })
        })
}

/// Rehydrate `[Image: … #att:<id>]` placeholders back into `[IMAGE:…]` markers,
/// returning a provider-only copy. An attachment stashed in the blob store
/// comes back as an inline `data:` URI; a legacy sidecar file in the
/// attachments directory as its path (re-read at dispatch). Placeholders whose
/// id is in neither (evicted, or written by a different workspace) keep their
/// text. The attachment host migrates managed sidecars into acting
/// workspace originals before provider resolution, including text fallbacks.
pub async fn rehydrate_image_placeholders(
    messages: &[TranscriptMessage],
) -> Vec<TranscriptMessage> {
    let index = build_attachment_index(messages).await;
    messages
        .iter()
        .map(|m| {
            if !row_has_image_placeholders(m) {
                return m.clone();
            }
            TranscriptMessage {
                content: markers::rehydrate_placeholders_in_text(&m.content, &index),
                parts: m.parts.as_ref().map(|parts| {
                    parts
                        .iter()
                        .map(|part| match part {
                            TranscriptPart::Text { text } => TranscriptPart::Text {
                                text: markers::rehydrate_placeholders_in_text(text, &index),
                            },
                            image => image.clone(),
                        })
                        .collect()
                }),
                cache_breakpoints: Vec::new(),
                ..m.clone()
            }
        })
        .collect()
}

// ── The attachment stash ─────────────────────────────────────────────────

/// Process-global legacy attachments directory (`<workspace>/attachments`),
/// installed once at core startup via [`init_attachments_dir`]. Sidecar files
/// written before the blob stash stay readable from here.
static ATTACHMENTS_DIR: OnceLock<PathBuf> = OnceLock::new();

/// The workspace whose storage backend holds the blob stash; the parent of the
/// directory given to [`init_attachments_dir`].
static ATTACHMENTS_WORKSPACE: OnceLock<PathBuf> = OnceLock::new();

/// Install the legacy attachments directory (`<workspace>/attachments`) and,
/// from its parent, the workspace the blob stash lives in. Call once at core
/// startup. Idempotent — first writer wins.
pub fn init_attachments_dir(dir: PathBuf) {
    if let Some(workspace) = dir.parent() {
        let _ = ATTACHMENTS_WORKSPACE.set(workspace.to_path_buf());
    }
    let _ = ATTACHMENTS_DIR.set(dir);
}

/// Resolve the legacy attachments dir, falling back to a **per-user private**
/// dir when unset (CLI / direct invocation / tests that never called
/// [`init_attachments_dir`]).
pub(crate) fn attachments_dir() -> PathBuf {
    ATTACHMENTS_DIR
        .get()
        .cloned()
        .unwrap_or_else(fallback_attachments_dir)
}

/// The calling tenant's blob-backed stash: the workspace's storage backend once
/// [`init_attachments_dir`] ran, else a process-local in-memory one (a CLI or
/// test with no workspace: placeholders resolve for the life of the process).
/// The mechanism (dedup'd writes, cap eviction, TTL sweep, id index) is
/// `tinyagents_harness::multimodal::BlobAttachmentStash`; the cap and TTL are
/// this host's policy (none: references stay resolvable).
fn stash() -> Option<BlobAttachmentStash> {
    match ATTACHMENTS_WORKSPACE.get() {
        Some(workspace) => crate::agent::session_store::attachment_stash_for(workspace),
        None => Some(memory_stash().clone()),
    }
}

fn memory_stash() -> &'static BlobAttachmentStash {
    static MEMORY: OnceLock<BlobAttachmentStash> = OnceLock::new();
    MEMORY.get_or_init(|| {
        use tinystoragedrivers::StorageBackend as _;
        let scoped = tinystoragedrivers::MemoryStorage::new()
            .for_scope(&tinystoragedrivers::Scope::local())
            .expect("the memory backend binds the local scope");
        BlobAttachmentStash::new(
            std::sync::Arc::clone(scoped.blobs()),
            std::sync::Arc::clone(scoped.documents()),
            u64::MAX,
            Duration::MAX,
        )
    })
}

/// Per-user fallback attachments dir used only when [`init_attachments_dir`]
/// was never called. Uses the OS user cache dir (e.g. `~/Library/Caches/…`,
/// `~/.cache/…`) so legacy sidecars aren't looked for in a world-readable
/// shared `temp_dir()` on multi-user hosts. Only falls back to `temp_dir()`
/// when no user cache dir can be resolved at all.
fn fallback_attachments_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|d| d.cache_dir().join("openhuman-attachments"))
        .unwrap_or_else(|| std::env::temp_dir().join("openhuman-attachments"))
}

/// `id -> marker target` for the placeholders in `messages`: the blob stash's
/// bytes as a `data:` URI, else (for an id the stash lacks) a legacy sidecar
/// file in the attachments directory, as its path. The marker target is carried
/// as a `PathBuf` only because that is the harness's index type; it is rendered
/// verbatim inside `[IMAGE:…]`.
async fn build_attachment_index(messages: &[TranscriptMessage]) -> HashMap<String, PathBuf> {
    let mut wanted = Vec::new();
    for row in messages {
        wanted.extend(placeholder_ids(&row.content));
        for part in row.parts.iter().flatten() {
            if let TranscriptPart::Text { text } = part {
                wanted.extend(placeholder_ids(text));
            }
        }
    }
    wanted.sort();
    wanted.dedup();
    let mut index = HashMap::new();
    if wanted.is_empty() {
        return index;
    }
    if let Some(stash) = stash() {
        for id in &wanted {
            match stash.read_data_uri(id).await {
                Ok(Some(data_uri)) => {
                    index.insert(id.clone(), PathBuf::from(data_uri));
                }
                Ok(None) => {}
                Err(err) => tracing::warn!(
                    target: "multimodal",
                    id = %id,
                    reason = %err,
                    "[multimodal::images][stash] could not read attachment; trying legacy sidecars"
                ),
            }
        }
    }
    if index.len() < wanted.len() {
        for (id, path) in legacy_attachment_index() {
            if wanted.contains(&id) {
                index.entry(id).or_insert(path);
            }
        }
    }
    index
}

/// The `#att:<id>` ids of the sidecar placeholders in `text`.
fn placeholder_ids(text: &str) -> Vec<String> {
    markers::extract_image_placeholders_in_text(text)
        .iter()
        .filter_map(|placeholder| {
            let after = placeholder.split(markers::IMAGE_STASH_REF).nth(1)?;
            let id = after.trim_end_matches(']').trim();
            (!id.is_empty()).then(|| id.to_string())
        })
        .collect()
}

/// `id -> path` of the legacy sidecar files directly under the attachments
/// directory (written before the blob stash). Symlinks and nested entries are
/// ignored.
fn legacy_attachment_index() -> HashMap<String, PathBuf> {
    legacy_attachment_index_in(&attachments_dir())
}

fn legacy_attachment_index_in(dir: &std::path::Path) -> HashMap<String, PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .filter_map(|entry| {
            let path = entry.path();
            let stem = path.file_stem()?.to_str()?.to_string();
            let valid = !stem.is_empty()
                && stem
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_'));
            valid.then_some((stem, path))
        })
        .collect()
}

/// Compatibility no-op: durable conversation attachments are not swept.
pub async fn sweep_stale_attachments() {
    // Retained compatibility API. Conversation references do not expire.
}

#[cfg(test)]
#[path = "multimodal_tests.rs"]
mod tests;
