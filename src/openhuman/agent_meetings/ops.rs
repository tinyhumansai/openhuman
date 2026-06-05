//! RPC handlers for the `agent_meetings` domain.
//!
//! Each handler emits a Socket.IO event to the backend via the global
//! `SocketManager`. The backend's meeting bot handler picks these up and
//! drives the Recall.ai (or Camoufox) session.

use serde_json::{json, Map, Value};

use crate::core::event_bus::BackendMeetTurn;
use crate::openhuman::meet::ops::validate_display_name;
use crate::openhuman::memory::ingest_pipeline;
use crate::openhuman::memory_sync::canonicalize::chat::{ChatBatch, ChatMessage};
use crate::openhuman::socket::global_socket_manager;
use crate::rpc::RpcOutcome;

use super::types::{
    BackendMeetHarnessResponseRequest, BackendMeetJoinRequest, BackendMeetJoinResponse,
    BackendMeetLeaveRequest,
};

const ALLOWED_HOSTS: &[(&str, &str)] = &[
    ("meet.google.com", "gmeet"),
    ("zoom.us", "zoom"),
    ("teams.microsoft.com", "teams"),
    ("webex.com", "webex"),
];

fn transcript_turns_to_chat_batch(
    turns: &[BackendMeetTurn],
    duration_ms: u64,
) -> Option<ChatBatch> {
    // Cap at 48 h to avoid DateTime underflow; real meetings never exceed this.
    const MAX_DURATION_MS: u64 = 172_800_000;
    let duration_i64 = i64::try_from(duration_ms.min(MAX_DURATION_MS)).unwrap_or(172_800_000);
    let base = chrono::Utc::now() - chrono::Duration::milliseconds(duration_i64);
    // Spread turns evenly across the duration; fall back to 1 ms spacing when
    // duration is zero or turns is empty (avoids division by zero).
    let spacing_ms = if turns.is_empty() {
        1i64
    } else {
        i64::try_from(duration_ms / turns.len() as u64).unwrap_or(1)
    };
    let mut messages = Vec::new();

    for (idx, turn) in turns.iter().enumerate() {
        let text = turn.content.trim();
        if text.is_empty() {
            continue;
        }
        let author = if turn.role.eq_ignore_ascii_case("assistant") {
            "OpenHuman"
        } else {
            "Meeting participant"
        };
        let offset_ms = spacing_ms.saturating_mul(idx as i64);
        messages.push(ChatMessage {
            author: author.to_string(),
            timestamp: base + chrono::Duration::milliseconds(offset_ms),
            text: text.to_string(),
            source_ref: Some(format!("backend-meet://turn/{idx}")),
        });
    }

    if messages.is_empty() {
        None
    } else {
        Some(ChatBatch {
            platform: "backend_meet".to_string(),
            channel_label: "Recall AI meeting".to_string(),
            messages,
        })
    }
}

pub async fn ingest_backend_meeting_transcript(
    turns: Vec<BackendMeetTurn>,
    duration_ms: u64,
) -> Result<(), String> {
    let Some(batch) = transcript_turns_to_chat_batch(&turns, duration_ms) else {
        tracing::debug!("[agent_meetings] transcript had no ingestible turns");
        return Ok(());
    };

    let config = crate::openhuman::config::Config::load_or_init()
        .await
        .map_err(|e| format!("[agent_meetings] config load failed: {e}"))?;
    let source_id = format!("meet:recall:{}", chrono::Utc::now().timestamp_millis());
    let tags = vec!["meeting".to_string(), "recall_ai".to_string()];
    let result = ingest_pipeline::ingest_chat(&config, &source_id, "user", tags, batch)
        .await
        .map_err(|e| format!("[agent_meetings] transcript ingest failed: {e:#}"))?;

    tracing::info!(
        source_id = %source_id,
        chunks_written = result.chunks_written,
        "[agent_meetings] transcript ingested into memory tree"
    );
    Ok(())
}

