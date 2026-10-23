//! Network tools: GitBooks docs lookup, Gmail unsubscribe, and (under the
//! `mcp` feature) the MCP client/setup tools, plus the host adapter for the
//! tools that live in `tinytools_std::network` (`http_request`, `web_fetch`,
//! `curl`, `pushover`). [`gate`] implements `NetGate` for `SecurityPolicy` and
//! [`host`] supplies the limits, HTML engine and x402 payment hook those tools
//! are handed. `url_guard` (in `tinytools_std`) is the shared lexical
//! SSRF/allowlist validator; the gate's `authorize_destination` then asks the
//! TinySecurity module for the destination and the addresses the tool may
//! connect to. Registration gates are documented in `tools/impl/README.md`.

mod gate;
mod gitbooks;
mod gmail_unsubscribe;
mod host;
// Leaf-gated: the only consumers of these two are the `#[cfg(feature = "mcp")]`
// blocks in `tools/ops.rs`, so no stub is needed — nothing names them when the
// feature is off. (`gitbooks` is deliberately NOT gated: it uses TinyMCP's
// confidential one-shot module call but is a docs tool, not MCP-subsystem code.)
#[cfg(feature = "mcp")]
mod mcp;
#[cfg(feature = "mcp")]
mod mcp_server_tools;

pub use gitbooks::{GitbooksGetPageTool, GitbooksSearchTool};
pub use gmail_unsubscribe::GmailUnsubscribeTool;
#[cfg(feature = "web3")]
pub(crate) use host::x402_request_tool;
pub use host::{
    http_request_tool, routed_curl_tool, routed_http_request_tool, routed_web_fetch_tool,
    web_fetch_tool,
};
#[cfg(feature = "mcp")]
pub use mcp::{mcp_call_tool, McpListServersTool, McpListToolsTool};
#[cfg(feature = "mcp")]
pub use mcp_server_tools::configured_server_tools_from_presentations;

/// Shared test helper for the network tools' local-only enforcement tests
/// (privacy epic S7, #4441). Returns a thread-scoped `LocalOnly` privacy
/// override guard: the egress gate (which reads
/// `live_policy::current_privacy_mode`) observes `LocalOnly` on this thread only,
/// so the test never mutates the process-global policy that sibling tests read
/// on other threads. Hold the returned guard for the duration of the tool call.
#[cfg(test)]
pub(crate) fn local_only_scope() -> crate::security::live_policy::TestPrivacyGuard {
    crate::security::live_policy::test_privacy_scope(crate::config::PrivacyMode::LocalOnly)
}
