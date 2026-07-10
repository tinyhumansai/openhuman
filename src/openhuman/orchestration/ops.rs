//! Orchestration transport-side helpers (hosted-brain era).
//!
//! The wake/reasoning graph runs server-side now; what remains on the device is
//! the glue the rest of the domain still calls:
//! - [`start_message_drain_supervisor`]: poll the relay mailbox → ingest → forward.
//! - [`session_send_plaintext`]: wrap an outgoing reply for a session/Master window.
//! - [`build_self_identity`]: the device's published identity card.
//! - attention signals: [`command_center_needs_input`], [`gather_unread_signals`],
//!   [`gather_remote_approval_signals`].

use serde::Serialize;
use serde_json::Value;

use crate::openhuman::config::Config;

use super::store;
use super::types::SessionEnvelopeV1;

const LOG: &str = "orchestration";

pub fn start_message_drain_supervisor() {
    tokio::spawn(async {
        // Receiving DMs is impossible unless this agent has published its Signal
        // keys (peers 404 on the prekey bundle otherwise) — the exact blocker
        // that leaves the orchestration receive loop silently dead. Ensure we
        // are discoverable before/while polling. This mirrors the manual
        // Messaging UI actions but runs automatically for any orchestration-
        // enabled instance. Retry each cycle until confirmed (the wallet may not
        // be unlocked at boot), then stop probing.
        let mut discoverable = false;
        loop {
            let config = match Config::load_or_init().await {
                Ok(c) => c,
                Err(e) => {
                    log::debug!(target: LOG, "[orchestration] drain config load: {e}");
                    tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                    continue;
                }
            };
            // Respect the orchestration opt-out. When `[orchestration].enabled` is
            // false we must NOT publish Signal keys (that mutates remote directory
            // state and makes the user discoverable) nor drain the mailbox.
            if !config.orchestration.enabled {
                tokio::time::sleep(std::time::Duration::from_secs(15)).await;
                continue;
            }
            if !discoverable {
                match crate::openhuman::tinyplace::ensure_signal_keys_published().await {
                    Ok(true) => {
                        discoverable = true;
                        log::info!(
                            target: LOG,
                            "[orchestration] discoverable: Signal keys published — peers can reply"
                        );
                    }
                    Ok(false) => log::debug!(
                        target: LOG,
                        "[orchestration] ensure_signal_keys: publish attempted, not yet confirmed — will retry"
                    ),
                    Err(e) => log::debug!(
                        target: LOG,
                        "[orchestration] ensure_signal_keys deferred (wallet locked / no signer?): {e}"
                    ),
                }
            }
            // Auto-accept contact requests from already-linked agents FIRST, so a
            // paired wrapped-agent that re-established contact is unblocked before
            // this same cycle drains its mailbox. Non-linked requesters are left
            // pending for the human (accepting a contact is a trust decision).
            match crate::openhuman::agent_orchestration::pairing::auto_accept_linked_contact_requests(
                &config,
            )
            .await
            {
                Ok(n) if n > 0 => {
                    log::info!(target: LOG, "[orchestration] auto-accepted {n} linked contact request(s)")
                }
                Ok(_) => {}
                Err(e) => log::debug!(target: LOG, "[orchestration] auto-accept error: {e}"),
            }
            match super::ingest::drain_mailbox_once(&config).await {
                Ok(n) if n > 0 => {
                    log::debug!(target: LOG, "[orchestration] drain: examined {n} envelope(s)")
                }
                Ok(_) => {}
                Err(e) => log::debug!(target: LOG, "[orchestration] drain error: {e}"),
            }
            tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        }
    });
}

/// Wire body for an agent reply into `session_id`: a v1 session envelope for a
/// real harness session (so the peer threads its reply under the same id), or
/// the plain body for the pinned Master / subconscious windows.
pub(crate) fn session_send_plaintext(session_id: &str, body: &str) -> anyhow::Result<String> {
    if session_id == "master" || session_id == "subconscious" {
        return Ok(body.to_string());
    }
    let message_id = format!("session-out:{}", uuid::Uuid::new_v4());
    let now = chrono::Utc::now().to_rfc3339();
    serde_json::to_string(&SessionEnvelopeV1::outgoing(
        session_id,
        body,
        &message_id,
        &now,
    ))
    .map_err(|e| anyhow::anyhow!("envelope encode: {e}"))
}

