//! Orchestration agent tools — read-only session-history browsing + a
//! send-on-behalf DM tool over the tiny.place transport. Offered to agents via
//! the shared tool registry; the reasoning that decides *what* to send now runs
//! in the hosted brain (see [`super::cloud`]).
//!
//! - [`ListSessionsTool`] (`orchestration_list_sessions`) — enumerate the
//!   persisted OpenHuman↔agent session windows (peers/threads, one-line preview).
//! - [`ReadSessionTool`] (`orchestration_read_session`) — read one session's
//!   transcript by id.
//! - [`ListContactsTool`] / [`SendToAgentTool`] — list paired agents and send a
//!   DM on the user's behalf.
//!
//! The read tools are `ReadOnly` and touch only the workspace-internal
//! orchestration DB via [`super::store`].

use std::future::Future;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};

use crate::openhuman::config::Config;
use crate::openhuman::tools::{PermissionLevel, Tool, ToolResult};

use super::store;
use super::types::{ChatKind, OrchestrationMessage, OrchestrationSession, SessionEnvelopeV1};

tokio::task_local! {
    /// The window the current wake cycle is serving (the reasoning core's session
    /// id). `orchestration_send_to_agent` reads it to record where a peer's async
    /// reply should thread back to (the ask's origin). Scoped by the `execute`
    /// node around the reasoning agent's turn (see [`super::ops`]).
    static ORIGIN_SESSION: String;
}

/// Scope the originating wake session id around the reasoning agent's turn `fut`,
/// so the send-on-behalf tool can correlate the eventual reply back to it.
pub async fn with_origin_session<F: Future>(session_id: String, fut: F) -> F::Output {
    ORIGIN_SESSION.scope(session_id, Box::pin(fut)).await
}

/// The current wake cycle's origin session id, or `None` outside a scope.
fn current_origin_session() -> Option<String> {
    ORIGIN_SESSION.try_with(|s| s.clone()).ok()
}

/// Process-global capture of the origin window a **local-master** turn is serving.
///
/// The [`ORIGIN_SESSION`] task-local above is set by the `execute` node around the
/// agent's turn, but it does **not** reach `orchestration_send_to_agent`: the agent
/// harness dispatches tool calls beyond one or more internal `tokio::spawn`
/// boundaries, and task-locals do not cross a `spawn` (standard tokio semantics —
/// the same reason `sandbox_context` re-scopes its mode right at `tool.execute`).
/// So the earlier task-local correlation silently never armed `pending_ask`, and
/// master-initiated asks never threaded their reply back. A process-global is
/// immune to the spawn boundary, so the tool can read it reliably.
///
/// Safe for the master window because local-master wakes are **serialized** by the
/// generation guard (one `master` session, deduped) — at most one master turn
/// brackets this at a time. A concurrent A2A `send_to_agent` during that window
/// would also read the master origin; in the single-user desktop model that
/// overlap is rare and at worst mis-threads one peer reply into master. Documented,
/// not ignored. Peer-session (A2A) W7 stays on the best-effort task-local for now.
static MASTER_ORIGIN: Mutex<Option<String>> = Mutex::new(None);

/// Open a master-origin capture window for the duration of a local-master turn.
/// `origin` is the window a peer's async reply should thread back to (always
/// `"master"`). Paired with [`end_master_origin`]; see [`super::ops`]'s `execute`.
pub fn begin_master_origin(origin: String) {
    if let Ok(mut slot) = MASTER_ORIGIN.lock() {
        *slot = Some(origin);
    }
}

/// Close the capture window opened by [`begin_master_origin`].
pub fn end_master_origin() {
    if let Ok(mut slot) = MASTER_ORIGIN.lock() {
        *slot = None;
    }
}

/// The origin window of the in-flight local-master turn, or `None` outside one.
fn current_master_origin() -> Option<String> {
    MASTER_ORIGIN.lock().ok().and_then(|slot| slot.clone())
}

/// Extract a required string field, returning an error `ToolResult` when absent.
fn required_str(args: &Value, field: &str) -> Result<String, ToolResult> {
    match args.get(field).and_then(Value::as_str) {
        Some(s) if !s.trim().is_empty() => Ok(s.to_string()),
        _ => Err(ToolResult::error(format!("`{field}` is required"))),
    }
}

