//! Notification domain.
//!
//! Two complementary sub-systems live here:
//!
//! **Core-bridge** (`bus`): Subscribes to selected
//! [`DomainEvent`](crate::core::event_bus::DomainEvent) variants (cron
//! completions, webhook processed, sub-agent completions) and republishes them
//! as `CoreNotificationEvent` payloads on a broadcast channel consumed by the
//! Socket.IO bridge. The frontend listens on `core_notification` and funnels
//! the payload into the in-app notification center.
//!
//! **Integration notifications** (`rpc` / `store` / `schemas`): Captures
//! notifications from embedded webview integrations (WhatsApp Web, Gmail,
//! Slack, …), runs them through the triage LLM pipeline, and stores them in a
//! unified notification center accessible via the RPC surface.
//!
//! ## Module layout
//!
//! - [`bus`]     — `NotificationBridgeSubscriber`, publish/subscribe helpers
//! - [`types`]   — `CoreNotificationEvent`, `IntegrationNotification`, request/response types
//! - [`store`]   — SQLite persistence (one DB per workspace)
//! - [`rpc`]     — Async RPC handler functions: ingest, list, mark_read
//! - [`schemas`] — Controller schema definitions and registered handler wrappers

pub mod bus;
pub mod rpc;
pub mod schemas;
pub mod store;
pub mod types;

pub use bus::{
    publish_core_notification, register_notification_bridge_subscriber,
    subscribe_core_notifications, NotificationBridgeSubscriber,
};
pub use schemas::{
    all_controller_schemas as all_notifications_controller_schemas,
    all_registered_controllers as all_notifications_registered_controllers,
};
pub use types::*;