// ── Self-identity composition (orchestration_self_identity read model) ────────

/// One @handle this agent's wallet holds (reverse-resolved from the directory).
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct HandleEntry {
    pub(crate) username: String,
    pub(crate) primary: bool,
}

/// This agent's own tiny.place identity and whether peers can reach it.
///
/// `discoverable` is the bottom line the UI cares about: a peer can DM this
/// agent only if both its directory card AND its Signal encryption key are
/// published. A fresh identity can accept contacts yet still be un-messageable
/// until it registers a @handle (which is what publishes both), so the
/// `SelfIdentityCard` surfaces the gap instead of leaving it a mystery 404.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SelfIdentity {
    pub(crate) agent_id: String,
    pub(crate) handles: Vec<HandleEntry>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) primary_handle: Option<String>,
    pub(crate) card_published: bool,
    pub(crate) key_published: bool,
    pub(crate) discoverable: bool,
}

/// Pure composition of the three tinyplace reads into the renderer shape. Kept
/// here (business logic) so the parsing/discoverability rules are unit-testable
/// without a live tiny.place client; the `schemas` handler supplies the reads.
///
/// `reverse` is the raw `directory_reverse` JSON (`{ identities: [...] }`), or
/// `None` on a reverse miss. Discoverable = card live AND encryption key
/// published + current — either gap leaves the agent un-messageable.
pub(crate) fn build_self_identity(
    agent_id: String,
    key_published: bool,
    reverse: Option<&Value>,
    card_published: bool,
) -> SelfIdentity {
    let mut handles: Vec<HandleEntry> = Vec::new();
    let mut primary_handle: Option<String> = None;
    if let Some(idents) = reverse
        .and_then(|r| r.get("identities"))
        .and_then(Value::as_array)
    {
        for ident in idents {
            let username = ident
                .get("username")
                .and_then(Value::as_str)
                .map(str::trim)
                .filter(|s| !s.is_empty());
            let Some(username) = username else { continue };
            let primary = ident
                .get("primary")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            if primary && primary_handle.is_none() {
                primary_handle = Some(username.to_string());
            }
            handles.push(HandleEntry {
                username: username.to_string(),
                primary,
            });
        }
    }
    // Fall back to the first handle when none is flagged primary.
    if primary_handle.is_none() {
        primary_handle = handles.first().map(|h| h.username.clone());
    }
    SelfIdentity {
        agent_id,
        handles,
        primary_handle,
        card_published,
        key_published,
        discoverable: card_published && key_published,
    }
}

// ── Attention queue aggregation ─────────────────────────────────────────────
//
// The `orchestration_attention` handler in [`super::schemas`] awaits the async
// approval gate itself, then delegates the two synchronous source reads below.
// Both are best-effort: a source failure degrades to an empty bucket (logged)
// so the surviving signals still surface. The neutral-signal → item mapping is
// the pure, unit-tested code in [`super::attention`].

/// Cap on the command-center runs scanned for the `NeedsInput` bucket — the
/// attention zone only needs the currently-blocked runs, not the full ledger.
const ATTENTION_RUN_LIMIT: u32 = 100;

/// Fetch the command-center `NeedsInput` bucket as neutral attention signals.
/// Best-effort — a read error yields an empty vec (logged) so the rest of the
/// attention queue still assembles.
///
/// The ledger query is filtered to `AwaitingUser` runs so [`ATTENTION_RUN_LIMIT`]
/// bounds *blocked* runs only. Fetching a global recent page then filtering (as
/// `list_agent_work` does) would let an older still-blocked run be paged out by
/// newer working/completed runs in a busy workspace, silently dropping it from
/// the attention queue.
pub(super) fn command_center_needs_input(
    config: &Config,
) -> Vec<super::attention::NeedsInputSignal> {
    use crate::openhuman::agent_orchestration::command_center::build_view;
    use crate::openhuman::session_db::run_ledger::{
        list_agent_runs, AgentRunListRequest, AgentRunStatus,
    };
    let request = AgentRunListRequest {
        status: Some(AgentRunStatus::AwaitingUser.as_str().to_string()),
        kind: None,
        parent_run_id: None,
        parent_thread_id: None,
        limit: Some(ATTENTION_RUN_LIMIT),
        offset: None,
    };
    match list_agent_runs(config, &request) {
        Ok(response) => {
            super::attention::needs_input_from_command_center(build_view(response.runs))
        }
        Err(e) => {
            log::warn!(target: LOG, "[orchestration_rpc] attention.command_center_failed: {e}");
            Vec::new()
        }
    }
}

