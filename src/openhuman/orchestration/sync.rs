//! Hosted read-surface sync + reachability.
//!
//! The device renders the hosted brain's state. This loop pulls the hosted read
//! surface (`GET /orchestration/v1/{sessions,sessions/:id/messages,steering}`)
//! into the local SQLite **render cache**, so the existing read handlers stay
//! synchronous and offline-safe: they read the cache (which is hosted-sourced
//! and kept fresh here + by live effects), and when the hosted brain is
//! unreachable the cache is the fallback and `cloud_reachable` drives the
//! "cloud brain unreachable" notice.
//!
//! Message sync is seq-gated (`?after=<local max>`), so the device's own
//! forwarded turns are never re-inserted — only turns it is missing (e.g.
//! authored on another device) are pulled in.

use std::time::Duration;

use serde::Deserialize;
use serde_json::json;

use crate::openhuman::config::Config;

use super::store;
use super::types::{ChatKind, OrchestrationMessage, OrchestrationSession};

const LOG: &str = "orchestration";

/// `kv` key: `"1"` when the last sync reached the hosted brain, `"0"` when not.
pub const REACHABLE_KEY: &str = "orch:cloud_reachable";
/// `kv` key: cached steering summary JSON (`{ text, maxCycles }`).
pub const STEERING_KEY: &str = "orch:steering";
/// Default cadence of the read-sync loop.
pub const DEFAULT_SYNC_INTERVAL: Duration = Duration::from_secs(20);

