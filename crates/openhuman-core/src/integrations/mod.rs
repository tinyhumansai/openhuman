//! Agent integrations that run through the core itself.
//!
//! Composio connectors ([`composio`]) and task sources ([`task_sources`]) live
//! here. The hosted integrations client, its managed tools (Google
//! Places, stock data, file storage, media generation) and the web3 crypto
//! client are not part of the core: a host that has such a service plugs them
//! in through the generic registries (tool sources, the Composio route
//! builders, the wallet-backend hook). With none registered the core runs
//! without them.

pub(crate) mod announcement_notes;
pub mod composio;
mod connected_identities;
pub mod gmail;
pub(crate) mod recorded_tools;
pub mod task_sources;
pub use connected_identities::{render_connected_identities, render_connected_identities_async};

// Product prompt metadata shared by connector discovery and agent surfaces.
pub use crate::agent::prompts::{
    ConnectedIntegration, ConnectedIntegrationTool, GatedIntegrationTool, IntegrationConnection,
};