fn recipient_may_receive_message(
    locally_linked: bool,
    has_known_session: bool,
    accepted_contact: bool,
) -> bool {
    locally_linked || has_known_session || accepted_contact
}

// ── Reasoning-core session-history read tools (Master chat) ──────────────────

/// Default / cap on how many messages a `orchestration_read_session` call returns.
const READ_SESSION_DEFAULT_LIMIT: u32 = 50;
const READ_SESSION_MAX_LIMIT: u32 = 200;
/// Cap on how many session rows `orchestration_list_sessions` returns.
const LIST_SESSIONS_MAX: usize = 100;
/// One-line preview length for the session list (char-safe, matches the roster).
const PREVIEW_MAX_CHARS: usize = 120;

/// The pinned sentinel windows — not agent↔agent transcripts, so they are hidden
/// from the history-browsing tools (the agent reads those via its normal channel).
fn is_pinned_window(session_id: &str) -> bool {
    matches!(session_id, "master" | "subconscious")
}

/// UTF-8-safe one-line preview (mirrors the roster `task_preview` in `schemas.rs`).
fn preview_line(body: &str) -> String {
    let trimmed = body.trim().replace('\n', " ");
    if trimmed.chars().count() <= PREVIEW_MAX_CHARS {
        return trimmed;
    }
    let mut out: String = trimmed.chars().take(PREVIEW_MAX_CHARS - 1).collect();
    out.push('…');
    out
}

/// `orchestration_list_sessions` — enumerate the persisted OpenHuman↔agent session
/// windows so the reasoning core can decide which history to read.
pub struct ListSessionsTool {
    config: Arc<Config>,
}

impl ListSessionsTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ListSessionsTool {
    fn name(&self) -> &str {
        "orchestration_list_sessions"
    }

    fn description(&self) -> &str {
        "List your saved chat sessions with other agents (the persisted OpenHuman↔agent \
         transcripts), newest activity first. Use this to find which past conversation to \
         read before answering a question. Returns each session's id, the peer agent, the \
         source harness, an optional label, the last activity time, the message count, and a \
         one-line preview. Read a session's full transcript with `orchestration_read_session`."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "contactId": {
                    "type": "string",
                    "description": "Optional: only sessions with this contact (peer agent id/address). Omit to list across all contacts."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max sessions to return (default all, capped at 100).",
                    "minimum": 1,
                    "maximum": LIST_SESSIONS_MAX,
                }
            },
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as usize).min(LIST_SESSIONS_MAX))
            .unwrap_or(LIST_SESSIONS_MAX);
        // Optional contact filter: only sessions with this peer (contact-wise view).
        let contact_id = args
            .get("contactId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);

        let workspace = self.config.workspace_dir.clone();
        let result = store::with_connection(&workspace, |conn| {
            let sessions: Vec<OrchestrationSession> = store::list_sessions(conn)?;
            let mut out = Vec::with_capacity(sessions.len());
            for s in sessions {
                if is_pinned_window(&s.session_id) {
                    continue;
                }
                if let Some(ref cid) = contact_id {
                    if &s.agent_id != cid {
                        continue;
                    }
                }
                let count = store::count_messages(conn, &s.agent_id, &s.session_id)?;
                // Newest message body as a one-line preview. `list_recent_messages`
                // orders newest-first internally (DESC) then reverses, so with a
                // limit of 1 the single returned row is the newest message.
                let preview = store::list_recent_messages(conn, &s.agent_id, &s.session_id, 1)?
                    .last()
                    .map(|m| preview_line(&m.body));
                out.push(json!({
                    "sessionId": s.session_id,
                    "peerAgentId": s.agent_id,
                    "source": s.source,
                    "label": s.label,
                    "lastMessageAt": s.last_message_at,
                    "messageCount": count,
                    "preview": preview,
                }));
                if out.len() >= limit {
                    break;
                }
            }
            Ok(out)
        });

        match result {
            Ok(sessions) => {
                log::debug!(
                    target: "orchestration",
                    "[orchestration] tool.list_sessions returned={}",
                    sessions.len(),
                );
                let body = serde_json::to_string(&json!({ "sessions": sessions }))
                    .unwrap_or_else(|_| "{\"sessions\":[]}".to_string());
                Ok(ToolResult::success(body))
            }
            Err(e) => Ok(ToolResult::error(format!("list_sessions failed: {e}"))),
        }
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
    }
}

