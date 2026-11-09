//! Durable audio for in-app voice notes.
//!
//! A voice note is an audio blob the thread row points at. The user's
//! recording and the assistant's spoken reply are both stored in the acting
//! scope's storage-port `BlobStore` under `voice_notes/<note_id>.<ext>`; the
//! row only carries
//! `{ noteId, mime }` in its metadata, so a reload (or a second window) can
//! fetch the bytes again through `channel.web_voice_note_read`.
//!
//! Assistant replies use [`reply_note_id`], derived from the turn's
//! `request_id`, which is what lets reply persistence find the audio the turn
//! synthesized without threading it through the turn result.

use std::path::Path;
use std::sync::Arc;

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tinystoragedrivers::{CollectionSpec, Precondition};
use tinystoragedrivers_api::BlobStore;

use crate::storage::{self, documents::Repo};

const LOG_PREFIX: &str = "[web_channel][voice_note]";
/// `source` the composer stamps on a turn that began as a voice note.
pub(crate) const VOICE_NOTE_SOURCE: &str = "voice_note";
const DIR: &str = "voice_notes";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ReplyVisemeTimeline {
    tenant: crate::core::runtime::Tenant,
    frames: Vec<Value>,
}

/// Backend and acting scope for this call's tenant.
fn backend_and_scope(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
) -> Result<(Arc<dyn storage::StorageBackend>, storage::Scope), String> {
    let tenant = crate::core::runtime::tenant::tenant_in(
        crate::core::runtime::is_saas(),
        ctx.resolved().as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let backend = storage::local::backend_for(workspace)
        .map_err(|error| format!("open voice-note storage: {error}"))?;
    let scope = storage::scope_from(
        tenant.profile.as_deref(),
        tenant.agent.as_deref(),
        crate::core::runtime::mode::is_saas(),
    )
    .map_err(|error| format!("scope voice-note storage: {error}"))?;
    Ok((backend, scope))
}

fn viseme_repo(ctx: &crate::core::runtime::DispatchCtx, workspace: &Path) -> Result<Repo, String> {
    let (backend, scope) = backend_and_scope(ctx, workspace)?;
    Repo::on(&backend, &scope, VISEME_DOMAIN, || {
        vec![CollectionSpec::new(VISEME_COLLECTION)]
    })
    .map_err(|error| format!("open voice-note timeline collection: {error}"))
}

/// The acting scope's blob store, where the note audio lives.
fn blob_store(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
) -> Result<Arc<dyn BlobStore>, String> {
    let (backend, scope) = backend_and_scope(ctx, workspace)?;
    let scoped = backend
        .for_scope(&scope)
        .map_err(|error| format!("open voice-note blob scope: {error}"))?;
    Ok(Arc::clone(scoped.blobs()))
}

fn blob_key(id: &str, ext: &str) -> String {
    format!("{DIR}/{id}.{ext}")
}

/// Runs a blob call on the storage bridge.
fn blob_call<T, F, Fut>(blobs: &Arc<dyn BlobStore>, call: F) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce(Arc<dyn BlobStore>) -> Fut,
    Fut: std::future::Future<Output = Result<T, storage::StorageError>> + Send + 'static,
{
    storage::block_on(call(Arc::clone(blobs))).map_err(|error| error.to_string())
}

const VISEME_DOMAIN: &str = "web_chat::voice_notes";
const VISEME_COLLECTION: &str = crate::threads::store::REPLY_VISEME_TIMELINES;
/// Largest note the store accepts (decoded bytes).
pub(crate) const MAX_NOTE_BYTES: usize = 25 * 1024 * 1024;

/// `(extension, mime)` pairs the store writes and serves.
const KNOWN: &[(&str, &str)] = &[
    ("webm", "audio/webm"),
    ("ogg", "audio/ogg"),
    ("m4a", "audio/mp4"),
    ("mp3", "audio/mpeg"),
    ("wav", "audio/wav"),
];

/// Id of the audio a turn's spoken reply is stored under.
pub(crate) fn reply_note_id(request_id: &str) -> String {
    let safe: String = request_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("reply-{safe}")
}

/// Ids become file names, so only a conservative alphabet is accepted.
pub(crate) fn is_valid_note_id(id: &str) -> bool {
    !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

fn extension_for_mime(mime: &str) -> &'static str {
    let base = mime
        .split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase();
    match base.as_str() {
        "audio/ogg" => "ogg",
        "audio/mp4" | "audio/x-m4a" | "audio/aac" => "m4a",
        "audio/mpeg" | "audio/mp3" => "mp3",
        "audio/wav" | "audio/x-wav" | "audio/wave" => "wav",
        _ => "webm",
    }
}

/// Pre-blob-store location of notes written as files; read-only fallback.
fn legacy_dir(workspace: &Path) -> std::path::PathBuf {
    workspace.join(DIR)
}

/// Stores `bytes` as note `id`, replacing any earlier blob for that id.
/// Returns the stored mime type.
pub(crate) fn store_note(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    id: &str,
    mime: &str,
    bytes: &[u8],
) -> Result<&'static str, String> {
    if !is_valid_note_id(id) {
        return Err("invalid voice note id".to_string());
    }
    if bytes.is_empty() {
        return Err("voice note audio is empty".to_string());
    }
    if bytes.len() > MAX_NOTE_BYTES {
        return Err(format!("voice note exceeds {MAX_NOTE_BYTES} bytes"));
    }
    let ext = extension_for_mime(mime);
    let stored_mime = KNOWN
        .iter()
        .find(|(e, _)| *e == ext)
        .map(|(_, m)| *m)
        .unwrap_or("audio/webm");
    let blobs = blob_store(ctx, workspace)?;
    let others: Vec<String> = KNOWN
        .iter()
        .filter(|(e, _)| *e != ext)
        .map(|(e, _)| blob_key(id, e))
        .collect();
    let key = blob_key(id, ext);
    let body = bytes.to_vec();
    blob_call(&blobs, move |blobs| async move {
        for other in others {
            blobs.delete(&other).await?;
        }
        blobs.put(&key, body, Some(stored_mime)).await?;
        Ok(())
    })
    .map_err(|e| format!("write voice note: {e}"))?;
    log::debug!(
        "{LOG_PREFIX} stored id={id} ext={ext} bytes={}",
        bytes.len()
    );
    Ok(stored_mime)
}

