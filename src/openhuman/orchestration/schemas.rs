//! JSON-RPC read surface for the orchestration layer.
//!
//! Renderer-only controllers (internal registry — never advertised to agents):
//! the `TinyPlaceOrchestrationTab` reads sessions + messages from the local
//! render cache (kept in sync with the hosted brain by `sync`), sends Master
//! steering DMs, and marks chats read. Namespace: `orchestration`; methods
//! `openhuman.orchestration_*`.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::{rpc as config_rpc, Config};

use super::attention;
use super::presence;
use super::store;
use super::types::{
    ChatKind, OrchestrationMessage, OrchestrationSession, SessionEnvelopeV1, LOCAL_MASTER_AGENT,
};

/// Active-window: a session is "active" if it saw traffic within this many ms.
const ACTIVE_WINDOW_MS: i64 = 45 * 60 * 1000;
const LOG: &str = "orchestration_rpc";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("orchestration_sessions_list"),
        schema_for("orchestration_sessions_create"),
        schema_for("orchestration_messages_list"),
        schema_for("orchestration_send_master_message"),
        schema_for("orchestration_mark_read"),
        schema_for("orchestration_status"),
        schema_for("orchestration_attention"),
        schema_for("orchestration_self_identity"),
        schema_for("orchestration_publish_identity"),
        schema_for("orchestration_relay_info"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("orchestration_sessions_list"),
            handler: handle_sessions_list,
        },
        RegisteredController {
            schema: schema_for("orchestration_sessions_create"),
            handler: handle_sessions_create,
        },
        RegisteredController {
            schema: schema_for("orchestration_messages_list"),
            handler: handle_messages_list,
        },
        RegisteredController {
            schema: schema_for("orchestration_send_master_message"),
            handler: handle_send_master_message,
        },
        RegisteredController {
            schema: schema_for("orchestration_mark_read"),
            handler: handle_mark_read,
        },
        RegisteredController {
            schema: schema_for("orchestration_status"),
            handler: handle_status,
        },
        RegisteredController {
            schema: schema_for("orchestration_attention"),
            handler: handle_attention,
        },
        RegisteredController {
            schema: schema_for("orchestration_self_identity"),
            handler: handle_self_identity,
        },
        RegisteredController {
            schema: schema_for("orchestration_publish_identity"),
            handler: handle_publish_identity,
        },
        RegisteredController {
            schema: schema_for("orchestration_relay_info"),
            handler: handle_relay_info,
        },
    ]
}

fn schema_for(function: &str) -> ControllerSchema {
    match function {
        "orchestration_sessions_list" => ControllerSchema {
            namespace: "orchestration",
            function: "sessions_list",
            description: "List orchestration chat windows (pinned master + subconscious plus per-session) with computed active + unread counts.",
            inputs: vec![],
            outputs: vec![json_output("result", "{ sessions: SessionSummary[] }.")],
        },
        "orchestration_sessions_create" => ControllerSchema {
            namespace: "orchestration",
            function: "sessions_create",
            description: "Create a new empty orchestration session for a contact (mints a fresh harness session id). Idempotent per (agentId, sessionId).",
            inputs: vec![
                required_str("agentId", "Contact agent id (address) the new session belongs to."),
                optional_str("label", "Optional human-friendly label for the session."),
            ],
            outputs: vec![json_output("result", "{ session: SessionSummary }.")],
        },
        "orchestration_messages_list" => ControllerSchema {
            namespace: "orchestration",
            function: "messages_list",
            description: "List messages for a chat: \"master\", \"subconscious\", or a harness session id.",
            inputs: vec![
                required_str("chat", "Chat key: \"master\" | \"subconscious\" | <sessionId>."),
                optional_str("before", "Exclusive ISO timestamp to page backwards from."),
                FieldSchema {
                    name: "limit",
                    ty: TypeSchema::Option(Box::new(TypeSchema::U64)),
                    comment: "Max messages to return (default 100, capped at 500).",
                    required: false,
                },
            ],
            outputs: vec![json_output("result", "{ messages: OrchestrationMessage[] }.")],
        },
        "orchestration_send_master_message" => ControllerSchema {
            namespace: "orchestration",
            function: "send_master_message",
            description: "Send a Master steering DM (owner → front-end agent) over the signal-send op. With a sessionId, sends a session-scoped envelope instead and threads it under that session window.",
            inputs: vec![
                required_str("body", "Message body to send to the Master counterpart."),
                optional_str("recipient", "Recipient agent id; defaults to the latest Master peer."),
                optional_str("sessionId", "Session id to send under; when set the body is wrapped in a v1 session envelope and mirrored into that session window instead of Master."),
            ],
            outputs: vec![json_output("result", "{ ok: bool, messageId?: string }.")],
        },
        "orchestration_mark_read" => ControllerSchema {
            namespace: "orchestration",
            function: "mark_read",
            description: "Advance a chat's read cursor to its newest message.",
            inputs: vec![required_str("chat", "Chat key: \"master\" | \"subconscious\" | <sessionId>.")],
            outputs: vec![json_output("result", "{ ok: bool }.")],
        },
        "orchestration_status" => ControllerSchema {
            namespace: "orchestration",
            function: "status",
            description: "Current steering directive, last subconscious tick, and ingest health.",
            inputs: vec![],
            outputs: vec![json_output("result", "OrchestrationStatus.")],
        },
        "orchestration_attention" => ControllerSchema {
            namespace: "orchestration",
            function: "attention",
            description: "Aggregate the \"needs you\" signals across the hub — pending local tool approvals, remote sessions parked on an approval, agent runs awaiting input, and instances with unread messages — into one priority-ordered queue.",
            inputs: vec![],
            outputs: vec![json_output("result", "AttentionQueue { items: AttentionItem[], counts }.")],
        },
        "orchestration_self_identity" => ControllerSchema {
            namespace: "orchestration",
            function: "self_identity",
            description: "This agent's own tiny.place identity + discoverability: agent id (address), reverse-resolved @handles, whether its directory card and Signal encryption key are published, and whether peers can therefore DM it. Composes the tinyplace signal/directory reads.",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "{ agentId, handles: {username, primary}[], primaryHandle?, cardPublished, keyPublished, discoverable }.",
            )],
        },
        "orchestration_publish_identity" => ControllerSchema {
            namespace: "orchestration",
            function: "publish_identity",
            description: "Make this agent discoverable: publish (or refresh) its directory card and Signal encryption key for the current wallet identity, then return the updated self-identity. No @handle registration or payment — repairs the common \"has identity but card/key unpublished\" gap that makes inbound DMs 404. Returns the same shape as self_identity.",
            inputs: vec![],
            outputs: vec![json_output(
                "result",
                "{ agentId, handles: {username, primary}[], primaryHandle?, cardPublished, keyPublished, discoverable }.",
            )],
        },
        "orchestration_relay_info" => ControllerSchema {
            namespace: "orchestration",
            function: "relay_info",
            description: "The tiny.place relay endpoint the core is talking to, plus a coarse network label (staging | prod) for the renderer's relay badge.",
            inputs: vec![],
            outputs: vec![json_output("result", "{ baseUrl, network }.")],
        },
        other => unreachable!("unknown orchestration schema: {other}"),
    }
}

