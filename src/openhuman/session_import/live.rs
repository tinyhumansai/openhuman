//! Live dual-write of new session turns into the TinyAgents store.
//!
//! Additive, best-effort, and gated by the `AgentConfig::session_dual_write`
//! **config flag** which **defaults ON** ([`dual_write_enabled`]); the
//! `OPENHUMAN_SESSION_DUAL_WRITE` env var is a **kill switch** — set it to a
//! falsey value (`0`/`false`/`no`/`off`/`disable`) to force the mirror off
//! regardless of config. This mirrors the `OPENHUMAN_APPROVAL_GATE`
//! default-on-with-kill-switch idiom. The legacy `session_raw/*.jsonl`
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
use std::sync::Arc;

use anyhow::{Context, Result};
use tinyagents::harness::store::{AppendStore, Store};

use crate::openhuman::agent::harness::session::transcript::SessionTranscript;

use super::convert::{
    build_descriptor, effective_thread_id, journal_messages, sanitize_store_name, stream_name,
};
use super::ops::{open_session_stores, SessionStores};
use super::types::{DescriptorSource, JournalMessage, NS_SESSIONS};

/// Kill-switch env var for the live session-store dual-write. The config flag
/// (`AgentConfig::session_dual_write`) defaults ON; setting this env var to a
/// falsey value forces the mirror OFF regardless of config. See
/// [`dual_write_enabled`].
const DUAL_WRITE_ENV: &str = "OPENHUMAN_SESSION_DUAL_WRITE";

/// Kill-switch env var for the store-backed session shadow read. The config
/// flag (`AgentConfig::session_shadow_reads`) defaults OFF; setting this env
/// var to a falsey value forces the shadow read OFF even when the flag is ON.
/// It can never force the shadow read ON. See [`shadow_reads_enabled`].
const SHADOW_READ_ENV: &str = "OPENHUMAN_SESSION_SHADOW_READS";

/// Whether `var` is set to a case-insensitive falsey value
/// (`0`/`false`/`no`/`off`/`disable`/`disabled`). Unset — or any non-falsey
/// value — is not a kill. Read live (not cached) so a config reload / env
/// change is honored on the next turn/read.
fn env_kill_switch_engaged(var: &str) -> bool {
    match std::env::var(var) {
        Ok(v) => matches!(
            v.trim().to_ascii_lowercase().as_str(),
            "0" | "false" | "no" | "off" | "disable" | "disabled"
        ),
        Err(_) => false,
    }
}

/// Whether the `OPENHUMAN_SESSION_DUAL_WRITE` kill switch is engaged (set to a
/// falsey value). Unset — or any non-falsey value — leaves the mirror driven by
/// the config flag. Read live (not cached) so a config reload / env change is
/// honored on the next turn.
fn kill_switch_engaged() -> bool {
    env_kill_switch_engaged(DUAL_WRITE_ENV)
}

/// Store-registry name under which the session KV store is registered on each
/// turn's `RunContext.stores` (issue #4249, 04.1). Slash-free so it round-trips
/// the crate `FileStore` name sanitizer. This is a forward-looking,
/// harness-visible handle to the same `tinyagents_store` KV tree the live
/// dual-write mirrors into; readers stay legacy until 04.2.
pub const TINYAGENTS_SESSION_KV_STORE: &str = "openhuman_sessions";

/// Whether the live session-store dual-write is enabled for this turn.
///
/// `config_enabled` is the `AgentConfig::session_dual_write` flag, which
/// **defaults ON**. The `OPENHUMAN_SESSION_DUAL_WRITE` env var is a pure kill
/// switch: an explicit falsey value (case-insensitive
/// `0`/`false`/`no`/`off`/`disable`/`disabled`) forces the mirror OFF regardless
/// of config; otherwise the config flag wins. Read live (never cached) so a
/// config reload / env change is honored on the next turn. This keeps a clean
/// 04.2 seam (reads can flip independently) while making the mirror the default
/// so new turns land in the store without opt-in.
pub fn dual_write_enabled(config_enabled: bool) -> bool {
    let killed = kill_switch_engaged();
    let enabled = config_enabled && !killed;
    log::debug!(
        "[session-store] dual-write decision config_enabled={config_enabled} kill_switch={killed} enabled={enabled}"
    );
    enabled
}

