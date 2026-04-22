//! Controller registry for `webview_notifications`.
//!
//! v1 has no user-facing controllers: the on/off toggle lives in the
//! Tauri shell (per-install state rather than core config) so the
//! settings UI can flip it without a sidecar round-trip. The stubs
//! below exist so this domain participates in `src/core/all.rs` the
//! same way every other domain does, which keeps future additions
//! (notification history, per-account mute, etc.) a trivial extend.

use crate::core::all::RegisteredController;
use crate::core::ControllerSchema;

pub fn all_webview_notifications_controller_schemas() -> Vec<ControllerSchema> {
    Vec::new()
}

pub fn all_webview_notifications_registered_controllers() -> Vec<RegisteredController> {
    Vec::new()
}