/// `orchestration_read_session` — read one session's transcript by id.
pub struct ReadSessionTool {
    config: Arc<Config>,
}

impl ReadSessionTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for ReadSessionTool {
    fn name(&self) -> &str {
        "orchestration_read_session"
    }

    fn description(&self) -> &str {
        "Read the transcript of one of your saved agent chat sessions (from \
         `orchestration_list_sessions`). Returns the messages in chronological order with role, \
         body, and timestamp. Use `before` to page backwards through a long history."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "sessionId": {
                    "type": "string",
                    "description": "The session id to read (from orchestration_list_sessions)."
                },
                "limit": {
                    "type": "integer",
                    "description": "Max messages to return (default 50, capped at 200).",
                    "minimum": 1,
                    "maximum": READ_SESSION_MAX_LIMIT,
                },
                "before": {
                    "type": "string",
                    "description": "Exclusive ISO-8601 timestamp to page backwards from."
                }
            },
            "required": ["sessionId"],
            "additionalProperties": false
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let session_id = match required_str(&args, "sessionId") {
            Ok(s) => s,
            Err(e) => return Ok(e),
        };
        if is_pinned_window(&session_id) {
            return Ok(ToolResult::error(
                "`sessionId` must be an agent session, not a pinned window".to_string(),
            ));
        }
        let limit = args
            .get("limit")
            .and_then(Value::as_u64)
            .map(|n| (n as u32).min(READ_SESSION_MAX_LIMIT))
            .unwrap_or(READ_SESSION_DEFAULT_LIMIT);
        let before = args
            .get("before")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);

        let workspace = self.config.workspace_dir.clone();
        let result = store::with_connection(&workspace, |conn| {
            store::list_messages_by_session(conn, &session_id, limit, before.as_deref())
        });

        match result {
            Ok(messages) => {
                log::debug!(
                    target: "orchestration",
                    "[orchestration] tool.read_session session={session_id} returned={}",
                    messages.len(),
                );
                let rendered: Vec<Value> = messages
                    .into_iter()
                    .map(|m| {
                        json!({
                            "role": m.role,
                            "body": m.body,
                            "timestamp": m.timestamp,
                        })
                    })
                    .collect();
                let body = serde_json::to_string(&json!({
                    "sessionId": session_id,
                    "messages": rendered,
                }))
                .unwrap_or_else(|_| "{\"messages\":[]}".to_string());
                Ok(ToolResult::success(body))
            }
            Err(e) => Ok(ToolResult::error(format!("read_session failed: {e}"))),
        }
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
    }
}

/// `orchestration_list_contacts` — enumerate this agent's tiny.place contacts.
/// The starting point for the browse loop: list contacts → `orchestration_list_sessions`
/// (with `contactId`) for a contact's threads → `orchestration_read_session` for history.
/// Read-only; delegates to the tiny.place `contacts_list` controller (no new logic here).
pub struct ListContactsTool;

#[async_trait]
impl Tool for ListContactsTool {
    fn name(&self) -> &str {
        "orchestration_list_contacts"
    }

    fn description(&self) -> &str {
        "List your tiny.place contacts — the agents you're connected with and can message. Use \
         this to find who to read a session history from (orchestration_list_sessions with that \
         contactId, then orchestration_read_session) or who to message (orchestration_send_to_agent). \
         Returns each contact's agent id (address) and handle/label."
    }

    fn parameters_schema(&self) -> Value {
        json!({ "type": "object", "properties": {}, "additionalProperties": false })
    }