// ── DTOs (camelCase for the renderer) ───────────────────────────────────────

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SessionSummary {
    session_id: String,
    agent_id: String,
    source: String,
    /// The emitting harness (claude/codex/gemini) when this is an external agent
    /// instance; absent for the pinned master/subconscious/user-created windows.
    #[serde(skip_serializing_if = "Option::is_none")]
    harness_type: Option<String>,
    /// Coarse instance status for the roster status dot (see `derive_status`).
    status: String,
    /// One-line current activity (latest message preview) for the roster.
    #[serde(skip_serializing_if = "Option::is_none")]
    current_task: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    chat_kind: String,
    last_message_at: String,
    unread: i64,
    /// Total persisted messages in the session (all kinds), for the roster's
    /// per-session count. `0` for the pinned windows / a freshly created session.
    message_count: i64,
    active: bool,
    pinned: bool,
    /// Live peer reachability from the in-memory presence map (`presence.rs`).
    /// `Some(true)` = confidently online (heard from within the TTL);
    /// `Some(false)` = confidently offline (heartbeat, once landed); `None` =
    /// unknown → the UI falls back to the recency-based `active`.
    #[serde(skip_serializing_if = "Option::is_none")]
    peer_online: Option<bool>,
    /// ISO-8601 last time we heard from this peer, if ever.
    #[serde(skip_serializing_if = "Option::is_none")]
    last_seen_at: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct SteeringSummary {
    text: String,
    created_at: String,
    expires_after_cycles: u32,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct OrchestrationStatus {
    #[serde(skip_serializing_if = "Option::is_none")]
    steering: Option<SteeringSummary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    last_tick_at: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ingest_last_message_at: Option<String>,
    /// Sessions with pending wake work (health signal — persistently > 0 means
    /// the wake loop is stuck).
    ingest_cursor_lag: i64,
    /// Most recent orchestration error, if any (short cause string, never a body).
    #[serde(skip_serializing_if = "Option::is_none")]
    last_error: Option<String>,
    /// Whether the hosted brain was reachable at the last health probe. Drives
    /// the "cloud brain unreachable" offline notice in the renderer.
    cloud_reachable: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct RelayInfo {
    base_url: String,
    network: String,
}

/// Resolve the `chat` param to a store session id. `master` / `subconscious` map
/// to their pinned session ids; anything else is treated as a harness session id.
fn chat_to_session_id(chat: &str) -> &str {
    match chat {
        "master" => "master",
        "subconscious" => "subconscious",
        other => other,
    }
}

fn chat_kind_for_session(session_id: &str) -> ChatKind {
    match session_id {
        "master" => ChatKind::Master,
        "subconscious" => ChatKind::Subconscious,
        _ => ChatKind::Session,
    }
}

fn is_active(last_message_at: &str) -> bool {
    match chrono::DateTime::parse_from_rfc3339(last_message_at) {
        Ok(ts) => {
            let age = chrono::Utc::now().signed_duration_since(ts.with_timezone(&chrono::Utc));
            age.num_milliseconds() < ACTIVE_WINDOW_MS
        }
        Err(_) => false,
    }
}

/// The harness provider for a session, when its `source` names one. Session
/// windows persist the emitting harness (claude/codex/gemini) in `source` (see
/// `ingest.rs`); the sentinel windows (master/subconscious/user_created/
/// orchestration) carry no harness and yield `None`.
fn harness_type_for(source: &str) -> Option<String> {
    matches!(source, "claude" | "codex" | "gemini").then(|| source.to_string())
}

/// Coarse instance status for the roster dot. Reads the persisted v2 run-state
/// (`status.state`) when present, mapping it to the renderer's five
/// `InstanceStatus` values; falls back to recency for v1/legacy sessions that
/// carry no run-state.
///
/// Staleness fallback: a `running`/`running_tool` session that has gone silent
/// (no recent traffic → `active == false`) is reported `stopped`, so a crashed
/// instance that never emitted a terminal `status` doesn't sit forever on a stuck
/// green dot. `waiting_approval`/`errored`/`stopped`/`idle` are honoured as-is —
/// they are already terminal/blocked states, not "silently alive" ones.
fn derive_status(status_state: Option<&str>, active: bool) -> &'static str {
    match status_state {
        // Actively working — but downgrade to stopped if the session has gone
        // silent (staleness fallback).
        Some("running") | Some("running_tool") => {
            if active {
                "running"
            } else {
                "stopped"
            }
        }
        Some("waiting_approval") => "waiting-approval",
        Some("idle") => "idle",
        Some("stopped") => "stopped",
        Some("errored") => "errored",
        // No persisted run-state (v1/legacy) or an unrecognised value → the
        // original recency heuristic.
        _ => {
            if active {
                "idle"
            } else {
                "stopped"
            }
        }
    }
}

/// One-line, UTF-8-safe preview of a message body for the roster task line.
/// Truncates on a char boundary and reserves room for the ellipsis so the result
/// never exceeds `MAX` chars (avoids the byte-slice panics noted in the codebase).
fn task_preview(body: &str) -> String {
    const MAX: usize = 80;
    let trimmed = body.trim();
    if trimmed.chars().count() <= MAX {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(MAX - 1).collect();
    out.push('…');
    out
}

fn summarize(
    session: OrchestrationSession,
    unread: i64,
    pinned: bool,
    current_task: Option<String>,
    message_count: i64,
) -> SessionSummary {
    let chat_kind = chat_kind_for_session(&session.session_id);
    let active = pinned || is_active(&session.last_message_at);
    let harness_type = harness_type_for(&session.source);
    let status = derive_status(session.status_state.as_deref(), active).to_string();
    // Overlay live peer presence for real peer sessions only (the pinned
    // master/subconscious windows have no remote peer).
    let (peer_online, last_seen_at) = if pinned {
        (None, None)
    } else {
        (
            presence::is_online(&session.agent_id),
            presence::last_seen_iso(&session.agent_id),
        )
    };
    SessionSummary {
        chat_kind: chat_kind.as_str().to_string(),
        active,
        unread,
        message_count,
        pinned,
        harness_type,
        status,
        current_task,
        peer_online,
        last_seen_at,
        session_id: session.session_id,
        agent_id: session.agent_id,
        source: session.source,
        label: session.label,
        workspace: session.workspace,
        last_message_at: session.last_message_at,
    }
}

// ── Handlers ────────────────────────────────────────────────────────────────

fn handle_sessions_list(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("sessions_list").await?;
        let sessions = store::with_connection(&config.workspace_dir, |conn| {
            let rows = store::list_sessions(conn)?;
            let visible_counts = store::visible_message_counts(conn)?;
            let pinned_visible_counts = store::visible_message_counts_by_session(conn)?;
            let mut out: Vec<SessionSummary> = Vec::with_capacity(rows.len() + 2);
            let mut have_master = false;
            let mut have_subconscious = false;
            for session in rows {
                let unread = store::unread_count(conn, &session.session_id)?;
                match session.session_id.as_str() {
                    "master" => have_master = true,
                    "subconscious" => have_subconscious = true,
                    _ => {}
                }
                let pinned = matches!(session.session_id.as_str(), "master" | "subconscious");
                // Roster task line: prefer the v2 `status.detail` (the harness's own
                // current-activity line / active tool) when present; otherwise fall
                // back to the latest message preview. Pinned windows carry neither.
                let current_task = if pinned {
                    None
                } else {
                    match session
                        .current_detail
                        .as_deref()
                        .map(str::trim)
                        .filter(|d| !d.is_empty())
                    {
                        Some(detail) => Some(task_preview(detail)),
                        None => store::latest_message_preview(
                            conn,
                            &session.agent_id,
                            &session.session_id,
                        )?
                        .map(|body| task_preview(&body)),
                    }
                };
                let message_count = if pinned {
                    pinned_visible_counts
                        .get(&session.session_id)
                        .copied()
                        .unwrap_or(0)
                } else {
                    visible_counts
                        .get(&(session.agent_id.clone(), session.session_id.clone()))
                        .copied()
                        .unwrap_or(0)
                };
                out.push(summarize(
                    session,
                    unread,
                    pinned,
                    current_task,
                    message_count,
                ));
            }
            // Ensure the pinned windows always exist even before any traffic.
            if !have_master {
                out.push(pinned_placeholder("master"));
            }
            if !have_subconscious {
                out.push(pinned_placeholder("subconscious"));
            }
            Ok(out)
        })
        .map_err(|e| format!("sessions_list: {e:#}"))?;
        to_json(serde_json::json!({ "sessions": sessions }))
    })
}

fn handle_sessions_create(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("sessions_create").await?;
        let agent_id = required_param(&params, "agentId")?.to_string();
        let label = params
            .get("label")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        let session_id = uuid::Uuid::new_v4().to_string();
        let now = chrono::Utc::now().to_rfc3339();
        log::debug!(
            target: LOG,
            "[orchestration_rpc] sessions_create agent_id={agent_id} session_id={session_id}"
        );
        let session = OrchestrationSession {
            session_id: session_id.clone(),
            agent_id: agent_id.clone(),
            source: "user_created".to_string(),
            label,
            workspace: None,
            last_seq: 0,
            created_at: now.clone(),
            last_message_at: now.clone(),
            ..Default::default()
        };
        store::with_connection(&config.workspace_dir, |conn| {
            store::upsert_session(conn, &session)
        })
        .map_err(|e| format!("sessions_create: {e:#}"))?;
        super::bus::notify_orchestration_message(&agent_id, &session_id, "session");
        to_json(serde_json::json!({ "session": summarize(session, 0, false, None, 0) }))
    })
}

