//! Types for tool-provided UI. The presentation vocabulary is `tinymcp`'s;
//! what is defined here is this host's own.

use serde_json::Value;

pub use tinymcp::ui::{
    McpUiPresentation, UiCallView, UiCsp, UiFlavor, UiLink, UiLinkKind, UiResource,
    MCP_APPS_EXTENSION, MCP_APP_MIME, MCP_UI_KIND,
};
pub use tinymcp::MCP_RESULT_KIND;

/// What a widget-initiated tool call needs to know about its target tool.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UiToolDescriptor {
    pub meta: Option<Value>,
    pub annotations: Option<Value>,
}
