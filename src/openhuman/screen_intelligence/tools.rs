//! LLM-callable wrappers over the `screen_intelligence` (accessibility) domain.
//!
//! These let the agent observe and drive the desktop: status, capture sessions,
//! single captures, input automation, recent vision summaries, and the
//! Globe/Fn hotkey listener. All delegate to the arg-less / payload functions
//! in [`crate::openhuman::screen_intelligence::ops`] (which read the global
//! engine + load config internally).
//!
//! Observation + capture/input tools are default-ON (capture requires a session
//! the user started with consent). The OS permission-request tools —
//! `screen_intelligence_request_permissions` / `_request_permission` — trigger
//! system permission dialogs, so they are `Dangerous` and ship default-OFF via
//! `tools/user_filter.rs` (`screen_permissions` toggle).

use async_trait::async_trait;
use serde_json::json;

use crate::openhuman::screen_intelligence::ops;
use crate::openhuman::screen_intelligence::types::{
    InputActionParams, PermissionRequestParams, StartSessionParams, StopSessionParams,
};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};

macro_rules! emit {
    ($outcome:expr, $name:literal) => {{
        let outcome = $outcome.map_err(|e| anyhow::anyhow!(concat!($name, ": {}"), e))?;
        Ok(ToolResult::success(serde_json::to_string(&outcome.value)?))
    }};
}

/// Arg-less SI tool at a given permission level.
macro_rules! argless_tool {
    ($ty:ident, $name:literal, $fn:ident, $perm:expr, $conc:expr, $desc:literal) => {
        pub struct $ty;
        #[async_trait]
        impl Tool for $ty {
            fn name(&self) -> &str {
                $name
            }
            fn description(&self) -> &str {
                $desc
            }
            fn parameters_schema(&self) -> serde_json::Value {
                json!({ "type": "object", "properties": {} })
            }
            fn permission_level(&self) -> PermissionLevel {
                $perm
            }
            async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
                log::debug!(concat!("[tool][screen_intelligence] ", $name, " invoked"));
                emit!(ops::$fn().await, $name)
            }
            fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
                $conc
            }
        }
    };
}

argless_tool!(
    ScreenStatusTool,
    "screen_intelligence_status",
    accessibility_status,
    PermissionLevel::ReadOnly,
    true,
    "Report screen-intelligence status: granted permissions, active session, and platform support."
);
argless_tool!(
    ScreenCaptureImageRefTool,
    "screen_intelligence_capture_image_ref",
    accessibility_capture_image_ref,
    PermissionLevel::ReadOnly,
    false,
    "Capture the current screen and return an image reference (no inline bytes)."
);
argless_tool!(
    ScreenVisionFlushTool,
    "screen_intelligence_vision_flush",
    accessibility_vision_flush,
    PermissionLevel::Execute,
    false,
    "Clear the cached recent-vision summaries."
);
argless_tool!(
    ScreenRefreshPermissionsTool,
    "screen_intelligence_refresh_permissions",
    accessibility_refresh_permissions,
    PermissionLevel::ReadOnly,
    false,
    "Re-detect current OS permission grants (does not prompt)."
);
argless_tool!(
    ScreenCaptureNowTool,
    "screen_intelligence_capture_now",
    accessibility_capture_now,
    PermissionLevel::Execute,
    false,
    "Capture a frame now within the active session and return its image reference."
);
argless_tool!(
    ScreenCaptureTestTool,
    "screen_intelligence_capture_test",
    accessibility_capture_test,
    PermissionLevel::Execute,
    false,
    "Run a standalone capture diagnostic (no session required)."
);
argless_tool!(
    ScreenGlobeStartTool,
    "screen_intelligence_globe_listener_start",
    accessibility_globe_listener_start,
    PermissionLevel::Execute,
    false,
    "Start the Globe/Fn hotkey listener."
);
argless_tool!(
    ScreenGlobePollTool,
    "screen_intelligence_globe_listener_poll",
    accessibility_globe_listener_poll,
    PermissionLevel::ReadOnly,
    true,
    "Poll for Globe/Fn hotkey events since the last poll."
);
argless_tool!(
    ScreenGlobeStopTool,
    "screen_intelligence_globe_listener_stop",
    accessibility_globe_listener_stop,
    PermissionLevel::Execute,
    false,
    "Stop the Globe/Fn hotkey listener."
);

/// Start a capture session (requires explicit consent).
pub struct ScreenSessionStartTool;

#[async_trait]
impl Tool for ScreenSessionStartTool {
    fn name(&self) -> &str {
        "screen_intelligence_session_start"
    }

    fn description(&self) -> &str {
        "Start a screen-capture session. Requires `consent: true`; optional \
         `ttl_secs` and `screen_monitoring`. Capture tools only work while a \
         session is active."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "consent": { "type": "boolean", "description": "Explicit user consent to capture (required true)." },
                "ttl_secs": { "type": "integer", "minimum": 1 },
                "screen_monitoring": { "type": "boolean" }
            },
            "required": ["consent"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][screen_intelligence] session_start invoked");
        let payload: StartSessionParams = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("screen_intelligence_session_start: invalid args: {e}"))?;
        emit!(
            ops::accessibility_start_session(payload).await,
            "screen_intelligence_session_start"
        )
    }
}