fn pinned_placeholder(session_id: &str) -> SessionSummary {
    SessionSummary {
        session_id: session_id.to_string(),
        agent_id: session_id.to_string(),
        source: "orchestration".to_string(),
        harness_type: None,
        status: derive_status(None, true).to_string(),
        current_task: None,
        label: None,
        workspace: None,
        chat_kind: chat_kind_for_session(session_id).as_str().to_string(),
        last_message_at: String::new(),
        unread: 0,
        message_count: 0,
        active: true,
        pinned: true,
        peer_online: None,
        last_seen_at: None,
    }
}

fn handle_messages_list(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("messages_list").await?;
        let chat = required_param(&params, "chat")?.to_string();
        let session_id = chat_to_session_id(&chat).to_string();
        let before = params
            .get("before")
            .and_then(Value::as_str)
            .map(str::to_string);
        let limit = params
            .get("limit")
            .and_then(Value::as_u64)
            .unwrap_or(100)
            .min(500) as u32;
        let messages: Vec<OrchestrationMessage> =
            store::with_connection(&config.workspace_dir, |conn| {
                store::list_messages_by_session(conn, &session_id, limit, before.as_deref())
            })
            .map_err(|e| format!("messages_list: {e:#}"))?;
        to_json(serde_json::json!({ "messages": messages }))
    })
}

