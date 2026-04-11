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
//!   [`crate::openhuman::tools::ops::all_tools_with_runtime`].
//!
//! - **Event bus** (`bus.rs`) — `ComposioTriggerSubscriber` listens for
//!   [`DomainEvent::ComposioTriggerReceived`] events published by the
//!   socket transport when the backend emits `composio:trigger`.
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
//! crate::core::event_bus::DomainEvent::ComposioTriggerReceived

pub mod bus;
pub mod client;
pub mod ops;
pub mod periodic;
pub mod providers;
pub mod schemas;
pub mod tools;
pub mod types;

pub use bus::{register_composio_trigger_subscriber, ComposioTriggerSubscriber};
pub use client::{build_composio_client, ComposioClient};
pub use periodic::start_periodic_sync;
pub use providers::{
    all_providers as all_composio_providers, get_provider as get_composio_provider,
    init_default_providers as init_default_composio_providers, ComposioProvider, ProviderContext,
    ProviderUserProfile, SyncOutcome, SyncReason,
};
pub use schemas::{
    all_controller_schemas as all_composio_controller_schemas,
    all_registered_controllers as all_composio_registered_controllers,
};
pub use tools::all_composio_agent_tools;
pub use types::{
    ComposioAuthorizeResponse, ComposioConnection, ComposioConnectionsResponse,
    ComposioDeleteResponse, ComposioExecuteResponse, ComposioToolFunction, ComposioToolSchema,
    ComposioToolkitsResponse, ComposioToolsResponse, ComposioTriggerEvent, ComposioTriggerMetadata,
};