/// Stop the capture session.
pub struct ScreenSessionStopTool;

#[async_trait]
impl Tool for ScreenSessionStopTool {
    fn name(&self) -> &str {
        "screen_intelligence_session_stop"
    }

    fn description(&self) -> &str {
        "Stop the active screen-capture session, with an optional `reason`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": { "reason": { "type": "string" } } })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][screen_intelligence] session_stop invoked");
        let payload: StopSessionParams = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("screen_intelligence_session_stop: invalid args: {e}"))?;
        emit!(
            ops::accessibility_stop_session(payload).await,
            "screen_intelligence_session_stop"
        )
    }
}

/// Drive a click/type/key input action.
pub struct ScreenInputActionTool;

#[async_trait]
impl Tool for ScreenInputActionTool {
    fn name(&self) -> &str {
        "screen_intelligence_input_action"
    }

    fn description(&self) -> &str {
        "Perform a desktop input action (click / type / key) via the \
         accessibility engine. Takes an `InputActionParams` object."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "description": "InputActionParams: the action kind plus its coordinates/text/keys.",
            "additionalProperties": true
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Execute
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][screen_intelligence] input_action invoked");
        let payload: InputActionParams = serde_json::from_value(args)
            .map_err(|e| anyhow::anyhow!("screen_intelligence_input_action: invalid args: {e}"))?;
        emit!(
            ops::accessibility_input_action(payload).await,
            "screen_intelligence_input_action"
        )
    }
}

/// Recent vision summaries.
pub struct ScreenVisionRecentTool;

#[async_trait]
impl Tool for ScreenVisionRecentTool {
    fn name(&self) -> &str {
        "screen_intelligence_vision_recent"
    }

    fn description(&self) -> &str {
        "Return recent vision summaries from captured frames, optionally capped \
         by `limit`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "limit": { "type": "integer", "minimum": 1 } }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][screen_intelligence] vision_recent invoked");
        let limit = args
            .get("limit")
            .and_then(serde_json::Value::as_u64)
            .map(|v| v as usize);
        emit!(
            ops::accessibility_vision_recent(limit).await,
            "screen_intelligence_vision_recent"
        )
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }
}

/// Request all OS permissions. Default-OFF, Dangerous.
pub struct ScreenRequestPermissionsTool;

#[async_trait]
impl Tool for ScreenRequestPermissionsTool {
    fn name(&self) -> &str {
        "screen_intelligence_request_permissions"
    }

    fn description(&self) -> &str {
        "Trigger the OS permission prompts needed for screen intelligence \
         (accessibility / input monitoring). Shows system dialogs. Default-OFF \
         (opt-in)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({ "type": "object", "properties": {} })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][screen_intelligence] request_permissions invoked");
        emit!(
            ops::accessibility_request_permissions().await,
            "screen_intelligence_request_permissions"
        )
    }
}

/// Request a single OS permission. Default-OFF, Dangerous.
pub struct ScreenRequestPermissionTool;

#[async_trait]
impl Tool for ScreenRequestPermissionTool {
    fn name(&self) -> &str {
        "screen_intelligence_request_permission"
    }

    fn description(&self) -> &str {
        "Trigger the OS permission prompt for a single `permission` kind. Shows \
         a system dialog. Default-OFF (opt-in)."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": { "permission": { "type": "string", "description": "Permission kind." } },
            "required": ["permission"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Dangerous
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        log::debug!("[tool][screen_intelligence] request_permission invoked");
        let payload: PermissionRequestParams = serde_json::from_value(args).map_err(|e| {
            anyhow::anyhow!("screen_intelligence_request_permission: invalid args: {e}")
        })?;
        emit!(
            ops::accessibility_request_permission(payload).await,
            "screen_intelligence_request_permission"
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::ToolScope;

    #[test]
    fn names_and_levels() {
        assert_eq!(ScreenStatusTool.name(), "screen_intelligence_status");
        assert_eq!(
            ScreenStatusTool.permission_level(),
            PermissionLevel::ReadOnly
        );
        assert_eq!(
            ScreenSessionStartTool.permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            ScreenInputActionTool.permission_level(),
            PermissionLevel::Execute
        );
        assert_eq!(
            ScreenRequestPermissionsTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(
            ScreenRequestPermissionTool.permission_level(),
            PermissionLevel::Dangerous
        );
        assert_eq!(ScreenStatusTool.scope(), ToolScope::All);
    }

    #[tokio::test]
    async fn session_start_requires_consent() {
        let err = ScreenSessionStartTool
            .execute(json!({}))
            .await
            .expect_err("missing consent");
        assert!(err.to_string().contains("session_start"));
    }
}