    async fn execute(&self, _args: Value) -> anyhow::Result<ToolResult> {
        match crate::openhuman::tinyplace::handle_tinyplace_contacts_list(serde_json::Map::new())
            .await
        {
            Ok(v) => {
                log::debug!(target: "orchestration", "[orchestration] tool.list_contacts ok");
                Ok(ToolResult::success(
                    serde_json::to_string(&v).unwrap_or_else(|_| "{}".to_string()),
                ))
            }
            Err(e) => Ok(ToolResult::error(format!("list_contacts failed: {e}"))),
        }
    }

    fn is_concurrency_safe(&self, _args: &Value) -> bool {
        true
    }
}

// ── Reasoning-core send-on-behalf tool (Master chat) ─────────────────────────

/// `orchestration_send_to_agent` — DM another agent on OpenHuman's behalf.
///
/// Guardrail (owner decision): **linked peers only** — the recipient must be a
/// linked/paired agent OR one this OpenHuman already has a session with. This
/// tool runs under a background origin that bypasses the interactive approval
/// gate, so cold-DMing an arbitrary new address is refused here rather than
/// prompting.
///
/// Session id (owner decision): **reuse-or-mint per peer** — reuse the peer's
/// most recent thread (its shared `wrapper_session_id`) so the reply threads
/// back into the same session (#227/#4582); mint a fresh uuid only when there is
/// no existing thread. An explicit `sessionId` overrides the lookup.
///
/// Effect: sends a v1 session envelope over the tiny.place Signal channel and
/// records the outbound message (`role = "owner"`) in the session window (so it
/// shows in the chat + the agent's own history). The peer's reply arrives
/// asynchronously via the normal ingest → wake path — this call does not block
/// for it.
pub struct SendToAgentTool {
    config: Arc<Config>,
}

impl SendToAgentTool {
    pub fn new(config: Arc<Config>) -> Self {
        Self { config }
    }
}

#[async_trait]
impl Tool for SendToAgentTool {
    fn name(&self) -> &str {
        "orchestration_send_to_agent"
    }

    fn description(&self) -> &str {
        "Send a direct message to another agent on OpenHuman's behalf (e.g. to ask them \
         something for the user). Only works for agents you are already linked with or have \
         chatted with before. By default the message threads into your existing conversation \
         with that agent (so their reply comes back into the same session); pass `sessionId` to \
         target a specific thread. The reply arrives asynchronously — it will show up in that \
         session, not as this tool's return value."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "recipient": {
                    "type": "string",
                    "description": "The agent id (address/@handle) to message. Must be a linked or already-known peer."
                },
                "message": {
                    "type": "string",
                    "description": "The message body to send."
                },
                "sessionId": {
                    "type": "string",
                    "description": "Optional: send under this existing session id. Omit to reuse your latest thread with the peer, or mint a new one."
                }
            },
            "required": ["recipient", "message"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        // External send effect — a Write-class action, so a read-only channel
        // cannot invoke it even though the deep-reasoning core can.
        PermissionLevel::Write
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let recipient = match required_str(&args, "recipient") {
            Ok(s) => s,
            Err(e) => return Ok(e),
        };
        let message = match required_str(&args, "message") {
            Ok(s) => s,
            Err(e) => return Ok(e),
        };
        let explicit_session = args
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && !is_pinned_window(s))
            .map(str::to_string);

        let workspace = self.config.workspace_dir.clone();

        // Guardrail: locally linked peer, accepted tiny.place contact, OR an
        // existing session with them. Never cold-DM an arbitrary new address
        // from this un-gated background origin. The live contact check matters
        // after login changes the workspace: the relay contact remains accepted
        // while the workspace-local orchestration pairing/session stores start
        // empty.
        let linked =
            crate::openhuman::agent_orchestration::pairing::linked_agent_ids(&workspace).await;
        let known_session = store::with_connection(&workspace, |conn| {
            store::latest_session_for_agent(conn, &recipient)
        })
        .map_err(|e| anyhow::anyhow!("lookup session: {e}"))?;
        let accepted_contact = if linked.contains(&recipient) || known_session.is_some() {
            false
        } else {
            match crate::openhuman::agent_orchestration::pairing::is_accepted_contact(&recipient)
                .await
            {
                Ok(accepted) => accepted,
                Err(e) => {
                    log::warn!(
                        target: "orchestration",
                        "[orchestration] tool.send_to_agent contact check failed: {e}"
                    );
                    false
                }
            }
        };
        if !recipient_may_receive_message(
            linked.contains(&recipient),
            known_session.is_some(),
            accepted_contact,
        ) {
            log::debug!(
                target: "orchestration",
                "[orchestration] tool.send_to_agent refused unlinked recipient",
            );
            return Ok(ToolResult::error(
                "recipient is not a linked or previously-contacted agent — cannot send".to_string(),
            ));
        }

