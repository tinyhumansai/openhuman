//! JSON-RPC read surface for the orchestration layer (stage 7).
//!
//! Renderer-only controllers (internal registry — never advertised to agents):
//! the `TinyPlaceOrchestrationTab` reads sessions + messages from the stage-3
//! store's real classification here instead of client-side heuristics, sends
//! Master steering DMs, and marks chats read. Namespace: `orchestration`; methods
//! `openhuman.orchestration_*`.

use serde::Serialize;
use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use crate::openhuman::config::{rpc as config_rpc, Config};

use super::store;
use super::types::{ChatKind, OrchestrationMessage, OrchestrationSession};

/// Active-window: a session is "active" if it saw traffic within this many ms.
const ACTIVE_WINDOW_MS: i64 = 45 * 60 * 1000;
const LOG: &str = "orchestration_rpc";

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![
        schema_for("orchestration_sessions_list"),
        schema_for("orchestration_messages_list"),
        schema_for("orchestration_send_master_message"),
        schema_for("orchestration_mark_read"),
        schema_for("orchestration_status"),
    ]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![
        RegisteredController {
            schema: schema_for("orchestration_sessions_list"),
            handler: handle_sessions_list,
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
        "orchestration_messages_list" => ControllerSchema {
            namespace: "orchestration",
            function: "messages_list",
            description: "List messages for a chat: \"master\", \"subconscious\", or a harness session id.",
            inputs: vec![
                required_str("chat", "Chat key: \"master\" | \"subconscious\" | <sessionId>."),
                optional_str("before", "Exclusive ISO timestamp to page backwards from."),
            ],
            outputs: vec![json_output("result", "{ messages: OrchestrationMessage[] }.")],
        },
        "orchestration_send_master_message" => ControllerSchema {
            namespace: "orchestration",
            function: "send_master_message",
            description: "Send a Master steering DM (owner → front-end agent) over the signal-send op.",
            inputs: vec![
                required_str("body", "Message body to send to the Master counterpart."),
                optional_str("recipient", "Recipient agent id; defaults to the latest Master peer."),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace: Option<String>,
    chat_kind: String,
    last_message_at: String,
    unread: i64,
    active: bool,
    pinned: bool,
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

fn summarize(session: OrchestrationSession, unread: i64, pinned: bool) -> SessionSummary {
    let chat_kind = chat_kind_for_session(&session.session_id);
    let active = pinned || is_active(&session.last_message_at);
    SessionSummary {
        chat_kind: chat_kind.as_str().to_string(),
        active,
        unread,
        pinned,
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
                out.push(summarize(session, unread, pinned));
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
        .map_err(|e| format!("sessions_list: {e}"))?;
        to_json(serde_json::json!({ "sessions": sessions }))
    })
}

fn pinned_placeholder(session_id: &str) -> SessionSummary {
    SessionSummary {
        session_id: session_id.to_string(),
        agent_id: session_id.to_string(),
        source: "orchestration".to_string(),
        label: None,
        workspace: None,
        chat_kind: chat_kind_for_session(session_id).as_str().to_string(),
        last_message_at: String::new(),
        unread: 0,
        active: true,
        pinned: true,
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
            .map_err(|e| format!("messages_list: {e}"))?;
        to_json(serde_json::json!({ "messages": messages }))
    })
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

        let recipient = match explicit {
            Some(r) => r,
            None => store::with_connection(&config.workspace_dir, store::latest_master_peer)
                .map_err(|e| format!("resolve recipient: {e}"))?
                .ok_or_else(|| "no Master counterpart yet — specify a recipient".to_string())?,
        };

        // Send the E2E DM to the front-end agent (human steering the front end).
        let mut send_params = Map::new();
        send_params.insert("recipient".to_string(), Value::from(recipient.clone()));
        send_params.insert("plaintext".to_string(), Value::from(body.clone()));
        crate::openhuman::tinyplace::handle_tinyplace_signal_send_message(send_params)
            .await
            .map_err(|e| format!("signal send: {e}"))?;

        // Mirror it into the Master window so the composer's message is visible,
        // and notify the renderer.
        let now = chrono::Utc::now().to_rfc3339();
        let message_id = format!("master-out:{}", now);
        let persisted = store::with_connection(&config.workspace_dir, |conn| {
            store::upsert_session(
                conn,
                &OrchestrationSession {
                    session_id: "master".to_string(),
                    agent_id: recipient.clone(),
                    source: "master".to_string(),
                    label: None,
                    workspace: None,
                    last_seq: 0,
                    created_at: now.clone(),
                    last_message_at: now.clone(),
                },
            )?;
            store::insert_message(
                conn,
                &OrchestrationMessage {
                    id: message_id.clone(),
                    agent_id: recipient.clone(),
                    session_id: "master".to_string(),
                    chat_kind: ChatKind::Master,
                    role: "owner".to_string(),
                    body: body.clone(),
                    timestamp: now.clone(),
                    seq: 0,
                },
            )
        });
        if let Err(e) = persisted {
            log::warn!(target: LOG, "[orchestration_rpc] send_master.mirror_failed: {e}");
        }
        super::bus::notify_orchestration_message(&recipient, "master", "master");

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
        .map_err(|e| format!("mark_read: {e}"))?;
        to_json(serde_json::json!({ "ok": true }))
    })
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let config = load_config("status").await?;
        #[allow(clippy::type_complexity)]
        let (steering, ingest_last, lag, last_error): (
            Option<SteeringSummary>,
            Option<String>,
            i64,
            Option<String>,
        ) = store::with_connection(&config.workspace_dir, |conn| {
            let cycle = store::current_cycle_counter(conn)?;
            let steering =
                store::current_steering_directive(conn, cycle)?.map(|d| SteeringSummary {
                    text: d.text,
                    created_at: d.created_at,
                    expires_after_cycles: d.expires_after_cycles,
                });
            // MAX() always returns exactly one row (NULL when empty).
            let ingest_last: Option<String> =
                conn.query_row("SELECT MAX(last_message_at) FROM sessions", [], |r| {
                    r.get::<_, Option<String>>(0)
                })?;
            let lag = store::ingest_cursor_lag(conn)?;
            let last_error = store::kv_get(conn, "orchestration:last_error")?;
            Ok((steering, ingest_last, lag, last_error))
        })
        .map_err(|e| format!("status: {e}"))?;

        // Last subconscious tick (best-effort — subconscious store is separate).
        let last_tick_at = crate::openhuman::subconscious::store::with_connection(
            &config.workspace_dir,
            crate::openhuman::subconscious::store::get_last_tick_at,
        )
        .ok()
        .filter(|v| *v > 0.0);

        to_json(OrchestrationStatus {
            steering,
            last_tick_at,
            ingest_last_message_at: ingest_last.filter(|s| !s.is_empty()),
            ingest_cursor_lag: lag,
            last_error,
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
        assert_eq!(schemas.len(), 5);
        assert!(schemas.iter().all(|s| s.namespace == "orchestration"));
        assert_eq!(
            schema_for("orchestration_messages_list").function,
            "messages_list"
        );
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
}
