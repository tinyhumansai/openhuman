//! First-login migration of local orchestration history to the hosted brain.
//!
//! The hosted brain has no import-without-wake route — every `POST /events` may
//! fire a wake cycle — so a blind bulk replay of the whole local DB would
//! re-answer long-completed threads. Instead this **resumes only pending work**:
//! for each session whose most-recent turn is an unanswered `user` ask, it
//! replays that single turn so the hosted brain picks it up where the retired
//! local brain left off. Everything else stays viewable in the local render
//! cache without being re-processed.
//!
//! Safe to call on every login: idempotent (the backend upserts on
//! `(counterpart, session, seq)` and re-runs are 202 no-ops) and one-shot via a
//! `kv` flag. If a replay fails the flag stays unset so it retries next login.

use crate::openhuman::config::Config;

use super::store;
use super::wire::OrchestrationEventEnvelopeWire;

const LOG: &str = "orchestration";
const MIGRATED_FLAG: &str = "orch:history_migrated";

/// Run the one-shot history migration if it hasn't completed yet.
pub async fn migrate_if_needed(config: &Config) {
    match store::with_connection(&config.workspace_dir, |c| store::kv_get(c, MIGRATED_FLAG)) {
        Ok(Some(_)) => return,
        Ok(None) => {}
        Err(e) => {
            log::warn!(target: LOG, "[orchestration] migrate.flag_read_failed: {e}");
            return;
        }
    }

    match resume_pending(config).await {
        Ok(resumed) => {
            if let Err(e) = store::with_connection(&config.workspace_dir, |c| {
                store::kv_set(c, MIGRATED_FLAG, "1")
            }) {
                // Not fatal — replays are idempotent, so a retry next login is safe.
                log::warn!(target: LOG, "[orchestration] migrate.flag_set_failed: {e}");
            }
            log::info!(target: LOG, "[orchestration] migrate.done resumed={resumed}");
        }
        Err(e) => {
            // Leave the flag unset so it retries next login.
            log::warn!(target: LOG, "[orchestration] migrate.failed (will retry next login): {e}");
        }
    }
}

/// Replay each session's single most-recent turn iff it is a pending `user` ask.
/// Returns the number of turns replayed.
async fn resume_pending(config: &Config) -> Result<usize, String> {
    let sessions = store::with_connection(&config.workspace_dir, store::list_sessions)
        .map_err(|e| format!("list sessions: {e}"))?;

    let mut resumed = 0usize;
    for session in sessions {
        // Agent-scoped read: `list_sessions` rows are keyed by `(agent_id, session_id)`,
        // and a legacy session id can collide across peers, so read the latest turn
        // for *this* session's `agent_id` — reading by `session_id` alone could replay
        // another peer's ask under `session.agent_id` and wake the wrong conversation.
        let latest = store::with_connection(&config.workspace_dir, |c| {
            store::latest_content_message(c, &session.agent_id, &session.session_id)
        })
        .map_err(|e| format!("latest message session={}: {e}", session.session_id))?;

        let Some(msg) = latest else {
            continue;
        };
        // Only a genuinely pending inbound ask is resumed; a thread whose last
        // turn is an assistant/owner/system message is already handled (or not
        // the brain's job) and must not be re-answered. Inbound peer Master DMs
        // are persisted role "peer" (they left-align in the transcript per
        // #4777) but are still a pending ask that must wake the brain on resume,
        // exactly as the live `forward_event` path does — so treat "peer" like
        // "user" here. (The replay envelope re-sanitizes "peer" → "user" in
        // `wire::build`, so the hosted brain sees the same role either way; the
        // distinction is display-only and never reaches inference.)
        if !matches!(msg.role.as_str(), "user" | "peer") || msg.seq <= 0 {
            continue;
        }

        let ts = super::wire::parse_ts_ms(&msg.timestamp).unwrap_or(0);
        let envelope = OrchestrationEventEnvelopeWire::build(
            &session.agent_id,
            &session.session_id,
            msg.seq,
            &msg.role,
            &session.agent_id,
            &msg.body,
            ts,
            msg.event_kind.as_deref().unwrap_or("message"),
        );
        let cycle_id = super::cloud::push_event(config, &envelope)
            .await
            .map_err(|e| format!("replay session={} seq={}: {e}", session.session_id, msg.seq))?;
        // Record the resumed cycle's device-authoritative origin so a Master-chat
        // turn resumed by the migration can still authorize `run_local_agent`
        // (otherwise the gate sees an unknown cycle and denies local execution).
        if let Some(cid) = cycle_id {
            super::exec_gate::record_cycle_origin(
                &cid,
                &envelope.counterpart_agent_id,
                &envelope.session_id,
            );
        }
        resumed += 1;
    }
    Ok(resumed)
}
