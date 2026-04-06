//! Webhook tunnel routing — maps backend tunnel UUIDs to owning skills.
//!
//! Routes incoming webhooks from the backend's hosted tunnel system to the
//! appropriate skill. The backend manages tunnel provisioning (ngrok, cloudflare,
//! etc.); this module handles the client-side routing and skill dispatch.

pub mod bus;
pub mod ops;
pub mod router;
mod schemas;
pub mod types;

pub use router::WebhookRouter;
pub use schemas::{
    all_controller_schemas as all_webhooks_controller_schemas,
    all_registered_controllers as all_webhooks_registered_controllers,
};
pub use types::{
    TunnelRegistration, WebhookActivityEntry, WebhookDebugEvent, WebhookDebugLogEntry,
    WebhookDebugLogListResult, WebhookDebugLogsClearedResult, WebhookDebugRegistrationsResult,
    WebhookRequest, WebhookResponseData,
};

#[cfg(test)]
mod tests;
