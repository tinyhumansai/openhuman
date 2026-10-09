//! Tool-provided UI in the chat: MCP Apps widgets, OpenAI Apps SDK widgets,
//! skill views, and the links a tool result offers.
//!
//! - [`resolve`] applies `tinymcp::ui`'s rules to one call's descriptor
//!   `_meta` and result, giving a [`types::McpUiPresentation`], the metadata
//!   the chat surface renders, and caches any document the result carried.
//! - [`decorate`] applies that to every MCP tool path.
//! - [`ops`] / `schemas` serve a widget its document and run the tool calls
//!   it asks for (`mcp_ui` namespace), through [`port::UiServerPort`].
//! - [`tools`] is `show_ui`, the host tool a skill renders HTML with.
//!
//! Widget HTML stays in [`cache`], in memory; events and transcripts carry
//! only the bounded presentation.

pub mod cache;
pub mod decorate;
pub mod discovery;
pub mod ops;
pub mod port;
pub mod resolve;
mod schemas;
pub mod tools;
pub mod types;

pub use decorate::{decorate_result, UiAwareTool};
pub use schemas::{
    all_controller_schemas as all_mcp_ui_controller_schemas,
    all_registered_controllers as all_mcp_ui_registered_controllers,
};
pub use tools::ShowUiTool;

/// `show_ui`, boxed for the tool list.
#[must_use]
pub fn show_ui_tool(config: &crate::config::Config) -> Box<dyn tinytools::Tool> {
    Box::new(ShowUiTool::for_config(config))
}

/// `mcp_call_tool` over a configured-server registry, presenting the UI of
/// the remote tool it calls.
#[must_use]
pub fn ui_aware_call(
    call: Box<dyn tinytools::Tool>,
    registry: &std::sync::Arc<crate::mcp::config_servers::McpServerRegistry>,
) -> Box<dyn tinytools::Tool> {
    Box::new(UiAwareTool::new(
        call,
        decorate::MetaLookup::Configured(std::sync::Arc::clone(registry)),
        decorate::ToolInput::Nested("arguments"),
    ))
}

/// The `initialize` capabilities this host advertises: MCP Apps support.
#[must_use]
pub fn client_capabilities() -> serde_json::Value {
    tinymcp::ui::client_capabilities()
}