/// Build the v1 session-envelope wire body for an outgoing session message so a
/// compliant peer harness threads the reply under the same `session_id`.
fn session_envelope_plaintext(
    session_id: &str,
    body: &str,
    message_id: &str,
    now: &str,
) -> Result<String, String> {
    serde_json::to_string(&SessionEnvelopeV1::outgoing(
        session_id, body, message_id, now,
    ))
    .map_err(|e| format!("envelope encode: {e}"))
}

fn handle_send_master_message(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("send_master_message").await?;
        let body = required_param(&params, "body")?.to_string();
        let explicit = params
            .get("recipient")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
            .map(str::to_string);
        // When present, the message threads under this session (envelope) rather
        // than the Master window.
        let session_id = params
            .get("sessionId")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty() && *s != "master" && *s != "subconscious")
            .map(str::to_string);

        // W2 — "ask OpenHuman locally". No recipient AND no session id means the
        // human is asking the OpenHuman agent itself via the Master chat (not
        // steering an external peer). Route the question into OpenHuman's OWN
        // reasoning graph: persist it in the Master window + wake the reasoning
        // core locally — NO outbound peer DM, no recipient required. The core
        // answers (using its history/read tools and, if it needs a real external
        // agent, `orchestration_send_to_agent` + W7 threading) and its reply lands
        // back in this Master window. This is the human↔OpenHuman channel; peer
        // steering still works by passing an explicit `recipient`/`sessionId`.
        if explicit.is_none() && session_id.is_none() {
            let now = chrono::Utc::now().to_rfc3339();
            let message_id = format!("master-ask:{now}");
            let persisted: Result<i64, _> = store::with_connection(&config.workspace_dir, |conn| {
                let seq = store::next_session_seq(conn, LOCAL_MASTER_AGENT, "master")?;
                store::upsert_session(
                    conn,
                    &OrchestrationSession {
                        session_id: "master".to_string(),
                        agent_id: LOCAL_MASTER_AGENT.to_string(),
                        source: "master".to_string(),
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
                        agent_id: LOCAL_MASTER_AGENT.to_string(),
                        session_id: "master".to_string(),
                        chat_kind: ChatKind::Master,
                        role: "user".to_string(),
                        body: body.clone(),
                        timestamp: now.clone(),
                        seq,
                        ..Default::default()
                    },
                )?;
                Ok(seq)
            });
            let seq = match persisted {
                Ok(seq) => seq,
                Err(e) => return Err(format!("master ask persist: {e}")),
            };
            // Fan to the renderer so the question shows immediately.
            super::bus::notify_orchestration_message(
                LOCAL_MASTER_AGENT,
                "master",
                ChatKind::Master.as_str(),
            );
            // Forward the ask to the hosted brain — it wakes, reasons, and returns
            // the reply as a `send_dm` effect on the "master" session, which the
            // device renders back into this window (see `effect_executor`). No wake
            // graph runs on the device.
            let ts = super::wire::parse_ts_ms(&now).unwrap_or(0);
            let envelope = super::wire::OrchestrationEventEnvelopeWire::build(
                LOCAL_MASTER_AGENT,
                "master",
                seq,
                "user",
                LOCAL_MASTER_AGENT,
                &body,
                ts,
                "message",
            );
            let forward_cfg = config.clone();
            tokio::spawn(async move {
                match super::cloud::push_event(&forward_cfg, &envelope).await {
                    Ok(cycle_id) => {
                        // Record this Master-chat cycle's origin so the local-exec
                        // trust gate authorizes it (counterpart == LOCAL_MASTER_AGENT).
                        if let Some(cid) = cycle_id {
                            super::exec_gate::record_cycle_origin(
                                &cid,
                                &envelope.counterpart_agent_id,
                                &envelope.session_id,
                            );
                        }
                    }
                    Err(e) => {
                        log::warn!(target: LOG, "[orchestration_rpc] master_ask.forward_failed: {e}")
                    }
                }
            });
            log::debug!(target: LOG, "[orchestration_rpc] master_ask.forwarded id={message_id} seq={seq}");
            return to_json(serde_json::json!({ "ok": true, "messageId": message_id }));
        }

        // Resolve the recipient: explicit wins; otherwise the session's contact
        // (session mode) or the latest Master peer (master mode).
        let recipient = match (explicit, session_id.as_deref()) {
            (Some(r), _) => r,
            (None, Some(sid)) => {
                let sid = sid.to_string();
                store::with_connection(&config.workspace_dir, move |conn| {
                    store::session_agent_id(conn, &sid)
                })
                .map_err(|e| format!("resolve session recipient: {e:#}"))?
                .ok_or_else(|| "unknown session — specify a recipient".to_string())?
            }
            (None, None) => {
                store::with_connection(&config.workspace_dir, store::latest_master_peer)
                    .map_err(|e| format!("resolve recipient: {e:#}"))?
                    .ok_or_else(|| "no Master counterpart yet — specify a recipient".to_string())?
            }
        };

        let now = chrono::Utc::now().to_rfc3339();
        let (window, chat_kind, message_id) = match &session_id {
            Some(sid) => (sid.clone(), ChatKind::Session, format!("session-out:{now}")),
            None => (
                "master".to_string(),
                ChatKind::Master,
                format!("master-out:{now}"),
            ),
        };

        // Session sends go over the wire as a v1 envelope; Master sends stay plain.
        let plaintext = match &session_id {
            Some(sid) => session_envelope_plaintext(sid, &body, &message_id, &now)?,
            None => body.clone(),
        };

        // Send the E2E DM to the front-end agent (human steering the front end).
        let mut send_params = Map::new();
        send_params.insert("recipient".to_string(), Value::from(recipient.clone()));
        send_params.insert("plaintext".to_string(), Value::from(plaintext));
        crate::openhuman::tinyplace::handle_tinyplace_signal_send_message(send_params)
            .await
            .map_err(|e| format!("signal send: {e}"))?;

        // Mirror it into the target window so the composer's message is visible,
        // and notify the renderer. `upsert_session` never clobbers an existing
        // session's `source`, so a user-created session keeps its origin.
        let persisted = store::with_connection(&config.workspace_dir, |conn| {
            store::upsert_session(
                conn,
                &OrchestrationSession {
                    session_id: window.clone(),
                    agent_id: recipient.clone(),
                    source: match &session_id {
                        Some(_) => "user_created".to_string(),
                        None => "master".to_string(),
                    },
                    label: None,
                    workspace: None,
                    last_seq: 0,
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
                    session_id: window.clone(),
                    chat_kind,
                    role: "owner".to_string(),
                    body: body.clone(),
                    timestamp: now.clone(),
                    seq: 0,
                    ..Default::default()
                },
            )
        });
        if let Err(e) = persisted {
            log::warn!(target: LOG, "[orchestration_rpc] send_master.mirror_failed: {e}");
        }
        super::bus::notify_orchestration_message(&recipient, &window, chat_kind.as_str());

        to_json(serde_json::json!({ "ok": true, "messageId": message_id }))
    })
}