/// Gather unread attention signals from the orchestration store: every non-pinned
/// session with a positive unread count. The pinned master/subconscious windows
/// are excluded — they are not agent instances.
pub(super) fn gather_unread_signals(
    conn: &rusqlite::Connection,
) -> anyhow::Result<Vec<super::attention::UnreadSignal>> {
    let mut out: Vec<super::attention::UnreadSignal> = Vec::new();
    for session in store::list_sessions(conn)? {
        if matches!(session.session_id.as_str(), "master" | "subconscious") {
            continue;
        }
        let unread = store::unread_count(conn, &session.session_id)?;
        if unread > 0 {
            out.push(super::attention::UnreadSignal {
                session_id: session.session_id,
                label: session.label,
                unread,
                last_message_at: Some(session.last_message_at),
            });
        }
    }
    Ok(out)
}

/// Gather remote-approval attention signals from the orchestration store: every
/// session whose persisted v2 run-state is `waiting_approval` (Phase 1 stamps
/// this — plus the prompt in `current_detail` and the in-flight `active_call_id`
/// — when a peer harness emits an `approval_request`). Mirrors
/// [`gather_unread_signals`]: pure store read, the pure mapper
/// [`super::attention::remote_approval_signals`] does the filtering.
pub(super) fn gather_remote_approval_signals(
    conn: &rusqlite::Connection,
) -> anyhow::Result<Vec<super::attention::RemoteApprovalSignal>> {
    let signals = super::attention::remote_approval_signals(store::list_sessions(conn)?);
    for sig in &signals {
        log::debug!(
            target: LOG,
            "[orchestration_rpc] attention.remote_approval session_id={} has_call={}",
            sig.session_id,
            sig.active_call_id.is_some(),
        );
    }
    Ok(signals)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::orchestration::types::{OrchestrationMessage, OrchestrationSession};

    #[test]
    fn gather_unread_signals_skips_pinned_and_zero_unread() {
        use super::super::types::ChatKind;
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        };
        let sess = |id: &str, source: &str, label: Option<&str>, at: &str| OrchestrationSession {
            session_id: id.into(),
            agent_id: "@peer".into(),
            source: source.into(),
            label: label.map(str::to_string),
            workspace: None,
            last_seq: 1,
            created_at: "2026-07-06T00:00:00Z".into(),
            last_message_at: at.into(),
            ..Default::default()
        };
        let message = |id: &str, session: &str, kind: ChatKind, at: &str| OrchestrationMessage {
            id: id.into(),
            agent_id: "@peer".into(),
            session_id: session.into(),
            chat_kind: kind,
            role: "user".into(),
            body: "hello".into(),
            timestamp: at.into(),
            seq: 1,
            ..Default::default()
        };

        let signals = store::with_connection(&config.workspace_dir, |conn| {
            // Non-pinned session with one unread message → surfaces.
            store::upsert_session(conn, &sess("h-1", "claude", Some("Claude · audit"), "t1"))?;
            store::insert_message(conn, &message("m1", "h-1", ChatKind::Session, "t1"))?;
            // Pinned master with a message → excluded (not an agent instance).
            store::upsert_session(conn, &sess("master", "core", None, "t2"))?;
            store::insert_message(conn, &message("m2", "master", ChatKind::Master, "t2"))?;
            // Non-pinned session with no messages → zero unread, dropped.
            store::upsert_session(conn, &sess("h-quiet", "codex", None, "t0"))?;
            gather_unread_signals(conn)
        })
        .unwrap();

        assert_eq!(
            signals.len(),
            1,
            "only the non-pinned unread session surfaces"
        );
        assert_eq!(signals[0].session_id, "h-1");
        assert_eq!(signals[0].unread, 1);
        assert_eq!(signals[0].label.as_deref(), Some("Claude · audit"));
    }

    #[test]
    fn command_center_needs_input_surfaces_only_blocked_runs() {
        use crate::openhuman::session_db::run_ledger::{
            upsert_agent_run, AgentRunKind, AgentRunStatus, AgentRunUpsert,
        };
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        };
        let seed = |id: &str, status: AgentRunStatus| {
            upsert_agent_run(
                &config,
                AgentRunUpsert {
                    id: id.into(),
                    kind: AgentRunKind::Subagent,
                    parent_run_id: None,
                    parent_thread_id: Some("thread-1".into()),
                    agent_id: Some("researcher".into()),
                    status,
                    prompt_ref: None,
                    worker_thread_id: None,
                    task_board_id: None,
                    task_card_id: None,
                    checkpoint_path: None,
                    checkpoint: None,
                    summary: None,
                    error: None,
                    metadata: serde_json::json!({}),
                    started_at: None,
                    completed_at: None,
                },
            )
            .unwrap();
        };
        // A blocked run and a working run — only the blocked one is attention-worthy.
        seed("run-blocked", AgentRunStatus::AwaitingUser);
        seed("run-working", AgentRunStatus::Running);

        let signals = command_center_needs_input(&config);
        assert_eq!(signals.len(), 1, "only the AwaitingUser run surfaces");
        assert_eq!(signals[0].run_id, "run-blocked");
    }

    #[test]
    fn self_identity_marks_published_identity_discoverable() {
        let reverse = serde_json::json!({
            "identities": [
                { "username": "  ", "primary": false },   // blank → skipped
                { "username": "openhuman", "primary": false },
                { "username": "oh_primary", "primary": true },
            ]
        });
        let id = build_self_identity("addr123".to_string(), true, Some(&reverse), true);
        assert_eq!(id.agent_id, "addr123");
        assert_eq!(id.handles.len(), 2, "blank username skipped");
        assert_eq!(id.primary_handle.as_deref(), Some("oh_primary"));
        assert!(id.card_published && id.key_published && id.discoverable);
    }

    #[test]
    fn self_identity_primary_falls_back_to_first_handle() {
        let reverse = serde_json::json!({
            "identities": [ { "username": "solo" } ] // no primary flag
        });
        let id = build_self_identity("addr".to_string(), true, Some(&reverse), true);
        assert_eq!(id.primary_handle.as_deref(), Some("solo"));
    }

    #[test]
    fn self_identity_undiscoverable_when_card_or_key_missing() {
        // No reverse (handle-less), card present but key not published → the
        // exact un-messageable case the SelfIdentityCard must flag.
        let no_key = build_self_identity("addr".to_string(), false, None, true);
        assert!(no_key.handles.is_empty());
        assert!(no_key.primary_handle.is_none());
        assert!(!no_key.discoverable, "key not published → not discoverable");

        let no_card = build_self_identity("addr".to_string(), true, None, false);
        assert!(
            !no_card.discoverable,
            "card not published → not discoverable"
        );
    }

    fn test_config(tmp: &tempfile::TempDir) -> Config {
        Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        }
    }

    #[test]
    fn session_reply_is_wrapped_but_master_reply_stays_plain() {
        // A real session id → v1 envelope threaded under that id.
        let wire = session_send_plaintext("h-42", "on it").expect("encode");
        let env = SessionEnvelopeV1::parse(&wire).expect("valid v1 envelope");
        assert_eq!(env.scope.harness_session_id, "h-42");
        assert_eq!(env.message.text, "on it");
        // The pinned windows stay plain (no envelope).
        assert_eq!(session_send_plaintext("master", "hi").unwrap(), "hi");
        assert_eq!(session_send_plaintext("subconscious", "hi").unwrap(), "hi");
    }
}
