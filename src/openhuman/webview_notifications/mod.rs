//! Webview-originated Web Notifications routed to the OS.
//!
//! Scope (v1): deliver `window.Notification` invocations from embedded
//! webviews (Slack, Gmail, Discord, …) as native OS toasts, with the
//! account + provider encoded on the notification so the UI can focus
//! the right webview on click.
//!
//! The CEF IPC hook that captures the renderer-side call lives in the
//! Tauri shell crate (`openhuman` crate at `app/src-tauri/` —
//! `tauri_runtime_cef::notification::register`). This domain owns the
//! shared wire types, the title-formatting contract (`OpenHuman:`
//! prefix for dedup against installed native apps), and future
//! controllers that read/write the user-facing on/off toggle over
//! JSON-RPC.

pub mod bus;
pub mod dispatch;
pub mod schemas;
pub mod types;

pub use dispatch::{format_title, OPENHUMAN_TITLE_PREFIX};
pub use schemas::{
    all_webview_notifications_controller_schemas, all_webview_notifications_registered_controllers,
};
pub use types::{NotificationSettings, WebviewNotificationEvent};