fn validate_meeting_url(raw: &str) -> Result<url::Url, String> {
    let url = url::Url::parse(raw.trim()).map_err(|e| format!("invalid meeting URL: {e}"))?;

    if url.scheme() != "https" && url.scheme() != "http" {
        return Err(format!(
            "invalid meeting URL: scheme `{}` not allowed",
            url.scheme()
        ));
    }

    let host = url
        .host_str()
        .ok_or_else(|| "invalid meeting URL: missing host".to_string())?;

    let is_allowed = ALLOWED_HOSTS
        .iter()
        .any(|(allowed, _)| host == *allowed || host.ends_with(&format!(".{allowed}")));

    if !is_allowed {
        return Err(format!(
            "invalid meeting URL: host `{host}` not recognized (supported: Google Meet, Zoom, Teams, Webex)"
        ));
    }

    Ok(url)
}

fn infer_platform(url: &url::Url) -> &'static str {
    let host = url.host_str().unwrap_or("");
    for (allowed, platform) in ALLOWED_HOSTS {
        if host == *allowed || host.ends_with(&format!(".{allowed}")) {
            return platform;
        }
    }
    "gmeet"
}

/// Build the `bot:join` Socket.IO payload from a validated request.
///
/// Extracted as a pure function so it can be unit-tested independently of the
/// live socket connection.
fn build_join_payload(
    meet_url: &str,
    display_name: &str,
    platform: &str,
    req: &BackendMeetJoinRequest,
) -> Value {
    let mut payload = json!({
        "meetUrl": meet_url,
        "displayName": display_name,
        "platform": platform,
    });
    if let Some(map) = payload.as_object_mut() {
        if let Some(agent_name) = &req.agent_name {
            map.insert("agentName".to_string(), json!(agent_name));
        }
        if let Some(system_prompt) = &req.system_prompt {
            map.insert("systemPrompt".to_string(), json!(system_prompt));
        }
        if let Some(mascot_id) = &req.mascot_id {
            map.insert("mascotId".to_string(), json!(mascot_id));
        }
        if let Some(rive_colors) = &req.rive_colors {
            map.insert(
                "riveColors".to_string(),
                json!({
                    "primaryColor": rive_colors.primary_color,
                    "secondaryColor": rive_colors.secondary_color,
                }),
            );
        }
        if let Some(respond_to) = &req.respond_to_participant {
            map.insert("respondToParticipant".to_string(), json!(respond_to));
        }
        if let Some(phrase) = &req.wake_phrase {
            map.insert("wakePhrase".to_string(), json!(phrase));
        }
    }
    payload
}

