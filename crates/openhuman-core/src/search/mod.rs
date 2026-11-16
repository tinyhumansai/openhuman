//! Web search: host policy over the TinySearch module.
//!
//! Provider implementations, tool schemas and role dispatch live in
//! `vendor/tinysearch`. This domain owns what OpenHuman decides: which
//! providers are usable for this process (`providers`), how a response is shown
//! to the model and the chat UI (`render`), the agent tool bridge (`tools`),
//! the `tools.web_*` / `tools.searxng_search` RPC controllers (`schemas`,
//! `rpc`), and refreshing the module when the credential changes (`bus`).

pub mod providers;
pub mod render;
mod rpc;
mod schemas;

pub(crate) use schemas::{
    all_controller_schemas as all_search_controller_schemas,
    all_registered_controllers as all_search_registered_controllers,
};

#[cfg(feature = "modules")]
pub mod bus;
#[cfg(feature = "modules")]
pub mod tools;

#[cfg(feature = "modules")]
pub use tools::{build_search_tools, TinySearchTool};

/// Without the module loader there is no search provider to call.
#[cfg(not(feature = "modules"))]
pub async fn build_search_tools(_config: &crate::config::Config) -> Vec<Box<dyn tinytools::Tool>> {
    tracing::debug!("[search][tool] modules feature disabled; no search tools");
    Vec::new()
}

/// Whether web search is on for this config: the `[components].search`
/// master switch (which absorbed the legacy `search.enabled = false` at load)
/// and the runtime `search.enabled` flag, which `OPENHUMAN_SEARCH_ENABLED`
/// and the settings RPC still set.
pub fn is_active(config: &crate::config::Config) -> bool {
    config.components.search && config.search.is_enabled()
}
