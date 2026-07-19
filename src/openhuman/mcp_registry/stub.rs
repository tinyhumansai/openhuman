//! Disabled-MCP facade for [`super`] (the dynamic Smithery registry).
//!
//! Compiled only when the `mcp` Cargo feature is OFF (see the gate in
//! [`super`]). It mirrors the subset of the real `mcp_registry` public surface
//! that always-compiled callers depend on, with no-op / disabled-error /
//! empty-collection bodies so the crate still compiles, boots, and serves
//! `/rpc` without the MCP domains.
//!
//! Note what is NOT here: [`super::types`] is ungated, so this module defines
//! no data types at all — it carries behaviour only. Every type an always-on
//! caller names ([`ConnectedServerOverview`], [`McpTool`]) is the *real* one,
//! which is why the two builds cannot drift in shape.
//!
//! The signatures here MUST match the real ones exactly (return types and
//! async-ness included). The disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift — if a real signature changes, update the
//! mirror below until that build is green again.

use crate::openhuman::config::Config;

/// Error text returned by every disabled-path operation that must yield a
/// `Result`. Shared so callers / log-greps see one stable string.
const DISABLED_MSG: &str = "mcp feature disabled at compile time";

// ---------------------------------------------------------------------------
// Controller registration (mirrors `schemas::all_registered_controllers`)
// ---------------------------------------------------------------------------

/// No controllers: the `mcp_clients` RPC namespace does not exist in this
/// build, so every `openhuman.mcp_clients_*` method is an unknown method over
/// `/rpc` and absent from `/schema`.
///
/// `src/core/all.rs` pushes this straight into its controller vec with no
/// `#[cfg]` of its own — the empty vec is what keeps that (very hot,
/// multi-agent) file untouched by this gate.
pub fn all_mcp_registry_registered_controllers() -> Vec<crate::core::all::RegisteredController> {
    Vec::new()
}

// ---------------------------------------------------------------------------
// Boot / lifecycle (mirrors `boot`, `bus`, `supervisor`)
// ---------------------------------------------------------------------------

/// Boot-time spawn of installed local MCP servers.
pub mod boot {
    use super::*;

    /// No-op: no installed-server store exists in this build, so there is
    /// nothing to spawn. The real one already treats every per-server failure
    /// as non-fatal and never blocks boot, so doing nothing is shape-identical
    /// to "every server failed to spawn".
    pub async fn spawn_installed_servers(_config: &Config) {
        log::debug!("[mcp_registry] {DISABLED_MSG} — skipping installed-server spawn");
    }
}

/// DomainEvent subscriber for MCP lifecycle logging.
pub mod bus {
    use super::*;

    /// No-op: with no connection lifecycle there are no events to subscribe to.
    pub fn init() {
        log::debug!("[mcp_registry] {DISABLED_MSG} — event subscriber not registered");
    }
}

/// Reconnect supervisor for installed local-spawn servers.
pub mod supervisor {
    use super::*;

    /// Returns immediately instead of running the reconnect tick loop.
    ///
    /// The real `run` never returns (it is an infinite `interval` loop), and
    /// its caller in `core/runtime/services.rs` spawns it as a background
    /// task and does not await completion — so returning at once simply means
    /// that task finishes rather than idling forever.
    pub async fn run(_config: Config) {
        log::debug!("[mcp_registry] {DISABLED_MSG} — supervisor not started");
    }
}

// ---------------------------------------------------------------------------
// OAuth callback (mirrors `oauth::complete`)
// ---------------------------------------------------------------------------

/// OAuth authorization-code completion for HTTP-remote MCP servers.
pub mod oauth {
    use super::*;

    /// Always errors: no pending-authorization map exists in this build, so a
    /// callback can only be a stale/blind hit. The real function returns the
    /// same `Err(String)` shape for an unknown state, and its `core/jsonrpc.rs`
    /// caller already renders that as an error page.
    pub async fn complete(_config: &Config, _state: &str, _code: &str) -> Result<String, String> {
        Err(DISABLED_MSG.to_string())
    }
}

// ---------------------------------------------------------------------------
// Live connection map (mirrors `connections`)
// ---------------------------------------------------------------------------

/// Global in-process registry of connected MCP servers.
pub mod connections {
    /// Re-exported from the ungated `types` module — the SAME type the enabled
    /// build uses, not a mirrored copy, so the orchestrator prompt builder's
    /// field access can never drift between builds.
    pub use super::super::types::ConnectedServerOverview;
    use super::super::types::McpTool;

    /// Empty: nothing can be connected when the registry is compiled out.
    ///
    /// Always-on callers already handle this shape with zero `#[cfg]` — the
    /// orchestrator prompt skips its "## Connected MCP Servers" block on an
    /// empty slice, and the turn loop seeds an empty announced-server set.
    pub async fn connected_overview() -> Vec<ConnectedServerOverview> {
        Vec::new()
    }

    /// Empty: no connected servers means no advertised tools. `tool_registry`
    /// folds this into its entry map, which simply gains no `mcp-client::*`
    /// entries.
    pub async fn all_connected_tools() -> Vec<(String, String, McpTool)> {
        Vec::new()
    }
}