fn handle_mark_read(params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("mark_read").await?;
        let chat = required_param(&params, "chat")?.to_string();
        let session_id = chat_to_session_id(&chat).to_string();
        store::with_connection(&config.workspace_dir, |conn| {
            store::mark_chat_read(conn, &session_id)
        })
        .map_err(|e| format!("mark_read: {e:#}"))?;
        to_json(serde_json::json!({ "ok": true }))
    })
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("status").await?;
        // Steering + reachability come from the hosted health probe's kv cache
        // (see `sync`), so this read stays synchronous and offline-safe.
        let steering =
            super::sync::cached_steering(&config).map(|(text, max_cycles)| SteeringSummary {
                text,
                // Hosted steering carries no local created_at; the renderer keys
                // its display off `text` presence, not this field.
                created_at: String::new(),
                expires_after_cycles: max_cycles,
            });
        let cloud_reachable = super::sync::cloud_reachable(&config);

        let (ingest_last, lag, last_error): (Option<String>, i64, Option<String>) =
            store::with_connection(&config.workspace_dir, |conn| {
                // MAX() always returns exactly one row (NULL when empty). Exclude the
                // pinned master/subconscious windows: they're bumped by manual owner
                // DMs (`handle_send_master_message`), which would otherwise mask a
                // stalled real ingestion pipeline with fresh traffic.
                let ingest_last: Option<String> = conn.query_row(
                    "SELECT MAX(last_message_at) FROM sessions \
                     WHERE session_id NOT IN ('master', 'subconscious')",
                    [],
                    |r| r.get::<_, Option<String>>(0),
                )?;
                let lag = store::ingest_cursor_lag(conn)?;
                let last_error = store::kv_get(conn, "orchestration:last_error")?;
                Ok((ingest_last, lag, last_error))
            })
            .map_err(|e| format!("status: {e:#}"))?;

        // Last subconscious tick (best-effort — subconscious store is separate).
        let last_tick_at =
            crate::openhuman::subconscious::store::with_connection(&config.workspace_dir, |conn| {
                crate::openhuman::subconscious::store::get_last_tick_at(conn, "memory")
            })
            .ok()
            .filter(|v| *v > 0.0);

        to_json(OrchestrationStatus {
            steering,
            last_tick_at,
            ingest_last_message_at: ingest_last.filter(|s| !s.is_empty()),
            ingest_cursor_lag: lag,
            last_error,
            cloud_reachable,
        })
    })
}