/// Max sessions whose messages we page in per sync pass — bounds one sweep.
const MAX_SESSIONS_PER_SYNC: usize = 100;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostedSession {
    session_id: String,
    #[serde(default)]
    counterpart_agent_id: String,
    #[serde(default)]
    last_seq: i64,
    #[serde(default)]
    status: String,
    #[serde(default)]
    last_event_ts: Option<i64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HostedMessage {
    seq: i64,
    #[serde(default)]
    role: String,
    #[serde(default)]
    body: String,
    #[serde(default)]
    kind: String,
    #[serde(default)]
    ts: i64,
    #[serde(default)]
    cycle_id: String,
}

fn chat_kind_for(session_id: &str) -> ChatKind {
    match session_id {
        "master" => ChatKind::Master,
        "subconscious" => ChatKind::Subconscious,
        _ => ChatKind::Session,
    }
}

/// Epoch-millis → RFC3339, falling back to now on a zero/invalid stamp so a row
/// always has a sortable timestamp.
fn ms_to_rfc3339(ms: i64) -> String {
    chrono::DateTime::<chrono::Utc>::from_timestamp_millis(ms)
        .unwrap_or_else(chrono::Utc::now)
        .to_rfc3339()
}

/// One sync pass: pull hosted sessions + missing messages + steering into the
/// render cache and update reachability. Returns whether the hosted brain was
/// reachable (a `GET /sessions` success).
pub async fn sync_reads(config: &Config) -> bool {
    // Resolve the token + backend client once and reuse them for every GET in this
    // pass (sessions + per-session messages + steering), so the profile lookup and
    // the reqwest connection pool aren't rebuilt on each request.
    let pass = match super::cloud::read_pass(config) {
        Ok(p) => p,
        Err(e) => {
            log::debug!(target: LOG, "[orchestration] sync.unreachable: {e}");
            let _ = store::with_connection(&config.workspace_dir, |c| {
                store::kv_set(c, REACHABLE_KEY, "0")
            });
            return false;
        }
    };
    let sessions_raw = match pass.fetch_sessions().await {
        Ok(v) => v,
        Err(e) => {
            log::debug!(target: LOG, "[orchestration] sync.unreachable: {e}");
            let _ = store::with_connection(&config.workspace_dir, |c| {
                store::kv_set(c, REACHABLE_KEY, "0")
            });
            return false;
        }
    };

    // Reachable — mark it before the best-effort detail sync so the offline
    // notice clears even if a per-session page fails.
    let _ = store::with_connection(&config.workspace_dir, |c| {
        store::kv_set(c, REACHABLE_KEY, "1")
    });

    let sessions: Vec<HostedSession> = serde_json::from_value(sessions_raw).unwrap_or_default();
    for session in sessions.into_iter().take(MAX_SESSIONS_PER_SYNC) {
        // INVARIANT: `counterpart_agent_id` is the opaque handle the device itself
        // forwarded as `counterpartAgentId` (base64 `envelope.from`, see
        // `ingest::forward_event`), echoed back verbatim — the hosted brain routes on
        // it without decoding, it is not a credential. It therefore string-equals the
        // `agent_id` under which ingest / `persist_reply` stored the device's own
        // rows, which is what lets `local_max` below skip re-paging the device's
        // forwarded turns. Every device-side session row keys on this same base64 form
        // (ingest, this sync, and the send_dm mirror), so the encodings never diverge
        // locally. If the backend is ever changed to re-encode agent keys (e.g. to the
        // base58 pairing form), resolve this through `ingest::decode_agent_key` before
        // deriving `agent_id`/`local_max`, or the same session would be inserted twice
        // under two encodings and every turn would render twice.
        let agent_id = if session.counterpart_agent_id.is_empty() {
            session.session_id.clone()
        } else {
            session.counterpart_agent_id.clone()
        };
        // `created_at` is a cosmetic "first seen" time that never advances, so a
        // synthesized `now` is harmless there. `last_message_at` feeds
        // `handle_status`'s `MAX(last_message_at)` ingest-staleness check, so it must
        // NOT be synthesized from `now`: a quiet hosted session with no `lastEventTs`
        // would otherwise look fresh on every 20s tick and mask real staleness. Leave
        // it empty so the upsert's `MAX()` preserves any real prior timestamp instead.
        let event_at = session.last_event_ts.map(ms_to_rfc3339);
        let created_at = event_at
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339());
        let last_message_at = event_at.unwrap_or_default();

        // Session metadata (status/last_seq) renders from hosted; COALESCE in
        // `upsert_session` preserves device-local enrichment (label, presence).
        let status_state = (!session.status.is_empty()).then(|| session.status.clone());
        let upsert = store::with_connection(&config.workspace_dir, |conn| {
            store::upsert_session(
                conn,
                &OrchestrationSession {
                    session_id: session.session_id.clone(),
                    agent_id: agent_id.clone(),
                    source: "hosted".to_string(),
                    last_seq: session.last_seq,
                    created_at: created_at.clone(),
                    last_message_at: last_message_at.clone(),
                    status_state: status_state.clone(),
                    ..Default::default()
                },
            )
        });
        if let Err(e) = upsert {
            log::warn!(target: LOG, "[orchestration] sync.session_upsert_failed session={} err={e}", session.session_id);
            continue;
        }

        // Page in only messages the device is missing (seq > local max), so its
        // own forwarded turns are never duplicated.
        let local_max = store::with_connection(&config.workspace_dir, |c| {
            store::next_session_seq(c, &agent_id, &session.session_id)
        })
        .map(|next| next - 1)
        .unwrap_or(0);

        let msgs_raw = match pass
            .fetch_messages(&session.session_id, Some(local_max))
            .await
        {
            Ok(v) => v,
            Err(e) => {
                log::debug!(target: LOG, "[orchestration] sync.messages_failed session={} err={e}", session.session_id);
                continue;
            }
        };
        let msgs: Vec<HostedMessage> = serde_json::from_value(msgs_raw).unwrap_or_default();
        if msgs.is_empty() {
            continue;
        }
        let kind = chat_kind_for(&session.session_id);
        let _ = store::with_connection(&config.workspace_dir, |conn| {
            for m in &msgs {
                store::insert_message(
                    conn,
                    &OrchestrationMessage {
                        id: format!("hosted:{}:{}", session.session_id, m.seq),
                        agent_id: agent_id.clone(),
                        session_id: session.session_id.clone(),
                        chat_kind: kind,
                        role: m.role.clone(),
                        body: m.body.clone(),
                        timestamp: ms_to_rfc3339(m.ts),
                        seq: m.seq,
                        event_kind: (!m.kind.is_empty()).then(|| m.kind.clone()),
                        call_id: (!m.cycle_id.is_empty()).then(|| m.cycle_id.clone()),
                        ..Default::default()
                    },
                )?;
            }
            Ok(())
        });
    }

    // Steering summary for the status surface (best-effort).
    if let Ok(data) = pass.fetch_steering().await {
        let steering_cache = data.get("active").filter(|a| !a.is_null()).map(|active| {
            json!({
                "text": active.get("directive").and_then(|v| v.as_str()).unwrap_or(""),
                "maxCycles": active.get("maxCycles").and_then(|v| v.as_i64()).unwrap_or(0),
            })
        });
        let _ = store::with_connection(&config.workspace_dir, |c| match &steering_cache {
            Some(v) => store::kv_set(c, STEERING_KEY, &v.to_string()),
            None => store::kv_delete(c, STEERING_KEY),
        });
    }

    true
}

/// Run the periodic read-sync loop until the process exits.
pub async fn run_sync_loop(config: Config, interval: Duration) {
    let mut tick = tokio::time::interval(interval);
    loop {
        tick.tick().await;
        if !config.orchestration.enabled {
            continue;
        }
        sync_reads(&config).await;
    }
}

/// Read the cached reachability flag. Absent (never synced) reads as reachable so
/// the UI does not flash an offline notice before the first sync completes.
pub fn cloud_reachable(config: &Config) -> bool {
    store::with_connection(&config.workspace_dir, |c| store::kv_get(c, REACHABLE_KEY))
        .ok()
        .flatten()
        .map(|v| v != "0")
        .unwrap_or(true)
}

/// Read the cached steering summary as `(text, max_cycles)`, if any.
pub fn cached_steering(config: &Config) -> Option<(String, u32)> {
    let raw = store::with_connection(&config.workspace_dir, |c| store::kv_get(c, STEERING_KEY))
        .ok()
        .flatten()?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let text = v
        .get("text")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if text.is_empty() {
        return None;
    }
    let max_cycles = v
        .get("maxCycles")
        .and_then(|x| x.as_i64())
        .unwrap_or(0)
        .max(0) as u32;
    Some((text, max_cycles))
}
