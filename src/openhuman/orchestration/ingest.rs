//! DM ingest: decrypt-once → classify → persist → acknowledge.
//!
//! Driven by the existing `DomainEvent::TinyPlaceStreamMessage` (the tinyplace
//! websocket recv loop), filtered to conversation/DM streams. Never logs message
//! bodies or seeds.

use std::path::Path;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::tinyplace::{acknowledge_message, decrypt_envelope};

use super::store;
use super::types::{ChatKind, OrchestrationMessage, OrchestrationSession, SessionEnvelopeV1};

const LOG: &str = "orchestration";

/// A decrypted DM turned into the fields we persist. Pure result of
/// [`classify_message`] — no IO, so it is unit-testable without a live client.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ClassifiedMessage {
    chat_kind: ChatKind,
    session_id: String,
    role: String,
    source: String,
    label: Option<String>,
    workspace: Option<String>,
    seq: i64,
    body: String,
    timestamp: String,
}

/// True for streams that carry ciphertext DM envelopes worth ingesting.
fn is_dm_stream(kind: &str, stream_id: &str) -> bool {
    kind.eq_ignore_ascii_case("conversation")
        || kind.eq_ignore_ascii_case("dm")
        || stream_id.starts_with("conversation:")
}

/// Classify a decrypted DM: a harness envelope becomes a per-session message,
/// anything else becomes a message in the peer's Master window. Pure.
fn classify_message(plaintext: String, fallback_timestamp: &str) -> ClassifiedMessage {
    match SessionEnvelopeV1::parse(&plaintext) {
        Some(env) => {
            // Compute the session key while `env` is still fully intact (before any
            // field moves below), since `session_key` borrows `&env`.
            let session_id = env.session_key();
            let label = (env.scope.scope_type == "folder").then(|| env.scope.key.clone());
            let workspace = (!env.scope.cwd.is_empty()).then(|| env.scope.cwd.clone());
            let timestamp = if env.message.timestamp.is_empty() {
                fallback_timestamp.to_string()
            } else {
                env.message.timestamp
            };
            ClassifiedMessage {
                chat_kind: ChatKind::Session,
                // Key on the single per-pair session id (the shared `wrapper_session_id`
                // both peers put on every message for a thread), so a reply threads back
                // into the same session. Falls back to `harness_session_id` for a legacy
                // envelope with no per-pair id. See `SessionEnvelopeV1::session_key`.
                session_id,
                role: env.message.role,
                source: env.harness.provider,
                label,
                workspace,
                seq: env.message.line,
                body: env.message.text,
                timestamp,
            }
        }
        None => ClassifiedMessage {
            chat_kind: ChatKind::Master,
            session_id: "master".to_string(),
            role: "user".to_string(),
            source: String::new(),
            label: None,
            workspace: None,
            seq: 0,
            body: plaintext,
            timestamp: fallback_timestamp.to_string(),
        },
    }
}

