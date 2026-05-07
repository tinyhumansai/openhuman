//! JSON-RPC handler for the `meet` domain.
//!
//! `openhuman.meet_join_call` validates the request, mints a `request_id`,
//! and returns a normalized echo. Opening the actual CEF webview window
//! happens on the Tauri shell side, keyed by `request_id`. Keeping the
//! RPC narrow lets the core stay platform-agnostic and lets future
//! callers (CLI, scripts, RPC tests) reach the validation layer without
//! pulling in the desktop shell.

use serde_json::{json, Map, Value};
use uuid::Uuid;

use crate::rpc::RpcOutcome;

use super::ops;
use super::types::MeetJoinCallRequest;

/// Handle `openhuman.meet_join_call`.
pub async fn handle_join_call(params: Map<String, Value>) -> Result<Value, String> {
    let req: MeetJoinCallRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("[meet] invalid join_call params: {e}"))?;

    let normalized_url =
        ops::validate_meet_url(&req.meet_url).map_err(|e| format!("[meet] {e}"))?;
    let display_name =
        ops::validate_display_name(&req.display_name).map_err(|e| format!("[meet] {e}"))?;

    let request_id = Uuid::new_v4().to_string();
    tracing::info!(
        request_id = %request_id,
        host = %normalized_url.host_str().unwrap_or(""),
        path = %normalized_url.path(),
        display_name_chars = display_name.chars().count(),
        "[meet] meet_join_call accepted; awaiting shell handoff"
    );

    let outcome = RpcOutcome::new(
        json!({
            "ok": true,
            "request_id": request_id,
            "meet_url": normalized_url.as_str(),
            "display_name": display_name,
        }),
        vec![],
    );
    outcome.into_cli_compatible_json()
}