/// Handle `openhuman.agent_meetings_join`.
pub async fn handle_join(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetJoinRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid join params: {e}"))?;

    let normalized_url =
        validate_meeting_url(&req.meet_url).map_err(|e| format!("[agent_meetings] {e}"))?;

    let display_name = match &req.display_name {
        Some(name) => validate_display_name(name).map_err(|e| format!("[agent_meetings] {e}"))?,
        None => "OpenHuman".to_string(),
    };

    let inferred = infer_platform(&normalized_url);
    let platform = match req.platform.as_deref() {
        Some(p) if p != inferred => {
            return Err(format!(
                "[agent_meetings] platform mismatch: URL implies `{inferred}` but `{p}` was supplied"
            ));
        }
        Some(p) => p,
        None => inferred,
    };

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        meet_url_host = %normalized_url.host_str().unwrap_or(""),
        platform = %platform,
        display_name_len = display_name.len(),
        "[agent_meetings] emitting bot:join"
    );

    let join_payload = build_join_payload(normalized_url.as_str(), &display_name, platform, &req);

    mgr.emit("bot:join", join_payload)
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let response = BackendMeetJoinResponse {
        ok: true,
        meet_url: normalized_url.to_string(),
        platform: platform.to_string(),
    };
    let outcome = RpcOutcome::new(
        serde_json::to_value(response).map_err(|e| format!("[agent_meetings] serialize: {e}"))?,
        vec![],
    );
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_leave`.
pub async fn handle_leave(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetLeaveRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid leave params: {e}"))?;

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    let reason = req.reason.unwrap_or_else(|| "requested".to_string());

    tracing::info!(reason = %reason, "[agent_meetings] emitting bot:leave");

    mgr.emit("bot:leave", json!({ "reason": reason }))
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

/// Handle `openhuman.agent_meetings_harness_response`.
pub async fn handle_harness_response(params: Map<String, Value>) -> Result<Value, String> {
    let req: BackendMeetHarnessResponseRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[agent_meetings] invalid harness_response params: {e}"))?;

    if req.result.trim().is_empty() {
        return Err("[agent_meetings] result must not be empty".to_string());
    }

    let mgr = global_socket_manager()
        .ok_or_else(|| "[agent_meetings] socket not connected to backend".to_string())?;

    if !mgr.is_connected() {
        return Err("[agent_meetings] socket not connected to backend".to_string());
    }

    tracing::info!(
        result_len = req.result.len(),
        "[agent_meetings] emitting bot:harness:response"
    );

    mgr.emit("bot:harness:response", json!({ "result": req.result }))
        .await
        .map_err(|e| format!("[agent_meetings] emit failed: {e}"))?;

    let outcome = RpcOutcome::new(json!({ "ok": true }), vec![]);
    outcome.into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_google_meet_url() {
        validate_meeting_url("https://meet.google.com/abc-defg-hij").unwrap();
    }

    #[test]
    fn accepts_zoom_url() {
        validate_meeting_url("https://zoom.us/j/123456789").unwrap();
        validate_meeting_url("https://company.zoom.us/j/123456789").unwrap();
    }

    #[test]
    fn accepts_teams_url() {
        validate_meeting_url("https://teams.microsoft.com/l/meetup-join/abc").unwrap();
    }

    #[test]
    fn accepts_webex_url() {
        validate_meeting_url("https://meet.webex.com/meet/abc").unwrap();
        validate_meeting_url("https://company.webex.com/meet/abc").unwrap();
    }

    #[test]
    fn rejects_unknown_host() {
        assert!(validate_meeting_url("https://example.com/meeting").is_err());
    }

    #[test]
    fn infers_platform_from_host() {
        let url = url::Url::parse("https://meet.google.com/abc-defg-hij").unwrap();
        assert_eq!(infer_platform(&url), "gmeet");

        let url = url::Url::parse("https://zoom.us/j/123").unwrap();
        assert_eq!(infer_platform(&url), "zoom");

        let url = url::Url::parse("https://teams.microsoft.com/l/meetup").unwrap();
        assert_eq!(infer_platform(&url), "teams");

        let url = url::Url::parse("https://meet.webex.com/meet/abc").unwrap();
        assert_eq!(infer_platform(&url), "webex");

        let url = url::Url::parse("https://company.zoom.us/j/123").unwrap();
        assert_eq!(infer_platform(&url), "zoom");
    }

    #[test]
    fn transcript_turns_convert_to_chat_batch() {
        let batch = transcript_turns_to_chat_batch(
            &[
                BackendMeetTurn {
                    role: "user".to_string(),
                    content: "[Alice] OpenHuman, summarize this.".to_string(),
                },
                BackendMeetTurn {
                    role: "assistant".to_string(),
                    content: "Sure, here is the summary.".to_string(),
                },
            ],
            1_000,
        )
        .expect("batch");

        assert_eq!(batch.platform, "backend_meet");
        assert_eq!(batch.messages.len(), 2);
        assert_eq!(batch.messages[0].author, "Meeting participant");
        assert_eq!(batch.messages[1].author, "OpenHuman");
        assert!(batch.messages[0].text.contains("summarize"));
    }

    #[tokio::test]
    async fn join_fails_when_socket_not_connected() {
        let params: Map<String, Value> =
            serde_json::from_value(json!({"meet_url": "https://meet.google.com/abc-defg-hij"}))
                .unwrap();
        let result = handle_join(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("socket not connected"));
    }

    #[tokio::test]
    async fn harness_response_rejects_empty_result() {
        let params: Map<String, Value> = serde_json::from_value(json!({"result": "   "})).unwrap();
        let result = handle_harness_response(params).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must not be empty"));
    }

    // --- build_join_payload ---

    fn minimal_req(meet_url: &str) -> BackendMeetJoinRequest {
        serde_json::from_value(json!({ "meet_url": meet_url })).unwrap()
    }

    #[test]
    fn build_join_payload_minimal() {
        let req = minimal_req("https://meet.google.com/abc-defg-hij");
        let payload = build_join_payload(
            "https://meet.google.com/abc-defg-hij",
            "OpenHuman",
            "gmeet",
            &req,
        );
        assert_eq!(payload["meetUrl"], "https://meet.google.com/abc-defg-hij");
        assert_eq!(payload["displayName"], "OpenHuman");
        assert_eq!(payload["platform"], "gmeet");
        assert!(payload.get("agentName").is_none());
        assert!(payload.get("systemPrompt").is_none());
        assert!(payload.get("mascotId").is_none());
        assert!(payload.get("riveColors").is_none());
        assert!(payload.get("respondToParticipant").is_none());
        assert!(payload.get("wakePhrase").is_none());
    }

    #[test]
    fn build_join_payload_with_respond_to_participant() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://zoom.us/j/123",
            "respond_to_participant": "Alice"
        }))
        .unwrap();
        let payload = build_join_payload("https://zoom.us/j/123", "Bot", "zoom", &req);
        assert_eq!(payload["respondToParticipant"], "Alice");
        assert!(payload.get("wakePhrase").is_none());
    }

    #[test]
    fn build_join_payload_with_wake_phrase() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://zoom.us/j/123",
            "wake_phrase": "Hey bot"
        }))
        .unwrap();
        let payload = build_join_payload("https://zoom.us/j/123", "Bot", "zoom", &req);
        assert_eq!(payload["wakePhrase"], "Hey bot");
        assert!(payload.get("respondToParticipant").is_none());
    }

    #[test]
    fn build_join_payload_with_all_optional_fields() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://teams.microsoft.com/l/meet/abc",
            "agent_name": "MyBot",
            "system_prompt": "You are a helpful assistant.",
            "mascot_id": "yellow",
            "rive_colors": {
                "primary_color": "#ff0000",
                "secondary_color": "#00ff00"
            },
            "respond_to_participant": "Bob",
            "wake_phrase": "Hello bot"
        }))
        .unwrap();
        let payload = build_join_payload(
            "https://teams.microsoft.com/l/meet/abc",
            "MyBot",
            "teams",
            &req,
        );
        assert_eq!(payload["agentName"], "MyBot");
        assert_eq!(payload["systemPrompt"], "You are a helpful assistant.");
        assert_eq!(payload["mascotId"], "yellow");
        assert_eq!(payload["riveColors"]["primaryColor"], "#ff0000");
        assert_eq!(payload["riveColors"]["secondaryColor"], "#00ff00");
        assert_eq!(payload["respondToParticipant"], "Bob");
        assert_eq!(payload["wakePhrase"], "Hello bot");
    }

    #[test]
    fn join_request_fields_deserialize_correctly() {
        let req: BackendMeetJoinRequest = serde_json::from_value(json!({
            "meet_url": "https://meet.google.com/abc-defg-hij",
            "respond_to_participant": "Alice",
            "wake_phrase": "Hey bot"
        }))
        .unwrap();
        assert_eq!(req.respond_to_participant.as_deref(), Some("Alice"));
        assert_eq!(req.wake_phrase.as_deref(), Some("Hey bot"));
    }

    #[test]
    fn join_request_optional_fields_absent_by_default() {
        let req: BackendMeetJoinRequest =
            serde_json::from_value(json!({ "meet_url": "https://meet.google.com/abc-defg-hij" }))
                .unwrap();
        assert!(req.respond_to_participant.is_none());
        assert!(req.wake_phrase.is_none());
        assert!(req.agent_name.is_none());
        assert!(req.system_prompt.is_none());
        assert!(req.mascot_id.is_none());
        assert!(req.rive_colors.is_none());
    }

    #[test]
    fn transcript_turns_empty_returns_none() {
        let result = transcript_turns_to_chat_batch(&[], 1_000);
        assert!(result.is_none());
    }

    #[test]
    fn transcript_turns_all_blank_content_returns_none() {
        let result = transcript_turns_to_chat_batch(
            &[BackendMeetTurn {
                role: "user".to_string(),
                content: "   ".to_string(),
            }],
            1_000,
        );
        assert!(result.is_none());
    }

    #[test]
    fn transcript_turns_zero_duration_no_panic() {
        let batch = transcript_turns_to_chat_batch(
            &[BackendMeetTurn {
                role: "user".to_string(),
                content: "hello".to_string(),
            }],
            0,
        )
        .expect("batch");
        assert_eq!(batch.messages.len(), 1);
    }

    #[test]
    fn rive_colors_deserialize() {
        use crate::openhuman::agent_meetings::types::RiveColors;
        let rc: RiveColors =
            serde_json::from_value(json!({"primary_color": "#abc", "secondary_color": "#def"}))
                .unwrap();
        assert_eq!(rc.primary_color.as_deref(), Some("#abc"));
        assert_eq!(rc.secondary_color.as_deref(), Some("#def"));
    }
}
