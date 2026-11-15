//! Network tools: the host adapter for the tools that live in
//! `tinyagents_harness_tools::net::network` (`http_request`, `web_fetch`,
//! `curl`, `pushover`). [`gate`] implements `NetGate` for `SecurityPolicy` and
//! [`host`] supplies the limits, HTML engine and x402 payment hook those tools
//! are handed. `url_guard` (in `tinyagents_harness`) is the shared SSRF/allowlist
//! validator. The MCP bridge tools and the GitBooks docs tools live in `mcp/`.
//! Registration gates are documented in `tools/impl/README.md`.

mod gate;
mod host;

#[cfg(feature = "web3")]
pub(crate) use host::x402_request_tool;
pub use host::{
    http_request_tool, routed_curl_tool, routed_http_request_tool, routed_web_fetch_tool,
    web_fetch_tool,
};

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
