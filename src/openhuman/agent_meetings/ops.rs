//! RPC handlers for the `agent_meetings` domain.
//!
//! Each handler emits a Socket.IO event to the backend via the global
//! `SocketManager`. The backend's meeting bot handler picks these up and
//! drives the Recall.ai (or Camoufox) session.

use serde_json::{json, Map, Value};

use crate::openhuman::meet::ops::validate_display_name;
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

    mgr.emit(
        "bot:join",
        json!({
            "meetUrl": normalized_url.as_str(),
            "displayName": display_name,
            "platform": platform,
        }),
    )
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
}
