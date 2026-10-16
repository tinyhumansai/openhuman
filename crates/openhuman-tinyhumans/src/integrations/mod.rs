//! Hosted integrations: the integrations client, budget gate, file storage,
//! media generation, web3 crypto client and the Composio backend route.
//!
//! This code used to live in `openhuman-core` (`integrations/client`,
//! `integrations/tools`, `integrations/file_storage`, `media/generation/provider`,
//! `web3/client`) next to the hosted routes it calls. It plugs back into the
//! core only through the generic registries on `openhuman_embed::__host`:
//! [`tool_sources`](openhuman_embed::__host::tool_sources) for the agent tools,
//! [`composio`](openhuman_embed::__host::composio) for the backend route, and
//! the module wallet-backend hook for the web3 crypto client, and
//! [`artifact_publisher`](openhuman_embed::__host::artifact_publisher) for sharing
//! artifacts through file storage. [`register`] is
//! the one place it does so; `crate::register::register_all` calls it from
//! `install()`.

pub mod client;
pub mod composio;
pub mod file_storage;
#[cfg(feature = "media")]
pub mod media;
pub mod tools;
pub mod types;
#[cfg(feature = "web3")]
pub mod web3;

pub use client::{build_client, pricing_for_config, IntegrationClient};
pub use types::{
    BackendResponse, IntegrationPricing, IntegrationPricingEntry, PricingIntegrations,
};

use openhuman_embed::__host::tool_sources::{
    register_tool_source_at, ToolSourcePlacement, ANCHOR_AFTER_DELEGATE, ANCHOR_AFTER_SEARCH,
};

use crate::install::InstallOptions;

/// Registers this area's hooks with the core. Idempotent.
///
/// The tool sources register in the order the core used to build them inline
/// (media, file storage, then the Google Places / stock tools) and at the
/// anchors it built them at: media and file storage right after the search
/// tools, the Google Places / stock tools right after the delegation tool, so
/// the catalogue order (a prompt-cache layout input) is unchanged.
pub(crate) fn register(_options: &InstallOptions) {
    #[cfg(feature = "media")]
    media::register();
    register_tool_source_at(
        FILE_STORAGE_SOURCE_ID,
        ToolSourcePlacement::After(ANCHOR_AFTER_SEARCH),
        |ctx| file_storage::build_file_storage_tools(ctx.root_config, ctx.action_dir),
    );
    register_tool_source_at(
        INTEGRATION_TOOLS_SOURCE_ID,
        ToolSourcePlacement::After(ANCHOR_AFTER_DELEGATE),
        |ctx| tools::build_integration_tools(ctx.root_config),
    );
    #[cfg(feature = "web3")]
    web3::register();
    composio::register();
    openhuman_embed::__host::artifact_publisher::register_artifact_publisher(
        ARTIFACT_PUBLISHER_ID,
        file_storage::artifact_publisher,
    );
    log::debug!(
        "[tinyhumans] integrations: tool sources, wallet backend, composio route and artifact publisher registered"
    );
}

/// The id the file-storage tool source registers under.
pub const FILE_STORAGE_SOURCE_ID: &str = "tinyhumans-file-storage";
/// The id the artifact publisher (sharing through file storage) registers under.
pub const ARTIFACT_PUBLISHER_ID: &str = "tinyhumans-file-storage";
/// The id the Google Places / stock tool source registers under.
pub const INTEGRATION_TOOLS_SOURCE_ID: &str = "tinyhumans-integrations";

#[cfg(test)]
mod test_support;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