fn handle_attention(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("attention").await?;

        // 1. Pending tool approvals (global gate; empty when the gate is not
        //    installed — never an error path). Mapping is the unit-tested pure
        //    `attention::approval_signals`.
        let approvals = attention::approval_signals(
            crate::openhuman::approval::rpc::approval_list_pending()
                .await
                .map_err(|e| format!("attention.approvals: {e}"))?
                .value,
        );

        // 2. Agent runs blocked awaiting user input (command-center NeedsInput).
        //    Best-effort: a command-center read failure must not sink the whole
        //    queue — approvals + unread still surface.
        let needs_input = super::ops::command_center_needs_input(&config);

        // 3. Per-instance unread (non-pinned orchestration sessions). Best-effort
        //    like the command-center read: a transient local-DB hiccup must not
        //    sink the approvals + needs-input signals that already resolved.
        let unread = match store::with_connection(
            &config.workspace_dir,
            super::ops::gather_unread_signals,
        ) {
            Ok(unread) => unread,
            Err(e) => {
                log::warn!(target: LOG, "[orchestration_rpc] attention.unread_failed: {e}");
                Vec::new()
            }
        };

        // 4. Remote agent sessions parked on a tool-approval decision (Phase 1's
        //    persisted `waiting_approval` run-state). Best-effort like the unread
        //    read: a transient local-DB hiccup must not sink the other sources.
        //    These fold into the Approval kind (see `assemble_attention`).
        let remote_approvals = match store::with_connection(
            &config.workspace_dir,
            super::ops::gather_remote_approval_signals,
        ) {
            Ok(remote_approvals) => remote_approvals,
            Err(e) => {
                log::warn!(target: LOG, "[orchestration_rpc] attention.remote_approvals_failed: {e}");
                Vec::new()
            }
        };

        let queue = attention::assemble_attention(approvals, needs_input, unread, remote_approvals);
        log::debug!(
            target: LOG,
            "[orchestration_rpc] attention.exit total={} approvals={} needs_input={} unread={}",
            queue.counts.total,
            queue.counts.approvals,
            queue.counts.needs_input,
            queue.counts.unread,
        );
        to_json(queue)
    })
}

/// Own tiny.place identity + discoverability, composed from the internal
/// tinyplace signal/directory reads. Delegates like `send_master` does
/// (`crate::openhuman::tinyplace::handle_tinyplace_*`), so there is no new
/// tiny.place logic here — only aggregation into the shape the renderer needs.
fn handle_self_identity(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        // 1. Key status → own agent id + whether the encryption key is published
        //    to the directory and current. `encryptionKeyPublished` is false when
        //    the card is missing OR the published key is stale (wrong wallet).
        let key_status =
            crate::openhuman::tinyplace::handle_tinyplace_signal_key_status(Map::new())
                .await
                .map_err(|e| format!("self_identity key_status: {e}"))?;
        let agent_id = key_status
            .get("agentId")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let key_published = key_status
            .get("encryptionKeyPublished")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        log::debug!(
            target: LOG,
            "[orchestration_rpc] self_identity agent_id_len={} key_published={key_published}",
            agent_id.len()
        );

        // 2. Reverse-resolve any @handles this wallet holds. Best-effort: a
        //    handle-less identity is normal (and is exactly the un-messageable
        //    case the card must flag), so a reverse miss is not an error.
        let reverse = if agent_id.is_empty() {
            None
        } else {
            let mut rev_params = Map::new();
            rev_params.insert("cryptoId".to_string(), Value::from(agent_id.clone()));
            // Bounded: a slow/degraded relay must degrade this to "no handle",
            // never hang the whole card on "Loading identity…" forever.
            match tokio::time::timeout(
                std::time::Duration::from_secs(5),
                crate::openhuman::tinyplace::handle_tinyplace_directory_reverse(rev_params),
            )
            .await
            {
                Ok(Ok(reverse)) => Some(reverse),
                Ok(Err(e)) => {
                    log::debug!(target: LOG, "[orchestration_rpc] self_identity reverse miss: {e}");
                    None
                }
                Err(_) => {
                    log::warn!(target: LOG, "[orchestration_rpc] self_identity reverse timed out (relay slow)");
                    None
                }
            }
        };

        // 3. Directory card presence: get_agent(self) 404s when no card is
        //    published. Ok → a card is live; Err → treat as unpublished.
        let card_published = if agent_id.is_empty() {
            false
        } else {
            let mut card_params = Map::new();
            card_params.insert("agentId".to_string(), Value::from(agent_id.clone()));
            // Bounded like the reverse lookup above: a stalled relay degrades to
            // "card not published" rather than pinning the card on "Loading…".
            matches!(
                tokio::time::timeout(
                    std::time::Duration::from_secs(5),
                    crate::openhuman::tinyplace::handle_tinyplace_directory_get_agent(card_params),
                )
                .await,
                Ok(Ok(_))
            )
        };

        let identity = super::ops::build_self_identity(
            agent_id,
            key_published,
            reverse.as_ref(),
            card_published,
        );
        log::debug!(
            target: LOG,
            "[orchestration_rpc] self_identity handles={} card_published={card_published} discoverable={}",
            identity.handles.len(),
            identity.discoverable
        );
        to_json(identity)
    })
}

