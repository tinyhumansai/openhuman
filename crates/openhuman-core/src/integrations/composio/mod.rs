//! Composio domain module — backend-proxied access to 1000+ OAuth
//! integrations (Gmail, Notion, GitHub, Slack, …).
//!
//! This module is the Rust counterpart to the backend routes under
//! `src/routes/agentIntegrations/composio.ts`. The backend owns the
//! Composio API key, billing/margin, toolkit allowlist, HMAC webhook
//! verification, and Socket.IO trigger fan-out. The core does **not**
//! hit the Composio API directly — everything goes through the backend.
//!
//! ## Surface
//!
//! - **RPC controllers** (`schemas.rs` / `ops.rs`) — `openhuman.composio_*`
//!   methods for listing toolkits, managing connections, listing tools,
//!   and executing actions. These are registered in
//!   [`crate::core::all`] alongside other domains.
//!
//! - **Agent tools** (`tools.rs`) — model-facing `composio_*` tools the
//!   autonomous agent loop can call. Registered from
//!   [`crate::tools::ops::all_tools_with_runtime`].
//!
//! - **Event bus** (`bus.rs`) — `ComposioTriggerSubscriber` listens for
//!   [`DomainEvent::ComposioTriggerReceived`] events published by the
//!   socket transport when the backend emits `composio:trigger`; the
//!   connection-created and config-changed subscribers keep the
//!   integrations cache fresh.
//!
//! ## Socket.IO trigger flow
//!
//! ```text
//!  Composio webhook → backend HMAC-verifies → backend emits
//!  `composio:trigger` on user sockets → core
//!  `socket::event_handlers::handle_sio_event` parses the payload →
//!  publishes `DomainEvent::ComposioTriggerReceived` → the
//!  `ComposioTriggerSubscriber` (and any future subscribers) reacts.
//! ```
//!
//! [`DomainEvent::ComposioTriggerReceived`]:
//! crate::core::events::DomainEvent::ComposioTriggerReceived

pub mod action_tool;
pub mod bus;
pub mod catalog;
pub mod client;
pub mod connected_integrations;
pub mod contract;
pub mod contract_gate;
pub(crate) mod direct_auth;
pub mod execute_dispatch;
pub(crate) mod file_store;
mod file_store_documents;
pub mod googlecalendar_args;
pub mod identity;
pub mod identity_store;
pub mod module_client;
pub mod ops;
pub(crate) mod processing;
pub mod providers;
pub mod schemas;
pub mod tools;
pub(crate) mod trigger_history;
pub mod types;

pub use crate::agent::prompts::types::ConnectedIntegration;
pub use crate::integrations::composio::providers::ProviderUserProfile;
pub use action_tool::ComposioActionTool;
pub use bus::{
    register_composio_trigger_subscriber, ComposioConfigChangedSubscriber,
    ComposioConnectionCreatedSubscriber, ComposioTriggerSubscriber,
};
pub use identity::connection_identity;
pub use ops::{
    cached_active_integrations, cached_active_integrations_including_expired, connected_set_hash,
    fetch_connected_integrations, fetch_connected_integrations_status,
    invalidate_connected_integrations_cache, FetchConnectedIntegrationsStatus,
};
pub use schemas::{
    all_controller_schemas as all_composio_controller_schemas,
    all_registered_controllers as all_composio_registered_controllers,
};
pub use tools::all_composio_agent_tools;
pub use trigger_history::{
    global as global_composio_trigger_history, init_global as init_composio_trigger_history,
};
pub use types::{
    ComposioAgentReadyToolkitsResponse, ComposioAuthorizeResponse, ComposioCapabilitiesResponse,
    ComposioCapability, ComposioConnection, ComposioConnectionsResponse, ComposioDeleteResponse,
    ComposioExecuteResponse, ComposioToolFunction, ComposioToolSchema, ComposioToolkitsResponse,
    ComposioToolsResponse, ComposioTriggerEvent, ComposioTriggerHistoryEntry,
    ComposioTriggerHistoryResult, ComposioTriggerMetadata,
};
