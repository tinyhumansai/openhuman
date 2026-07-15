mod clob_auth;
mod curl;
mod gitbooks;
mod gmail_unsubscribe;
mod http_request;
mod mcp;
mod mcp_setup;
mod polymarket;
mod polymarket_orders;
mod url_guard;
mod web_fetch;

pub use curl::CurlTool;
pub use gitbooks::{GitbooksGetPageTool, GitbooksSearchTool};
pub use gmail_unsubscribe::GmailUnsubscribeTool;
pub use http_request::HttpRequestTool;
pub use mcp::{McpCallTool, McpListServersTool, McpListToolsTool};
pub use mcp_setup::{
    McpSetupGetTool, McpSetupInstallAndConnectTool, McpSetupRequestSecretTool, McpSetupSearchTool,
    McpSetupTestConnectionTool,
};
pub use polymarket::PolymarketTool;
pub use web_fetch::WebFetchTool;

/// Shared test helper for the network tools' local-only enforcement tests
/// (privacy epic S7, #4441). Returns a thread-scoped `LocalOnly` privacy
/// override guard: the egress gate (which reads
/// `live_policy::current_privacy_mode`) observes `LocalOnly` on this thread only,
/// so the test never mutates the process-global policy that sibling tests read
/// on other threads. Hold the returned guard for the duration of the tool call.
#[cfg(test)]
pub(crate) fn local_only_scope() -> crate::openhuman::security::live_policy::TestPrivacyGuard {
    crate::openhuman::security::live_policy::test_privacy_scope(
        crate::openhuman::config::PrivacyMode::LocalOnly,
    )
}