        // Session id: explicit → reuse latest thread → mint fresh.
        let session_id = explicit_session
            .or(known_session)
            .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

        let now = chrono::Utc::now().to_rfc3339();
        let message_id = format!("orch-ask:{}", uuid::Uuid::new_v4());

        // Wrap in a v1 session envelope so a compliant peer threads its reply
        // under the same session id.
        let plaintext = match serde_json::to_string(&SessionEnvelopeV1::outgoing(
            &session_id,
            &message,
            &message_id,
            &now,
        )) {
            Ok(s) => s,
            Err(e) => return Ok(ToolResult::error(format!("envelope encode: {e}"))),
        };

        // Send over the tiny.place Signal channel (same op the graph's send_dm and
        // the send_master RPC use).
        let mut send_params = serde_json::Map::new();
        send_params.insert("recipient".to_string(), Value::from(recipient.clone()));
        send_params.insert("plaintext".to_string(), Value::from(plaintext));
        if let Err(e) =
            crate::openhuman::tinyplace::handle_tinyplace_signal_send_message(send_params).await
        {
            log::warn!(target: "orchestration", "[orchestration] tool.send_to_agent send failed: {e}");
            return Ok(ToolResult::error(format!("send failed: {e}")));
        }

        // Record the outbound message in the session window (role=owner) so it
        // surfaces in the chat + the agent's own history, and notify the renderer.
        // Mirrors effect_executor::persist_reply and the send_master RPC.
        let persisted = store::with_connection(&workspace, |conn| {
            let seq = store::next_session_seq(conn, &recipient, &session_id)?;
            store::upsert_session(
                conn,
                &OrchestrationSession {
                    session_id: session_id.clone(),
                    agent_id: recipient.clone(),
                    source: String::new(),
                    label: None,
                    workspace: None,
                    last_seq: seq,
                    created_at: now.clone(),
                    last_message_at: now.clone(),
                    ..Default::default()
                },
            )?;
            store::insert_message(
                conn,
                &OrchestrationMessage {
                    id: message_id.clone(),
                    agent_id: recipient.clone(),
                    session_id: session_id.clone(),
                    chat_kind: ChatKind::Session,
                    role: "owner".to_string(),
                    body: message.clone(),
                    timestamp: now.clone(),
                    seq,
                    ..Default::default()
                },
            )
        });
        if let Err(e) = persisted {
            log::warn!(target: "orchestration", "[orchestration] tool.send_to_agent persist failed: {e}");
        } else {
            super::bus::notify_orchestration_message(
                &recipient,
                &session_id,
                ChatKind::Session.as_str(),
            );
        }

        // Correlate the eventual reply back to the window this ask came from, so
        // the wake path threads the peer's answer into the Master chat (or the
        // originating session) instead of auto-replying to the peer. One-shot:
        // consumed by the next inbound message on this session. Skipped when the
        // origin is unknown (tool invoked outside a wake) or is the same session.
        //
        // Resolve origin from the process-global master beacon FIRST — it survives
        // the harness's `tokio::spawn` tool-dispatch boundary that drops the
        // `ORIGIN_SESSION` task-local (which is why this correlation used to never
        // arm). Fall back to the task-local for the best-effort A2A peer path.
        if let Some(origin) = current_master_origin().or_else(current_origin_session) {
            if origin != session_id && !origin.is_empty() {
                // Key by (recipient, session) so a legacy session-id collision with a
                // different peer can't consume this ask (matches the store scoping).
                if let Err(e) = store::with_connection(&workspace, |conn| {
                    store::set_pending_ask(conn, &recipient, &session_id, &origin)
                }) {
                    log::warn!(target: "orchestration", "[orchestration] tool.send_to_agent correlate failed: {e}");
                }
            }
        }

