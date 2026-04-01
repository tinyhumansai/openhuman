//! Webhook tunnel routing — maps backend tunnel UUIDs to owning skills.
//!
//! Routes incoming webhooks from the backend's hosted tunnel system to the
//! appropriate skill. The backend manages tunnel provisioning (ngrok, cloudflare,
//! etc.); this module handles the client-side routing and skill dispatch.

pub mod router;
pub mod types;

pub use router::WebhookRouter;
pub use types::{TunnelRegistration, WebhookActivityEntry, WebhookRequest, WebhookResponseData};
