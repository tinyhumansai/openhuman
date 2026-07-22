//! Disabled-build stub for the `screen_intelligence` domain (`desktop-automation`
//! off). Re-exposes the always-on caller surface — the controller/tool
//! aggregators, the `global_engine()` handle (`apply_config` / `status` /
//! `disable`), the `server` lifecycle, the `rpc` capture entry point, and the
//! `cli` subcommand — with empty / no-op / disabled bodies.
//!
//! The status/session/result types are carved out in `super::types` and stay
//! compiled in both builds, so `app_state`'s literal `AccessibilityStatus`
//! construction needs no stub. Only behaviour lives here.

use std::sync::Arc;

use once_cell::sync::Lazy;

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;
use crate::openhuman::config::ScreenIntelligenceConfig;

use super::types::{
    AccessibilityFeatures, AccessibilityStatus, PermissionState, PermissionStatus, SessionStatus,
};

const DISABLED: &str =
    "screen intelligence is disabled in this build (rebuild with --features desktop-automation)";

/// Real: `schemas::all_registered_controllers` (re-exported as
/// `all_screen_intelligence_registered_controllers`). Empty ⇒ unregistered.
pub fn all_screen_intelligence_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}

/// Real: `schemas::all_controller_schemas`.
pub fn all_screen_intelligence_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

/// Real: `state::AccessibilityEngine`. Inert handle exposing only the methods the
/// always-on callers (`app_state`, `config::ops::ui`) reach.
pub struct AccessibilityEngine;

impl AccessibilityEngine {
    /// Real: `AccessibilityEngine::apply_config` → `Result<AccessibilityStatus, _>`.
    pub async fn apply_config(
        &self,
        config: ScreenIntelligenceConfig,
    ) -> Result<AccessibilityStatus, String> {
        Ok(disabled_status(config))
    }

    /// Real: `AccessibilityEngine::status`.
    pub async fn status(&self) -> AccessibilityStatus {
        disabled_status(ScreenIntelligenceConfig::default())
    }

    /// Real: `AccessibilityEngine::disable` → `SessionStatus`.
    pub async fn disable(&self, reason: Option<String>) -> SessionStatus {
        disabled_session(reason)
    }
}

static ACCESSIBILITY_ENGINE: Lazy<Arc<AccessibilityEngine>> =
    Lazy::new(|| Arc::new(AccessibilityEngine));

/// Real: `state::global_engine`.
pub fn global_engine() -> Arc<AccessibilityEngine> {
    ACCESSIBILITY_ENGINE.clone()
}

/// Disabled-build mirror of `screen_intelligence::ops` — the `rpc` alias.
/// `tools::local_cli` reaches `rpc::accessibility_capture_image_ref`.
pub mod rpc {
    use super::DISABLED;
    use crate::openhuman::screen_intelligence::types::CaptureImageRefResult;
    use crate::rpc::RpcOutcome;

    /// Real: `ops::accessibility_capture_image_ref`.
    pub async fn accessibility_capture_image_ref(
    ) -> Result<RpcOutcome<CaptureImageRefResult>, String> {
        log::debug!(
            "[screen_intelligence] capture_image_ref rejected: desktop-automation disabled at compile time"
        );
        Ok(RpcOutcome::new(
            CaptureImageRefResult {
                ok: false,
                image_ref: None,
                mime_type: "image/png".to_string(),
                bytes_estimate: None,
                message: DISABLED.to_string(),
            },
            vec![DISABLED.to_string()],
        ))
    }
}

/// Disabled-build mirror of `screen_intelligence::server`.
pub mod server {
    use crate::openhuman::config::Config;

    /// Real: `server::start_if_enabled`. No server to start when disabled.
    pub async fn start_if_enabled(_app_config: &Config) {}

    /// Real: `server::try_global_server`. Never a running server when disabled;
    /// returning `None` short-circuits the `if let Some(server)` stop path in
    /// `credentials::ops`.
    pub fn try_global_server() -> Option<std::sync::Arc<SiServer>> {
        None
    }

    /// Real: `server::SiServer`. Only reached via `try_global_server()`, which the
    /// stub always returns `None` for, so no method body is ever invoked — but the
    /// `server.stop()` call site still needs the method to exist to type-check.
    pub struct SiServer;

    impl SiServer {
        pub async fn stop(&self) {}
    }
}

/// Disabled-build mirror of `screen_intelligence::cli`.
pub mod cli {
    use anyhow::Result;

    /// Real: `cli::run_screen_intelligence_command`. Reports the build fact rather
    /// than running a no-op command.
    pub(crate) fn run_screen_intelligence_command(_args: &[String]) -> Result<()> {
        log::debug!(
            "[screen_intelligence] CLI command rejected: desktop-automation disabled at compile time"
        );
        Err(anyhow::anyhow!(super::DISABLED))
    }
}

fn disabled_status(config: ScreenIntelligenceConfig) -> AccessibilityStatus {
    AccessibilityStatus {
        platform_supported: false,
        permissions: PermissionStatus {
            screen_recording: PermissionState::Unknown,
            accessibility: PermissionState::Unknown,
            input_monitoring: PermissionState::Unknown,
            microphone: PermissionState::Unknown,
        },
        features: AccessibilityFeatures {
            screen_monitoring: false,
        },
        session: disabled_session(None),
        foreground_context: None,
        config,
        denylist: Vec::new(),
        is_context_blocked: false,
        permission_check_process_path: None,
        core_process: None,
    }
}

fn disabled_session(reason: Option<String>) -> SessionStatus {
    SessionStatus {
        active: false,
        started_at_ms: None,
        expires_at_ms: None,
        remaining_ms: None,
        ttl_secs: 0,
        panic_hotkey: String::new(),
        stop_reason: reason,
        capture_count: 0,
        frames_in_memory: 0,
        last_capture_at_ms: None,
        last_context: None,
        last_window_title: None,
        vision_enabled: false,
        vision_state: "disabled".to_string(),
        vision_queue_depth: 0,
        last_vision_at_ms: None,
        last_vision_summary: None,
        vision_persist_count: 0,
        last_vision_persisted_key: None,
        last_vision_persist_error: None,
    }
}