        log::debug!(
            target: "orchestration",
            "[orchestration] tool.send_to_agent sent session={session_id}",
        );
        let body = serde_json::to_string(&json!({
            "ok": true,
            "sessionId": session_id,
            "note": "Message sent. Fire-and-forget: the reply arrives later and is \
                     surfaced to this chat AUTOMATICALLY when it comes. Do NOT wait, \
                     poll, or call read_session for it — just tell your human you've \
                     asked and will report back, then end your turn.",
        }))
        .unwrap_or_else(|_| "{\"ok\":true}".to_string());
        Ok(ToolResult::success(body))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn master_origin_beacon_sets_and_clears() {
        // The W7 arming fix: unlike the `ORIGIN_SESSION` task-local (which does not
        // survive the harness's `tokio::spawn` tool-dispatch boundary), this
        // process-global beacon is readable from `orchestration_send_to_agent`.
        end_master_origin(); // normalize against any cross-test residue
        assert_eq!(current_master_origin(), None);

        // Inside a local-master turn: the tool can read the origin to arm pending_ask.
        begin_master_origin("master".to_string());
        assert_eq!(current_master_origin(), Some("master".to_string()));

        // Closed after the turn: a later A2A wake cannot read a stale master origin.
        end_master_origin();
        assert_eq!(current_master_origin(), None);
    }

    // ── session-history read tools ──────────────────────────────────────────

    use super::super::types::{ChatKind, OrchestrationMessage};