/// Open the session KV store as an `Arc<dyn Store>` for registration on the
/// per-turn `RunContext.stores` under [`TINYAGENTS_SESSION_KV_STORE`], honoring
/// the dual-write flag (config default ON + env kill switch).
///
/// Best-effort: `None` when the dual-write is disabled **or** the config (hence
/// workspace) cannot be resolved. When present it is the exact same
/// `{workspace}/tinyagents_store/kv` `FileStore` the importer and the live
/// dual-write use, so a harness-side reader (04.2+) sees identical records. The
/// journal (`JsonlAppendStore`, an `AppendStore` rather than a `Store`) is not
/// registrable on the `StoreRegistry`; the dual-write opens it directly.
pub async fn session_kv_store() -> Option<Arc<dyn Store>> {
    let cfg = match crate::openhuman::config::Config::load_or_init().await {
        Ok(cfg) => cfg,
        Err(err) => {
            log::warn!("[session-store] cannot resolve config for store registration: {err:#}");
            return None;
        }
    };
    if !dual_write_enabled(cfg.agent.session_dual_write) {
        log::debug!(
            "[session-store] dual-write disabled; skipping RunContext session-store registration"
        );
        return None;
    }
    let workspace = cfg.workspace_dir;
    let SessionStores { kv, .. } = open_session_stores(&workspace);
    log::debug!(
        "[session-store] opened session kv store for RunContext.stores workspace={}",
        workspace.display()
    );
    Some(Arc::new(kv))
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

// ─────────────────────────────────────────────────────────────────────────────
// Store-backed SHADOW READ (issue #4249, sessions 04.2 phase 2)
//
// Beside the legacy authoritative transcript reader
// (`session/turn/session_io.rs` → `try_load_session_transcript`), read the
// same session's messages back from the crate journal store, normalize both
// sides through the same `convert` machinery the dual-write uses, compare, and
// log divergence. Legacy stays authoritative: this observes + logs only and
// never affects, fails, or slows the authoritative read.
// ─────────────────────────────────────────────────────────────────────────────

/// Whether the store-backed session **shadow read** is enabled for this read.
///
/// `config_enabled` is the `AgentConfig::session_shadow_reads` flag, which
/// **defaults OFF** (unlike `session_dual_write`). The
/// `OPENHUMAN_SESSION_SHADOW_READS` env var is a pure kill switch: an explicit
/// falsey value (case-insensitive `0`/`false`/`no`/`off`/`disable`/`disabled`)
/// forces the shadow read OFF regardless of config; it can never force it ON.
/// Read live (never cached) so a config reload / env change is honored on the
/// next read. Mirrors the [`dual_write_enabled`] flag/env idiom exactly, only
/// with the default flipped and no default-on behavior.
pub fn shadow_reads_enabled(config_enabled: bool) -> bool {
    let killed = env_kill_switch_engaged(SHADOW_READ_ENV);
    let enabled = config_enabled && !killed;
    log::debug!(
        "[session_shadow_read] decision config_enabled={config_enabled} kill_switch={killed} enabled={enabled}"
    );
    enabled
}

/// Outcome of one shadow-read comparison. Returned for tests/observability;
/// the compact divergence summary is also logged (`[session_shadow_read]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ShadowReadOutcome {
    /// The store stream rendered exactly the legacy transcript's messages.
    Match { messages: usize },
    /// The store stream diverged from the legacy render. Carries only compact
    /// counts + the first differing index — never message bodies (PII).
    Divergence {
        legacy: usize,
        shadow: usize,
        first_diff: Option<usize>,
    },
    /// No shadow available: the store read errored, or the stream is
    /// empty/absent for a non-empty legacy transcript (e.g. dual-write was off
    /// when this session was written). Treated as non-divergent — the legacy
    /// read is authoritative regardless.
    Unavailable,
}

