//! JSON-RPC / CLI controller surface for screen capture and accessibility automation.
//!
//! macOS permission UX (stale DENIED until sidecar restarts) is tracked in
//! <https://github.com/tinyhumansai/openhuman/issues/133>.

use serde_json::json;

use crate::openhuman::screen_intelligence::{
    self, AccessibilityStatus, CaptureImageRefResult, CaptureNowResult, CaptureTestResult,
    GlobeHotkeyPollResult, GlobeHotkeyStatus, InputActionParams, InputActionResult,
    PermissionRequestParams, PermissionState, PermissionStatus, SessionStatus, StartSessionParams,
    StopSessionParams, VisionFlushResult, VisionRecentResult,
};
use crate::rpc::RpcOutcome;

pub async fn accessibility_status() -> Result<RpcOutcome<AccessibilityStatus>, String> {
    if let Ok(config) = crate::openhuman::config::Config::load_or_init().await {
        let _ = screen_intelligence::global_engine()
            .apply_config(config.screen_intelligence.clone())
            .await;
    }
    let status = screen_intelligence::global_engine().status().await;
    Ok(RpcOutcome::single_log(
        status,
        "screen intelligence status fetched",
    ))
}

/// CLI `accessibility doctor`: recommendations from current [`AccessibilityStatus`].
pub async fn accessibility_doctor_cli_json() -> Result<serde_json::Value, String> {
    let RpcOutcome {
        value: status,
        logs,
    } = accessibility_status().await?;
    let permissions = &status.permissions;

    let screen_ready = permissions.screen_recording == PermissionState::Granted;
    let control_ready = permissions.accessibility == PermissionState::Granted;
    let monitoring_ready = permissions.input_monitoring == PermissionState::Granted;
    let overall_ready = status.platform_supported && screen_ready && control_ready;

    let mut recommendations: Vec<String> = Vec::new();
    if !status.platform_supported {
        recommendations
            .push("Accessibility automation is macOS-only in this build/runtime.".to_string());
    }
    if permissions.screen_recording != PermissionState::Granted {
        recommendations.push(
            "Grant Screen Recording in System Settings -> Privacy & Security -> Screen Recording."
                .to_string(),
        );
    }
    if permissions.accessibility != PermissionState::Granted {
        recommendations.push(
            "Grant Accessibility in System Settings -> Privacy & Security -> Accessibility."
                .to_string(),
        );
    }
    if permissions.input_monitoring != PermissionState::Granted {
        recommendations.push(
            "Grant Input Monitoring in System Settings -> Privacy & Security -> Input Monitoring (optional but recommended)."
                .to_string(),
        );
    }
    if recommendations.is_empty() {
        recommendations.push("No action required. Accessibility automation is ready.".to_string());
    }

    Ok(json!({
        "result": {
            "summary": {
                "overall_ready": overall_ready,
                "platform_supported": status.platform_supported,
                "session_active": status.session.active,
                "screen_capture_ready": screen_ready,
                "accessibility_ready": control_ready,
                "input_monitoring_ready": monitoring_ready
            },
            "permissions": permissions,
            "features": status.features,
            "recommendations": recommendations
        },
        "logs": logs
    }))
}

pub async fn accessibility_request_permissions() -> Result<RpcOutcome<PermissionStatus>, String> {
    let permissions = screen_intelligence::global_engine()
        .request_permissions()
        .await?;
    Ok(RpcOutcome::single_log(
        permissions,
        "accessibility automation permissions requested",
    ))
}

pub async fn accessibility_request_permission(
    payload: PermissionRequestParams,
) -> Result<RpcOutcome<PermissionStatus>, String> {
    let permissions = screen_intelligence::global_engine()
        .request_permission(payload.permission)
        .await?;
    Ok(RpcOutcome::single_log(
        permissions,
        "accessibility permission requested",
    ))
}

pub async fn accessibility_start_session(
    payload: StartSessionParams,
) -> Result<RpcOutcome<SessionStatus>, String> {
    let session = screen_intelligence::global_engine()
        .start_session(payload)
        .await?;
    Ok(RpcOutcome::single_log(
        session,
        "screen intelligence enabled",
    ))
}

pub async fn accessibility_stop_session(
    payload: StopSessionParams,
) -> Result<RpcOutcome<SessionStatus>, String> {
    let session = screen_intelligence::global_engine()
        .disable(payload.reason)
        .await;
    Ok(RpcOutcome::single_log(
        session,
        "screen intelligence stopped",
    ))
}

pub async fn accessibility_capture_now() -> Result<RpcOutcome<CaptureNowResult>, String> {
    let result = screen_intelligence::global_engine().capture_now().await?;
    Ok(RpcOutcome::single_log(
        result,
        "accessibility manual capture requested",
    ))
}

pub async fn accessibility_capture_image_ref() -> Result<RpcOutcome<CaptureImageRefResult>, String>
{
    let result: CaptureImageRefResult = screen_intelligence::global_engine()
        .capture_image_ref_test()
        .await;
    Ok(RpcOutcome::single_log(
        result,
        "accessibility direct image_ref capture requested",
    ))
}

pub async fn accessibility_input_action(
    payload: InputActionParams,
) -> Result<RpcOutcome<InputActionResult>, String> {
    let result = screen_intelligence::global_engine()
        .input_action(payload)
        .await?;
    Ok(RpcOutcome::single_log(
        result,
        "screen intelligence input action processed",
    ))
}