/// Persist a classified message + its session row. Idempotent by `msg_id`;
/// returns true if a new message row landed. Testable with a tempdir DB.
fn persist_message(
    workspace_dir: &Path,
    msg_id: &str,
    agent_id: &str,
    classified: &ClassifiedMessage,
    now: &str,
) -> Result<bool, String> {
    store::with_connection(workspace_dir, |c| {
        // Wake idempotence keys on a per-session `seq` being monotonic, but the
        // harness `message.line` we classify into `seq` is NOT reliable: a wrapped
        // Claude harness stamps `line = 0` on every DM, and a peer reusing one
        // `wrapper_session_id` across harness sessions can reset it. Under the
        // shared per-pair session key that collapses every message into one
        // session whose `last_seq`/wake cursor then pins at 0, so after the first
        // message the graph is skipped and the DM is silently dropped (#4583).
        //
        // Fix: ignore the wire `line` for ordering and stamp a store-assigned,
        // strictly-increasing per-(agent, session) ingest ordinal. Messages are
        // append-only and deduped-by-id upstream, so `MAX(seq)+1` is monotonic and
        // every genuinely-new DM advances `last_seq` past the cursor → wakes the graph.
        //
        // Allocate the ordinal and write both rows in one IMMEDIATE txn so a
        // concurrent writer on the same session (the drain here vs the graph's
        // `send_dm` reply persist) can't read the same `MAX(seq)` and duplicate it.
        store::in_immediate_txn(c, |c| {
            let ingest_seq = store::next_session_seq(c, agent_id, &classified.session_id)?;
            store::upsert_session(
                c,
                &OrchestrationSession {
                    session_id: classified.session_id.clone(),
                    agent_id: agent_id.to_string(),
                    source: classified.source.clone(),
                    label: classified.label.clone(),
                    workspace: classified.workspace.clone(),
                    last_seq: ingest_seq,
                    created_at: now.to_string(),
                    last_message_at: classified.timestamp.clone(),
                },
            )?;
            store::insert_message(
                c,
                &OrchestrationMessage {
                    id: msg_id.to_string(),
                    agent_id: agent_id.to_string(),
                    session_id: classified.session_id.clone(),
                    chat_kind: classified.chat_kind,
                    role: classified.role.clone(),
                    body: classified.body.clone(),
                    timestamp: classified.timestamp.clone(),
                    seq: ingest_seq,
                },
            )
        })
    })
    .map_err(|e| format!("persist: {e}"))
}

/// Entry point from the bus subscriber. Cheap no-op when orchestration is
/// disabled or the stream is not a DM stream.
pub async fn ingest_stream_message(
    config: &Config,
    kind: &str,
    stream_id: &str,
    raw: &serde_json::Value,
) {
    if !config.orchestration.enabled {
        return;
    }
    if !is_dm_stream(kind, stream_id) {
        return;
    }
    let envelope: tinyplace::types::MessageEnvelope = match serde_json::from_value(raw.clone()) {
        Ok(env) => env,
        Err(e) => {
            log::debug!(target: LOG, "[orchestration] ingest.skip stream={stream_id} not-an-envelope err={e}");
            return;
        }
    };
    if let Err(e) = ingest_one(config, envelope).await {
        log::warn!(target: LOG, "[orchestration] ingest.error stream={stream_id}: {e}");
    }
}

async fn ingest_one(
    config: &Config,
    envelope: tinyplace::types::MessageEnvelope,
) -> Result<(), String> {
    let msg_id = envelope.id.clone();
    let agent_id = envelope.from.clone();
    log::debug!(target: LOG, "[orchestration] ingest.entry id={msg_id} from={agent_id}");
    let workspace_dir = config.workspace_dir.clone();

    // 0. Sender gate: only ingest DMs from linked (accepted) pairing agents —
    //    i.e. wrapped Codex/Claude sessions. Decrypting advances the Signal
    //    ratchet, so an unpaired sender's DM (an ordinary human message) must
    //    never be decrypted or consumed here; it stays readable by the existing
    //    Messaging UI via messages.list / signal.decryptMessage.
    let linked =
        crate::openhuman::agent_orchestration::pairing::linked_agent_ids(&workspace_dir).await;
    if !linked.contains(&agent_id) {
        log::debug!(target: LOG, "[orchestration] ingest.skip_unpaired from={agent_id}");
        return Ok(());
    }

    // 1. Dedupe BEFORE decrypt — protects the non-idempotent Signal ratchet.
    let already = store::with_connection(&workspace_dir, |c| store::message_exists(c, &msg_id))
        .map_err(|e| format!("store lookup: {e}"))?;
    if already {
        // The row already exists but a prior run may have crashed (or the relay
        // ack failed) after persist. Retry the ack so the relay copy is
        // consumed; never re-decrypt or re-publish.
        log::debug!(target: LOG, "[orchestration] ingest.dedupe id={msg_id}");
        if let Err(e) = acknowledge_message(&msg_id).await {
            log::warn!(target: LOG, "[orchestration] ingest.ack_failed_dedupe id={msg_id}: {e}");
        }
        return Ok(());
    }

    // 2. Decrypt exactly once, then classify + persist.
    let plaintext = decrypt_envelope(&envelope).await?;
    let classified = classify_message(plaintext, &envelope.timestamp);
    let now = chrono::Utc::now().to_rfc3339();
    let landed = persist_message(&workspace_dir, &msg_id, &agent_id, &classified, &now)?;

    // 3. Acknowledge (consume once) + fan out for stages 4/7.
    if landed {
        if let Err(e) = acknowledge_message(&msg_id).await {
            log::warn!(target: LOG, "[orchestration] ingest.ack_failed id={msg_id}: {e}");
        }
        publish_global(DomainEvent::OrchestrationSessionMessage {
            agent_id,
            session_id: classified.session_id,
            chat_kind: classified.chat_kind.as_str().to_string(),
        });
    }
    log::debug!(target: LOG, "[orchestration] ingest.exit id={msg_id} landed={landed}");
    Ok(())
}