/// Reads note `id`; `Ok(None)` when none is stored. Notes written as files
/// before the blob store existed are still served.
pub(crate) fn read_note(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    id: &str,
) -> Result<Option<(Vec<u8>, &'static str)>, String> {
    if !is_valid_note_id(id) {
        return Err("invalid voice note id".to_string());
    }
    let blobs = blob_store(ctx, workspace)?;
    let keys: Vec<(String, &'static str)> = KNOWN
        .iter()
        .map(|(ext, mime)| (blob_key(id, ext), *mime))
        .collect();
    let found = blob_call(&blobs, move |blobs| async move {
        for (key, mime) in keys {
            if let Some(blob) = blobs.get(&key).await? {
                return Ok(Some((blob.bytes, mime)));
            }
        }
        Ok(None)
    })
    .map_err(|e| format!("read voice note: {e}"))?;
    if found.is_some() {
        return Ok(found);
    }
    let root = legacy_dir(workspace);
    for (ext, mime) in KNOWN {
        let path = root.join(format!("{id}.{ext}"));
        if path.is_file() {
            let bytes = std::fs::read(&path).map_err(|e| format!("read voice note: {e}"))?;
            return Ok(Some((bytes, mime)));
        }
    }
    Ok(None)
}

/// Whether any audio is stored for `id` (blob store, else legacy file).
fn note_mime(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    id: &str,
) -> Option<&'static str> {
    let blobs = blob_store(ctx, workspace).ok()?;
    let keys: Vec<(String, &'static str)> = KNOWN
        .iter()
        .map(|(ext, mime)| (blob_key(id, ext), *mime))
        .collect();
    let found = blob_call(&blobs, move |blobs| async move {
        for (key, mime) in keys {
            if blobs.head(&key).await?.is_some() {
                return Ok(Some(mime));
            }
        }
        Ok(None)
    })
    .ok()
    .flatten();
    found.or_else(|| {
        let root = legacy_dir(workspace);
        KNOWN
            .iter()
            .find(|(ext, _)| root.join(format!("{id}.{ext}")).is_file())
            .map(|(_, mime)| *mime)
    })
}

/// Ensure a typed chat attachment names and carries the already stored note.
/// The RPC upload happens before chat submission; this comparison binds the
/// following typed part to those durable bytes instead of resending audio or
/// treating the opaque reference as model-visible attachment text.
pub(crate) fn validate_typed_media(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    note_id: &str,
    media: &tinychannels_bus::MediaReference,
) -> Result<(), String> {
    if !is_valid_note_id(note_id) || media.opaque_id.as_deref() != Some(note_id) {
        return Err("voice note media must name its stored note id".into());
    }
    if media.url.is_some() || media.path.is_some() || media.local_cache_path.is_some() {
        return Err("typed voice note must use its opaque stored-media reference".into());
    }
    if media.kind != tinychannels_bus::MediaKind::Audio {
        return Err("typed voice note reference must have audio kind".into());
    }
    let (_, stored_mime) = read_note(ctx, workspace, note_id)?
        .ok_or_else(|| "typed voice note has no stored audio".to_string())?;
    if media.content_type.as_deref().is_some_and(|content_type| {
        content_type
            .split(';')
            .next()
            .unwrap_or(content_type)
            .trim()
            != stored_mime
    }) {
        return Err("typed voice note mime does not match the stored audio".into());
    }
    Ok(())
}

/// The `voiceNote` metadata for a reply whose audio was stored, if any.
pub(crate) fn persist_reply_visemes(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    request_id: &str,
    frames: &[impl serde::Serialize],
) -> Result<(), String> {
    let id = reply_note_id(request_id);
    if !is_valid_note_id(&id) {
        return Err("invalid voice-note request id".into());
    }
    let tenant = crate::core::runtime::tenant::tenant_in(
        crate::core::runtime::is_saas(),
        ctx.resolved().as_deref(),
    )
    .map_err(|error| error.to_string())?;
    let frames = serde_json::to_value(frames)
        .map_err(|error| format!("serialize voice-note timeline: {error}"))?
        .as_array()
        .cloned()
        .unwrap_or_default();
    let repo = viseme_repo(ctx, workspace)?;
    let body = serde_json::to_value(ReplyVisemeTimeline { tenant, frames })
        .map_err(|error| format!("serialize voice-note timeline record: {error}"))?;
    repo.run(|docs| async move {
        docs.put(VISEME_COLLECTION, &id, body, Precondition::None)
            .await?;
        Ok(())
    })
    .map_err(|error| format!("persist voice-note timeline: {error}"))
}

/// Voice-note timelines use scoped documents; the file read is compatibility
/// for replies written before the timelines moved to the storage port.
pub(crate) fn reply_note_metadata(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    request_id: &str,
) -> Option<Value> {
    let id = reply_note_id(request_id);
    let mime = note_mime(ctx, workspace, &id)?;
    let mut metadata = json!({ "noteId": id.clone(), "mime": mime });
    let timeline = viseme_repo(ctx, workspace).ok().and_then(|repo| {
        repo.run(|docs| {
            let id = id.clone();
            async move { docs.get(VISEME_COLLECTION, &id).await }
        })
        .ok()
        .flatten()
    });
    let frames = match timeline {
        Some(record) => serde_json::from_value::<ReplyVisemeTimeline>(record.doc)
            .ok()
            .filter(|record| {
                crate::core::runtime::tenant::tenant_in(
                    crate::core::runtime::is_saas(),
                    ctx.resolved().as_deref(),
                )
                .is_ok_and(|tenant| tenant == record.tenant)
            })
            .map(|record| record.frames),
        None => {
            let bytes =
                std::fs::read(legacy_dir(workspace).join(format!("{id}.visemes.json"))).ok();
            bytes.and_then(|bytes| serde_json::from_slice::<Vec<Value>>(&bytes).ok())
        }
    };
    if let Some(frames) = frames.filter(|frames| !frames.is_empty()) {
        metadata["visemes"] = json!(frames);
    }
    Some(metadata)
}

/// `channel.web_voice_note_store`: decode and store a recorded note.
pub(crate) fn store_from_rpc(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    note_id: &str,
    mime: &str,
    audio_base64: &str,
) -> Result<crate::core::Outcome<Value>, String> {
    use base64::Engine as _;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(audio_base64.trim())
        .map_err(|e| format!("audio_base64 is not valid base64: {e}"))?;
    let stored = store_note(ctx, workspace, note_id, mime, &bytes)?;
    Ok(crate::core::Outcome::single_log(
        json!({ "note_id": note_id, "mime": stored }),
        "voice note stored",
    ))
}

/// `channel.web_voice_note_read`: the note's audio, or null audio when absent.
pub(crate) fn read_for_rpc(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    note_id: &str,
) -> Result<crate::core::Outcome<Value>, String> {
    use base64::Engine as _;
    let value = match read_note(ctx, workspace, note_id)? {
        Some((bytes, mime)) => json!({
            "note_id": note_id,
            "mime": mime,
            "audio_base64": base64::engine::general_purpose::STANDARD.encode(bytes),
        }),
        None => json!({ "note_id": note_id, "mime": null, "audio_base64": null }),
    };
    Ok(crate::core::Outcome::single_log(value, "voice note read"))
}

/// Whether this turn's reply is also spoken: an explicit `speak_reply` always
/// wins, otherwise the `[channel_voice]` policy for the `web` channel decides
/// (`always`, or `mirror` when the user sent a voice note).
pub(crate) fn should_speak_reply(
    config: &crate::config::Config,
    thread_id: &str,
    speak_reply: Option<bool>,
    source: Option<&str>,
) -> bool {
    use crate::config::schema::VoiceReplyMode;
    if speak_reply == Some(true) {
        return true;
    }
    match config.channel_voice.resolve_mode("web", thread_id) {
        VoiceReplyMode::Always => true,
        VoiceReplyMode::Mirror => source == Some(VOICE_NOTE_SOURCE),
        VoiceReplyMode::Never | VoiceReplyMode::SpeakReply => false,
    }
}

/// Decodes and stores the audio a turn synthesized. `false` when the backend
/// returned no audio (or it could not be stored); the reply then stays text.
pub(crate) fn store_synthesized_reply(
    ctx: &crate::core::runtime::DispatchCtx,
    workspace: &Path,
    request_id: &str,
    audio_base64: &str,
    audio_mime: &str,
) -> bool {
    use base64::Engine as _;
    if audio_base64.is_empty() {
        return false;
    }
    let bytes = match base64::engine::general_purpose::STANDARD.decode(audio_base64) {
        Ok(b) => b,
        Err(err) => {
            log::warn!("{LOG_PREFIX} reply audio is not base64 request_id={request_id}: {err}");
            return false;
        }
    };
    let id = reply_note_id(request_id);
    match store_note(ctx, workspace, &id, audio_mime, &bytes) {
        Ok(_) => true,
        Err(err) => {
            log::warn!("{LOG_PREFIX} could not store reply audio request_id={request_id}: {err}");
            false
        }
    }
}

#[cfg(test)]
#[path = "voice_notes_tests.rs"]
mod tests;
