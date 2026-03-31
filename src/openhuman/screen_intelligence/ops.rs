//! JSON-RPC / CLI controller surface for screen capture and accessibility automation.

use serde_json::json;

use crate::openhuman::screen_intelligence::{
    self, AccessibilityStatus, CaptureImageRefResult, CaptureNowResult, CaptureTestResult,
    InputActionParams, InputActionResult, PermissionRequestParams, PermissionState,
    PermissionStatus, SessionStatus, StartSessionParams, StopSessionParams, VisionFlushResult,
    VisionRecentResult,
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
                "device_control_ready": control_ready,
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
        "accessibility permissions requested",
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

pub async fn accessibility_capture_test() -> Result<RpcOutcome<CaptureTestResult>, String> {
    let result: CaptureTestResult = screen_intelligence::global_engine().capture_test().await;
    Ok(RpcOutcome::single_log(
        result,
        "screen intelligence capture test completed",
    ))
}