/// Poll the relay mailbox once and run every delivered envelope through the
/// ingest pipeline.
///
/// The relay delivers DMs to `/messages` (poll-only) and — unlike inbox items —
/// never publishes them to the `/inbox/stream` WebSocket, so a poller is the
/// only way orchestration learns about inbound DMs. Envelopes from senders that
/// are not orchestration-linked are skipped by [`ingest_one`] WITHOUT being
/// decrypted or acknowledged, so they stay in the mailbox for the Messaging UI.
///
/// Returns the number of envelopes examined this pass. Best-effort per envelope:
/// a decrypt/persist failure on one is logged and does not abort the batch.
pub async fn drain_mailbox_once(config: &Config) -> Result<usize, String> {
    if !config.orchestration.enabled {
        return Ok(0);
    }
    let client = crate::openhuman::tinyplace::ops::global_state()
        .client()
        .await?;
    let signer = client
        .http()
        .signer()
        .ok_or_else(|| "no signer configured".to_string())?;
    let agent_id = signer.agent_id();
    let resp = client
        .messages
        .list(&agent_id, Some(100))
        .await
        .map_err(|e| format!("messages.list: {e}"))?;
    let count = resp.messages.len();
    if count > 0 {
        log::debug!(target: LOG, "[orchestration] drain.fetched count={count}");
    }
    for envelope in resp.messages {
        if let Err(e) = ingest_one(config, envelope).await {
            log::warn!(target: LOG, "[orchestration] drain.ingest_error: {e}");
        }
    }
    Ok(count)
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENVELOPE: &str = r#"{
        "envelope_version": "tinyplace.harness.session.v1",
        "version": 1,
        "scope": { "type": "folder", "key": "my-repo", "cwd": "/w",
                   "wrapper_session_id": "w1", "harness_session_id": "h1" },
        "harness": { "provider": "codex", "command": "codex", "argv": [] },
        "message": { "id": "m1", "line": 7, "role": "user", "text": "hello",
                     "timestamp": "2026-07-02T01:00:00Z" },
        "source": { "path": "p", "record_type": "user" }
    }"#;

    #[test]
    fn dm_stream_filter() {
        assert!(is_dm_stream("conversation", "conversation:abc"));
        assert!(is_dm_stream("DM", "x"));
        assert!(is_dm_stream("other", "conversation:abc"));
        assert!(!is_dm_stream("inbox", "inbox"));
    }

    #[tokio::test]
    async fn drain_is_a_noop_when_orchestration_disabled() {
        // Guard short-circuits before touching the tiny.place client, so this
        // exercises the early return without any wallet/network.
        let mut config = Config::default();
        config.orchestration.enabled = false;
        assert_eq!(drain_mailbox_once(&config).await, Ok(0));
    }

    #[test]
    fn classifies_harness_envelope_as_session() {
        let c = classify_message(ENVELOPE.to_string(), "2026-07-02T09:00:00Z");
        assert_eq!(c.chat_kind, ChatKind::Session);
        assert_eq!(c.session_id, "w1"); // keyed on the shared per-pair wrapper_session_id
        assert_eq!(c.role, "user");
        assert_eq!(c.source, "codex");
        assert_eq!(c.label.as_deref(), Some("my-repo")); // folder scope → label
        assert_eq!(c.workspace.as_deref(), Some("/w"));
        assert_eq!(c.seq, 7);
        assert_eq!(c.body, "hello");
        assert_eq!(c.timestamp, "2026-07-02T01:00:00Z"); // envelope ts wins
    }

    #[test]
    fn classifies_plain_dm_as_master_with_fallback_timestamp() {
        let c = classify_message("just chatting".to_string(), "2026-07-02T09:00:00Z");
        assert_eq!(c.chat_kind, ChatKind::Master);
        assert_eq!(c.session_id, "master");
        assert_eq!(c.role, "user");
        assert!(c.label.is_none());
        assert_eq!(c.seq, 0);
        assert_eq!(c.body, "just chatting");
        assert_eq!(c.timestamp, "2026-07-02T09:00:00Z"); // fallback used
    }

    #[test]
    fn persist_message_is_idempotent_and_buckets_by_session() {
        let tmp = tempfile::tempdir().unwrap();
        let session = classify_message(ENVELOPE.to_string(), "2026-07-02T09:00:00Z");
        let master = classify_message("hi".to_string(), "2026-07-02T09:00:00Z");

        assert!(persist_message(tmp.path(), "m1", "@peer", &session, "now").unwrap());
        // Replay of the same relay id does not double-insert.
        assert!(!persist_message(tmp.path(), "m1", "@peer", &session, "now").unwrap());
        assert!(persist_message(tmp.path(), "m2", "@peer", &master, "now").unwrap());

        store::with_connection(tmp.path(), |c| {
            assert_eq!(store::count_messages(c, "@peer", "w1")?, 1); // per-pair wrapper id
            assert_eq!(store::count_messages(c, "@peer", "master")?, 1);
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn persist_stamps_monotonic_ingest_seq_so_line_zero_dms_still_wake() {
        // Regression for the silent drop (#4583). A wrapped Claude harness stamps
        // `line = 0` on EVERY DM, so pre-fix both messages classified to seq 0;
        // under the shared wrapper-session key the wake cursor pinned at 0 and the
        // second message was persisted + acked but never woke the graph (no reply).
        // Persist must ignore the wire `line` and stamp a strictly-increasing
        // per-(agent, session) ingest ordinal so `last_seq` advances past the cursor.
        let tmp = tempfile::tempdir().unwrap();
        let line_zero = || {
            classify_message(
                ENVELOPE.replace("\"line\": 7", "\"line\": 0"),
                "2026-07-02T09:00:00Z",
            )
        };
        let first = line_zero();
        let second = line_zero();
        assert_eq!(first.seq, 0); // wire line is 0 for both …
        assert_eq!(second.seq, 0);

        assert!(persist_message(tmp.path(), "mA", "@peer", &first, "now").unwrap());
        assert!(persist_message(tmp.path(), "mB", "@peer", &second, "now").unwrap());

        store::with_connection(tmp.path(), |c| {
            // … but the persisted seqs are monotonic ingest ordinals 1 and 2, and
            // last_seq advanced to 2 — so a wake cursor left at 1 sees new work.
            assert_eq!(store::count_messages(c, "@peer", "w1")?, 2);
            assert_eq!(store::session_last_seq(c, "@peer", "w1")?, Some(2));
            let seqs: Vec<i64> = store::list_recent_messages(c, "@peer", "w1", 10)?
                .iter()
                .map(|m| m.seq)
                .collect();
            assert_eq!(seqs, vec![1, 2]);
            Ok(())
        })
        .unwrap();
    }
}
