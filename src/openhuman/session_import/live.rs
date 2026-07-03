//! Live dual-write of new session turns into the TinyAgents store.
//!
//! Additive, best-effort, and behind the `OPENHUMAN_SESSION_DUAL_WRITE`
//! environment flag (default **OFF**). The legacy `session_raw/*.jsonl`
//! transcript (`session/turn/session_io.rs` → `transcript::write_transcript`)
//! stays the primary and authoritative writer; this module mirrors each
//! *already-persisted* turn into the same store layout the Phase-1 importer
//! produces (`{workspace}/tinyagents_store/{kv,journal}`), reusing
//! [`super::convert`] normalization so live and imported records are
//! shape-identical.
//!
//! Reads stay 100% legacy in this slice — 04.2 flips readers independently,
//! gated on the same flag. A store-write failure here must never fail or alter
//! a chat turn: the caller treats every error as non-fatal (log + swallow), and
//! nothing in this module touches the legacy transcript path.

use std::path::Path;
use std::sync::OnceLock;

use anyhow::{Context, Result};
use tinyagents::harness::store::{AppendStore, Store};

use crate::openhuman::agent::harness::session::transcript::SessionTranscript;

use super::convert::{
    build_descriptor, effective_thread_id, journal_messages, sanitize_store_name, stream_name,
};
use super::ops::{open_session_stores, SessionStores};
use super::types::{DescriptorSource, NS_SESSIONS};

/// Environment flag gating the live session-store dual-write. Default OFF.
const DUAL_WRITE_ENV: &str = "OPENHUMAN_SESSION_DUAL_WRITE";

/// Whether the live session-store dual-write is enabled.
///
/// Read **once** from the environment and cached for the process lifetime.
/// Truthy values (case-insensitive): `1`, `true`, `yes`, `on`. Anything else —
/// including an unset variable — is OFF, so default behavior is byte-identical
/// to today (no store handle constructed, no extra writes).
pub fn dual_write_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| {
        let enabled = std::env::var(DUAL_WRITE_ENV)
            .map(|v| {
                matches!(
                    v.trim().to_ascii_lowercase().as_str(),
                    "1" | "true" | "yes" | "on"
                )
            })
            .unwrap_or(false);
        log::debug!("[session-store] dual-write flag {DUAL_WRITE_ENV} resolved to {enabled}");
        enabled
    })
}

/// Mirror one completed turn's transcript into the TinyAgents store.
///
/// Best-effort: the caller must treat any returned error as non-fatal (log and
/// swallow) — it never fails or alters the legacy chat turn.
///
/// Mirrors the importer's **full-rewrite** semantics: the legacy JSONL
/// transcript is rewritten in full on every turn (not appended), and the
/// `JsonlAppendStore` has no truncate, so the journal stream file is dropped and
/// re-appended each turn. This keeps the store stream shape-identical to an
/// import of the final JSONL. The descriptor is upserted in `NS_SESSIONS`
/// exactly as the importer would, reusing [`build_descriptor`].
pub async fn write_live_turn(
    workspace: &Path,
    session_key: &str,
    transcript: &SessionTranscript,
) -> Result<()> {
    log::debug!(
        "[session-store] dual-write enter stem={session_key} workspace={} messages={}",
        workspace.display(),
        transcript.messages.len()
    );

    let SessionStores {
        kv,
        journal,
        journal_root,
    } = open_session_stores(workspace);

    let stream = stream_name(session_key);

    // Full-rewrite parity: drop the stream file, then re-append every message so
    // the journal reflects the current transcript exactly (the importer resets
    // the same way on re-import). Layout: `{journal_root}/{stream}.jsonl`.
    let stream_file = journal_root.join(format!("{stream}.jsonl"));
    if stream_file.exists() {
        std::fs::remove_file(&stream_file)
            .with_context(|| format!("reset journal stream {stream}"))?;
    }

    let records = journal_messages(transcript);
    let message_count = records.len();
    for (idx, record) in records.iter().enumerate() {
        let value = serde_json::to_value(record)
            .with_context(|| format!("serialize live message {idx}"))?;
        journal
            .append(&stream, value)
            .await
            .with_context(|| format!("journal append failed at message {idx}"))?;
    }

    // Descriptor: same projection the importer uses. No run-ledger join here
    // (live turns have no `agent_runs` link yet) and zero warnings; the source
    // pointer records the workspace-relative JSONL twin.
    let (thread_id, synthesized) =
        effective_thread_id(session_key, transcript.meta.thread_id.as_deref());
    let descriptor = build_descriptor(
        session_key,
        transcript,
        thread_id,
        synthesized,
        Vec::new(),
        DescriptorSource {
            jsonl: Some(format!("session_raw/{session_key}.jsonl")),
            md: None,
        },
        chrono::Utc::now().to_rfc3339(),
        0,
    );
    let descriptor_key = sanitize_store_name(session_key);
    let descriptor_value =
        serde_json::to_value(&descriptor).context("serialize live session descriptor")?;
    kv.put(NS_SESSIONS, &descriptor_key, descriptor_value)
        .await
        .context("descriptor write failed")?;

    log::debug!(
        "[session-store] dual-write exit stem={session_key} stream={stream} messages={message_count}"
    );
    Ok(())
}