/// Make this agent fully messageable, then echo back the refreshed identity.
///
/// The `SelfIdentityCard` shows *why* peers can't DM this agent but had no
/// remediation — the user was left to go register a @handle elsewhere even when
/// the wallet already holds one. Two publishes are needed for a peer to actually
/// deliver a first DM, and this does both:
///
/// 1. **Directory card + encryption key** (`signal_register_encryption_key`):
///    upserts the directory card (minting a minimal one if none exists) and
///    writes the wallet's encryption key into its metadata, so peers can *resolve*
///    this address.
/// 2. **X3DH prekey bundle** (`signal_provision`): generates a signed pre-key +
///    one-time pre-keys and uploads them to `/keys/<addr>/bundle`, so peers can
///    *initiate* a Signal session. Without this, resolution succeeds but the very
///    first DM 404s on the bundle fetch — "discoverable" would be a lie. This is
///    the step the old card-only publish missed.
///
/// Then re-reads `self_identity` so the card flips to "discoverable" in place. No
/// @handle registration and no payment: this only publishes presence for the
/// identity the wallet already has. Registering a brand-new paid handle stays in
/// the tiny.place registry surface.
fn handle_publish_identity(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        log::debug!(target: LOG, "[orchestration_rpc] publish_identity entry");
        // 1. Directory card + encryption key → resolvable.
        crate::openhuman::tinyplace::handle_tinyplace_signal_register_encryption_key(Map::new())
            .await
            .map_err(|e| format!("publish_identity card: {e}"))?;
        // 2. X3DH prekey bundle → a peer can initiate the first DM. A peer that
        //    resolves the card but finds no bundle 404s on `/keys/<addr>/bundle`,
        //    so this is required for real messageability, not optional polish.
        crate::openhuman::tinyplace::handle_tinyplace_signal_provision(Map::new())
            .await
            .map_err(|e| format!("publish_identity prekeys: {e}"))?;
        // 3. Re-read discoverability from the directory so the renderer gets the
        //    post-publish truth (card + key now live), not an optimistic guess.
        handle_self_identity(Map::new()).await
    })
}

/// Relay endpoint + network label for the renderer's relay badge. Reads only the
/// configured base URL (no client build / wallet unlock), so it always resolves.
fn handle_relay_info(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let (base_url, network) = crate::openhuman::tinyplace::ops::relay_endpoint();
        log::debug!(target: LOG, "[orchestration_rpc] relay_info network={network}");
        to_json(RelayInfo {
            base_url,
            network: network.to_string(),
        })
    })
}

// ── helpers ─────────────────────────────────────────────────────────────────

async fn load_config(action: &str) -> Result<Config, String> {
    log::debug!(target: LOG, "[orchestration_rpc] {action}.config_load");
    config_rpc::load_config_with_timeout()
        .await
        .inspect_err(|err| {
            log::warn!(target: LOG, "[orchestration_rpc] {action}.config_failed err={err}");
        })
}

fn required_param<'a>(params: &'a Map<String, Value>, key: &str) -> Result<&'a str, String> {
    params
        .get(key)
        .and_then(Value::as_str)
        .filter(|value| !value.trim().is_empty())
        .ok_or_else(|| format!("{key} is required"))
}

fn required_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::String,
        comment,
        required: true,
    }
}

fn optional_str(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Option(Box::new(TypeSchema::String)),
        comment,
        required: false,
    }
}

fn json_output(name: &'static str, comment: &'static str) -> FieldSchema {
    FieldSchema {
        name,
        ty: TypeSchema::Json,
        comment,
        required: true,
    }
}