/// Read a session's messages back from the crate journal store
/// (`{workspace}/tinyagents_store/journal`, stream `session.{stem}.messages`)
/// as normalized [`JournalMessage`]s — the same shape the importer and live
/// dual-write write. A missing stream yields an empty vec (not an error).
async fn read_shadow_messages(workspace: &Path, session_key: &str) -> Result<Vec<JournalMessage>> {
    let SessionStores { journal, .. } = open_session_stores(workspace);
    let stream = stream_name(session_key);
    let records = journal
        .read_from(&stream, 0)
        .await
        .with_context(|| format!("shadow read of stream {stream}"))?;
    let mut out = Vec::with_capacity(records.len());
    for (offset, value) in records {
        let msg: JournalMessage = serde_json::from_value(value)
            .with_context(|| format!("shadow record shape at offset {offset}"))?;
        out.push(msg);
    }
    Ok(out)
}

/// Shadow-read the given session back from the store and compare it against the
/// legacy transcript, logging divergence. Legacy stays authoritative — the
/// caller ignores the returned outcome for control flow (it exists for tests /
/// observability). Best-effort: any store-read error is logged at debug and
/// reported as [`ShadowReadOutcome::Unavailable`]; it never breaks or slows the
/// authoritative read.
///
/// Both sides are normalized through the importer's `convert` machinery
/// ([`journal_messages`]) so live/legacy renders are directly comparable, then
/// compared by message count and normalized content. On mismatch a **compact**
/// summary (counts + first differing index) is warn-logged; message bodies are
/// never emitted (PII).
pub async fn shadow_read_compare(
    workspace: &Path,
    session_key: &str,
    legacy: &SessionTranscript,
) -> ShadowReadOutcome {
    let expected = journal_messages(legacy);
    log::debug!(
        "[session_shadow_read] enter stem={session_key} workspace={} legacy_messages={}",
        workspace.display(),
        expected.len()
    );

    let shadow = match read_shadow_messages(workspace, session_key).await {
        Ok(v) => v,
        Err(err) => {
            log::debug!(
                "[session_shadow_read] store read error stem={session_key}: {err:#} — no shadow available"
            );
            return ShadowReadOutcome::Unavailable;
        }
    };

    // Empty/absent store stream against a non-empty legacy transcript: the
    // session simply was not mirrored (dual-write off when it was written).
    // Treat as "no shadow" rather than a spurious divergence.
    if shadow.is_empty() && !expected.is_empty() {
        log::debug!(
            "[session_shadow_read] no store stream stem={session_key} legacy_messages={} — no shadow available",
            expected.len()
        );
        return ShadowReadOutcome::Unavailable;
    }

    if shadow == expected {
        log::debug!(
            "[session_shadow_read] parity OK stem={session_key} messages={}",
            expected.len()
        );
        return ShadowReadOutcome::Match {
            messages: expected.len(),
        };
    }

    // Divergence: first index where the two normalized renders differ, or the
    // shorter length when one is a strict prefix of the other. Compact only.
    let first_diff = expected
        .iter()
        .zip(shadow.iter())
        .position(|(a, b)| a != b)
        .or_else(|| (expected.len() != shadow.len()).then(|| expected.len().min(shadow.len())));
    log::warn!(
        "[session_shadow_read] DIVERGENCE stem={session_key} legacy_count={} shadow_count={} first_diff={first_diff:?}",
        expected.len(),
        shadow.len()
    );
    ShadowReadOutcome::Divergence {
        legacy: expected.len(),
        shadow: shadow.len(),
        first_diff,
    }
}