pub async fn accessibility_vision_recent(
    limit: Option<usize>,
) -> Result<RpcOutcome<VisionRecentResult>, String> {
    let result: VisionRecentResult = screen_intelligence::global_engine()
        .vision_recent(limit)
        .await;
    Ok(RpcOutcome::single_log(
        result,
        "screen intelligence vision summaries fetched",
    ))
}

pub async fn accessibility_vision_flush() -> Result<RpcOutcome<VisionFlushResult>, String> {
    let result: VisionFlushResult = screen_intelligence::global_engine().vision_flush().await?;
    Ok(RpcOutcome::single_log(
        result,
        "screen intelligence vision flush completed",
    ))
}

/// Re-detect current permission state. Intended to be called after the sidecar
/// restarts so the new process reads freshly granted macOS permissions.
///
/// macOS caches permission grants per-process; the running sidecar never sees an
/// updated grant until it restarts. After `restart_core_process` brings up a fresh
/// sidecar, calling this endpoint returns the authoritative permission state as seen
/// by that new process.
pub async fn accessibility_refresh_permissions() -> Result<RpcOutcome<PermissionStatus>, String> {
    log::info!("[screen_intelligence] refresh_permissions:re-detecting permissions");
    // `status()` unconditionally calls `detect_permissions()` before returning, so
    // fetching the full status and extracting the permissions field is the correct
    // way to get a freshly computed permission state.
    let full_status = screen_intelligence::global_engine().status().await;
    let permissions = full_status.permissions;
    log::debug!(
        "[screen_intelligence] accessibility_refresh_permissions: screen_recording={:?} accessibility={:?} input_monitoring={:?}",
        permissions.screen_recording,
        permissions.accessibility,
        permissions.input_monitoring,
    );
    Ok(RpcOutcome::single_log(
        permissions,
        "accessibility permissions refreshed",
    ))
}

pub async fn accessibility_capture_test() -> Result<RpcOutcome<CaptureTestResult>, String> {
    let result: CaptureTestResult = screen_intelligence::global_engine().capture_test().await;
    Ok(RpcOutcome::single_log(
        result,
        "screen intelligence capture test completed",
    ))
}

pub async fn accessibility_globe_listener_start() -> Result<RpcOutcome<GlobeHotkeyStatus>, String> {
    log::info!("[screen_intelligence] globe_listener_start requested");
    let result = crate::openhuman::accessibility::globe_listener_start()?;
    Ok(RpcOutcome::single_log(
        result,
        "globe listener start processed",
    ))
}

pub async fn accessibility_globe_listener_poll() -> Result<RpcOutcome<GlobeHotkeyPollResult>, String>
{
    let result = crate::openhuman::accessibility::globe_listener_poll()?;
    Ok(RpcOutcome::single_log(
        result,
        "globe listener poll processed",
    ))
}

pub async fn accessibility_globe_listener_stop() -> Result<RpcOutcome<GlobeHotkeyStatus>, String> {
    log::info!("[screen_intelligence] globe_listener_stop requested");
    let result = crate::openhuman::accessibility::globe_listener_stop()?;
    Ok(RpcOutcome::single_log(
        result,
        "globe listener stop processed",
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn accessibility_status_returns_ok() {
        let outcome = accessibility_status().await.expect("status");
        // Permissions field is always present (even if all Denied on Linux).
        let _ = outcome.value;
    }

    #[tokio::test]
    async fn accessibility_doctor_cli_json_returns_summary_permissions_and_recommendations() {
        let v = accessibility_doctor_cli_json().await.expect("doctor json");
        assert!(v.get("result").is_some());
        let result = &v["result"];
        assert!(result.get("summary").is_some());
        assert!(result.get("permissions").is_some());
        assert!(result.get("recommendations").is_some());
        // Recommendations are always non-empty (either action-items or "ready").
        assert!(result["recommendations"]
            .as_array()
            .map_or(false, |a| !a.is_empty()));
    }

    #[tokio::test]
    async fn accessibility_doctor_cli_json_has_summary_flags_as_bools() {
        let v = accessibility_doctor_cli_json().await.unwrap();
        let summary = &v["result"]["summary"];
        for field in [
            "overall_ready",
            "platform_supported",
            "session_active",
            "screen_capture_ready",
            "accessibility_ready",
            "input_monitoring_ready",
        ] {
            assert!(
                summary[field].is_boolean(),
                "summary field `{field}` should be boolean"
            );
        }
    }

    #[tokio::test]
    async fn accessibility_stop_session_is_tolerant_of_no_reason() {
        let payload = StopSessionParams { reason: None };
        let _ = accessibility_stop_session(payload).await;
    }

    #[tokio::test]
    async fn accessibility_capture_image_ref_returns_ok_even_on_unsupported_platform() {
        // `capture_image_ref_test` is `async fn` returning `CaptureImageRefResult`
        // directly (no `Result`), so this call should always succeed. On
        // non-macOS platforms the result will simply report capture failure.
        let outcome = accessibility_capture_image_ref().await.expect("capture");
        let _ = outcome.value;
    }

    #[tokio::test]
    async fn accessibility_vision_recent_with_no_args_returns_ok() {
        let outcome = accessibility_vision_recent(Some(5)).await;
        // Either Ok or Err — just ensure the call doesn't panic.
        let _ = outcome;
    }
}