    fn test_config(tmp: &tempfile::TempDir) -> Arc<Config> {
        Arc::new(Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        })
    }

    fn seed_msg(
        conn: &rusqlite::Connection,
        session: &str,
        seq: i64,
        role: &str,
        body: &str,
        ts: &str,
    ) {
        store::insert_message(
            conn,
            &OrchestrationMessage {
                id: format!("{session}-{seq}"),
                agent_id: "@peer".into(),
                session_id: session.into(),
                chat_kind: ChatKind::Session,
                role: role.into(),
                body: body.into(),
                timestamp: ts.into(),
                seq,
                ..Default::default()
            },
        )
        .unwrap();
    }

    fn seed_session(conn: &rusqlite::Connection, session: &str, source: &str, last_at: &str) {
        store::upsert_session(
            conn,
            &OrchestrationSession {
                session_id: session.into(),
                agent_id: "@peer".into(),
                source: source.into(),
                label: None,
                workspace: None,
                last_seq: 0,
                created_at: last_at.into(),
                last_message_at: last_at.into(),
                ..Default::default()
            },
        )
        .unwrap();
    }

    #[tokio::test]
    async fn list_sessions_tool_lists_agent_sessions_and_hides_pinned() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        store::with_connection(&config.workspace_dir, |conn| {
            seed_session(conn, "s-1", "claude", "2026-07-02T00:01:00Z");
            seed_msg(
                conn,
                "s-1",
                1,
                "user",
                "how do I ship it?",
                "2026-07-02T00:01:00Z",
            );
            // A pinned window must be excluded from the history browser.
            seed_session(conn, "master", "master", "2026-07-02T00:02:00Z");
            seed_msg(conn, "master", 1, "user", "steer", "2026-07-02T00:02:00Z");
            Ok(())
        })
        .unwrap();

        let tool = ListSessionsTool::new(config);
        let out = tool.execute(json!({})).await.unwrap();
        assert!(!out.is_error);
        let v: Value = serde_json::from_str(&out.text()).unwrap();
        let sessions = v["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1, "only the agent session, not master");
        assert_eq!(sessions[0]["sessionId"], "s-1");
        assert_eq!(sessions[0]["peerAgentId"], "@peer");
        assert_eq!(sessions[0]["source"], "claude");
        assert_eq!(sessions[0]["messageCount"], 1);
        assert_eq!(sessions[0]["preview"], "how do I ship it?");
    }

    #[tokio::test]
    async fn list_sessions_tool_filters_by_contact() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        // Two contacts, one session each.
        let sess = |agent: &str, session: &str| OrchestrationSession {
            session_id: session.into(),
            agent_id: agent.into(),
            source: "claude".into(),
            label: None,
            workspace: None,
            last_seq: 0,
            created_at: "2026-07-02T00:01:00Z".into(),
            last_message_at: "2026-07-02T00:01:00Z".into(),
            ..Default::default()
        };
        store::with_connection(&config.workspace_dir, |conn| {
            store::upsert_session(conn, &sess("@alice", "s-alice"))?;
            store::upsert_session(conn, &sess("@bob", "s-bob"))?;
            Ok(())
        })
        .unwrap();

        let tool = ListSessionsTool::new(config);
        // No filter → both contacts' sessions.
        let all = tool.execute(json!({})).await.unwrap();
        let v: Value = serde_json::from_str(&all.text()).unwrap();
        assert_eq!(v["sessions"].as_array().unwrap().len(), 2);

        // contactId filter → only that contact's sessions.
        let out = tool
            .execute(json!({ "contactId": "@alice" }))
            .await
            .unwrap();
        let v: Value = serde_json::from_str(&out.text()).unwrap();
        let sessions = v["sessions"].as_array().unwrap();
        assert_eq!(sessions.len(), 1);
        assert_eq!(sessions[0]["peerAgentId"], "@alice");
        assert_eq!(sessions[0]["sessionId"], "s-alice");
    }

    #[tokio::test]
    async fn read_session_tool_returns_transcript_chronologically() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        store::with_connection(&config.workspace_dir, |conn| {
            seed_session(conn, "s-1", "codex", "2026-07-02T00:03:00Z");
            seed_msg(conn, "s-1", 1, "user", "first", "2026-07-02T00:01:00Z");
            seed_msg(conn, "s-1", 2, "agent", "second", "2026-07-02T00:02:00Z");
            Ok(())
        })
        .unwrap();

        let tool = ReadSessionTool::new(config);
        let out = tool.execute(json!({ "sessionId": "s-1" })).await.unwrap();
        assert!(!out.is_error);
        let v: Value = serde_json::from_str(&out.text()).unwrap();
        let msgs = v["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0]["role"], "user");
        assert_eq!(msgs[0]["body"], "first");
        assert_eq!(msgs[1]["body"], "second"); // chronological order
    }

    #[tokio::test]
    async fn read_session_tool_rejects_missing_id_and_pinned_window() {
        let tmp = tempfile::tempdir().unwrap();
        let config = test_config(&tmp);
        let tool = ReadSessionTool::new(config);
        assert!(tool.execute(json!({})).await.unwrap().is_error);
        assert!(
            tool.execute(json!({ "sessionId": "master" }))
                .await
                .unwrap()
                .is_error
        );
    }

    // ── send-on-behalf tool ─────────────────────────────────────────────────

    #[tokio::test]
    async fn send_to_agent_rejects_missing_args() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = SendToAgentTool::new(test_config(&tmp));
        assert!(tool.execute(json!({})).await.unwrap().is_error);
        assert!(
            tool.execute(json!({ "recipient": "@peer" }))
                .await
                .unwrap()
                .is_error
        ); // missing message
        assert!(
            tool.execute(json!({ "message": "hi" }))
                .await
                .unwrap()
                .is_error
        ); // missing recipient
    }

    #[tokio::test]
    async fn send_to_agent_refuses_unlinked_unknown_recipient() {
        // Empty workspace: no linked peers, no existing session. The guardrail
        // must refuse BEFORE any network send (this test does no I/O to tiny.place).
        let tmp = tempfile::tempdir().unwrap();
        let tool = SendToAgentTool::new(test_config(&tmp));
        let out = tool
            .execute(json!({ "recipient": "@stranger", "message": "hello" }))
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.text().contains("not a linked"));
    }

    #[test]
    fn accepted_contact_can_receive_without_workspace_pairing_or_session() {
        assert!(recipient_may_receive_message(false, false, true));
        assert!(recipient_may_receive_message(true, false, false));
        assert!(recipient_may_receive_message(false, true, false));
        assert!(!recipient_may_receive_message(false, false, false));
    }
}