fn to_json<T: serde::Serialize>(value: T) -> Result<Value, String> {
    serde_json::to_value(value).map_err(|err| format!("serialize orchestration response: {err}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schemas_use_orchestration_namespace() {
        let schemas = all_controller_schemas();
        assert_eq!(schemas.len(), 10);
        assert!(schemas.iter().all(|s| s.namespace == "orchestration"));
        assert_eq!(schema_for("orchestration_attention").function, "attention");
        assert_eq!(
            schema_for("orchestration_self_identity").function,
            "self_identity"
        );
        assert_eq!(
            schema_for("orchestration_publish_identity").function,
            "publish_identity"
        );
        // The publish-identity remediation must be wired into the live registry,
        // not just describable — otherwise the SelfIdentityCard button 404s.
        assert!(all_registered_controllers()
            .iter()
            .any(|c| c.schema.function == "publish_identity"));
        assert_eq!(
            schema_for("orchestration_relay_info").function,
            "relay_info"
        );
        assert_eq!(
            schema_for("orchestration_messages_list").function,
            "messages_list"
        );
        assert_eq!(
            schema_for("orchestration_sessions_create").function,
            "sessions_create"
        );
    }

    #[test]
    fn session_envelope_plaintext_roundtrips_as_v1() {
        let wire =
            session_envelope_plaintext("sess-1", "hello world", "msg-1", "2026-07-04T00:00:00Z")
                .expect("encode");
        let parsed = SessionEnvelopeV1::parse(&wire).expect("valid v1 envelope");
        assert_eq!(parsed.scope.harness_session_id, "sess-1");
        assert_eq!(parsed.message.text, "hello world");
        assert_eq!(parsed.message.role, "owner");
    }

    #[tokio::test]
    async fn created_session_persists_and_resolves_its_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        };
        let now = "2026-07-04T00:00:00Z".to_string();
        let session = OrchestrationSession {
            session_id: "sess-42".to_string(),
            agent_id: "@peer".to_string(),
            source: "user_created".to_string(),
            label: Some("Design review".to_string()),
            workspace: None,
            last_seq: 0,
            created_at: now.clone(),
            last_message_at: now,
            ..Default::default()
        };
        let resolved = store::with_connection(&config.workspace_dir, |conn| {
            store::upsert_session(conn, &session)?;
            let rows = store::list_sessions(conn)?;
            assert!(rows.iter().any(|s| s.session_id == "sess-42"
                && s.source == "user_created"
                && s.agent_id == "@peer"));
            store::session_agent_id(conn, "sess-42")
        })
        .unwrap();
        assert_eq!(resolved.as_deref(), Some("@peer"));
    }

    #[test]
    fn chat_resolution_and_kind() {
        assert_eq!(chat_to_session_id("master"), "master");
        assert_eq!(chat_to_session_id("subconscious"), "subconscious");
        assert_eq!(chat_to_session_id("h1-uuid"), "h1-uuid");
        assert_eq!(chat_kind_for_session("master"), ChatKind::Master);
        assert_eq!(chat_kind_for_session("h1"), ChatKind::Session);
    }

    #[tokio::test]
    async fn sessions_list_includes_pinned_windows_when_empty() {
        // Build against an empty tempdir workspace.
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            workspace_dir: tmp.path().to_path_buf(),
            ..Config::default()
        };
        let sessions = store::with_connection(&config.workspace_dir, |conn| {
            // Directly exercise the pinned-fill logic path via list_sessions.
            let rows = store::list_sessions(conn)?;
            assert!(rows.is_empty());
            Ok(())
        });
        sessions.unwrap();
        // The handler always yields the two pinned placeholders.
        let master = pinned_placeholder("master");
        let sub = pinned_placeholder("subconscious");
        assert_eq!(master.chat_kind, "master");
        assert!(master.pinned && sub.pinned);
    }

    #[test]
    fn required_param_rejects_blank() {
        let mut params = Map::new();
        params.insert("chat".to_string(), Value::String("  ".to_string()));
        assert!(required_param(&params, "chat").is_err());
    }

    #[test]
    fn harness_type_only_for_known_providers() {
        assert_eq!(harness_type_for("claude").as_deref(), Some("claude"));
        assert_eq!(harness_type_for("codex").as_deref(), Some("codex"));
        assert_eq!(harness_type_for("gemini").as_deref(), Some("gemini"));
        // Sentinel / origin sources are not harnesses.
        assert_eq!(harness_type_for("master"), None);
        assert_eq!(harness_type_for("user_created"), None);
        assert_eq!(harness_type_for("orchestration"), None);
    }

    #[test]
    fn status_falls_back_to_recency_without_persisted_state() {
        // v1/legacy sessions carry no run-state → the original recency heuristic.
        assert_eq!(derive_status(None, true), "idle");
        assert_eq!(derive_status(None, false), "stopped");
        // An unrecognised persisted value also falls back to recency.
        assert_eq!(derive_status(Some("weird"), true), "idle");
    }

    #[test]
    fn status_maps_persisted_v2_run_state() {
        assert_eq!(derive_status(Some("running"), true), "running");
        assert_eq!(derive_status(Some("running_tool"), true), "running");
        assert_eq!(
            derive_status(Some("waiting_approval"), true),
            "waiting-approval"
        );
        assert_eq!(derive_status(Some("idle"), true), "idle");
        assert_eq!(derive_status(Some("stopped"), true), "stopped");
        assert_eq!(derive_status(Some("errored"), true), "errored");
        // waiting/errored are honoured even when the session has gone stale.
        assert_eq!(derive_status(Some("errored"), false), "errored");
        assert_eq!(
            derive_status(Some("waiting_approval"), false),
            "waiting-approval"
        );
    }

    #[test]
    fn status_staleness_downgrades_silent_running_to_stopped() {
        // A `running` session that has gone silent (no recent traffic) must not
        // sit on a stuck green dot — it downgrades to stopped.
        assert_eq!(derive_status(Some("running"), false), "stopped");
        assert_eq!(derive_status(Some("running_tool"), false), "stopped");
    }

    #[test]
    fn summarize_reads_persisted_run_state_and_detail() {
        let session = OrchestrationSession {
            session_id: "w2".to_string(),
            agent_id: "@peer".to_string(),
            source: "claude".to_string(),
            last_seq: 3,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            // Recent enough to be "active".
            last_message_at: chrono::Utc::now().to_rfc3339(),
            status_state: Some("waiting_approval".to_string()),
            current_detail: Some("approve rm -rf".to_string()),
            ..Default::default()
        };
        // current_task is threaded from the handler (status.detail preferred there).
        let summary = summarize(session, 0, false, Some("approve rm -rf".to_string()), 7);
        assert_eq!(summary.status, "waiting-approval");
        assert_eq!(summary.current_task.as_deref(), Some("approve rm -rf"));
        assert_eq!(summary.message_count, 7);
    }

    #[test]
    fn task_preview_trims_and_caps_on_char_boundary() {
        assert_eq!(task_preview("  hello  "), "hello");
        // A multibyte string longer than the cap truncates with an ellipsis and
        // never exceeds MAX chars (no mid-codepoint panic).
        let long = "é".repeat(200);
        let preview = task_preview(&long);
        assert_eq!(preview.chars().count(), 80);
        assert!(preview.ends_with('…'));
    }

    #[test]
    fn summarize_derives_harness_status_and_carries_task() {
        let session = OrchestrationSession {
            session_id: "w1".to_string(),
            agent_id: "@peer".to_string(),
            source: "claude".to_string(),
            label: None,
            workspace: None,
            last_seq: 3,
            created_at: "2020-01-01T00:00:00Z".to_string(),
            // Stale timestamp → not active → stopped.
            last_message_at: "2020-01-01T00:00:00Z".to_string(),
            ..Default::default()
        };
        let summary = summarize(session, 2, false, Some("drafting cards".to_string()), 12);
        assert_eq!(summary.harness_type.as_deref(), Some("claude"));
        assert_eq!(summary.status, "stopped");
        assert_eq!(summary.current_task.as_deref(), Some("drafting cards"));
        assert_eq!(summary.message_count, 12);
        assert!(!summary.active);

        // A pinned window is always active → idle, and carries no harness/task.
        let pinned = pinned_placeholder("master");
        assert_eq!(pinned.status, "idle");
        assert!(pinned.harness_type.is_none());
    }
}
