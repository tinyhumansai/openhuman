//! Disabled-MCP facade for [`super`] (the OpenHuman-as-an-MCP-server surface).
//!
//! Compiled only when the `mcp` Cargo feature is OFF (see the gate in
//! [`super`]). It mirrors the subset of the real `mcp_server` public surface
//! that always-compiled callers reach, with disabled-error / empty bodies.
//!
//! [`super::tools::types`] is ungated, so [`McpToolSpec`] here is the real
//! type, not a mirrored copy — this module carries behaviour only.
//!
//! The signatures MUST match the real ones exactly (return types and
//! async-ness included). The disabled build
//! (`cargo check --no-default-features --features tokenjuice-treesitter`) is
//! the only thing that catches drift.

use super::tools::McpToolSpec;

/// Error text returned by every disabled-path operation that must yield a
/// `Result`. Shared so callers / log-greps see one stable string.
const DISABLED_MSG: &str = "mcp feature disabled at compile time";

// ---------------------------------------------------------------------------
// CLI entry point (mirrors `stdio::run_stdio_from_cli`)
// ---------------------------------------------------------------------------

/// Fails with a build-fact diagnostic instead of serving MCP over stdio.
///
/// This is deliberately a stub rather than a `#[cfg]` on the `"mcp"` match arm
/// in `src/core/cli.rs`. Deleting the arm is the naive move and is WRONG: the
/// `mcp` token would fall through to generic namespace resolution and die with
/// `unknown namespace: mcp`, which reads like the user typo'd a command rather
/// than like a deliberate property of this build. Keeping the arm and failing
/// here means:
///
/// * an MCP host (Claude Desktop, Cursor, …) that spawns `openhuman mcp` gets
///   a non-zero exit and a one-line stderr diagnostic naming the fix, instead
///   of hanging forever on an stdout stream that never speaks JSON-RPC;
/// * `cli.rs` needs no `#[cfg]` at all, so the gate stays invisible to the
///   transport layer.
///
/// Banner suppression in `cli.rs` is a `matches!` on the raw string, so it
/// keeps working here without touching a gated symbol.
pub fn run_stdio_from_cli(_args: &[String]) -> anyhow::Result<()> {
    log::warn!(
        "[mcp_server] {DISABLED_MSG} — `openhuman mcp` rejected; rebuild with `--features mcp`"
    );
    anyhow::bail!(
        "{DISABLED_MSG}: this build was compiled without the `mcp` feature, so the MCP stdio \
         server is unavailable. Rebuild with `--features mcp`."
    )
}

// ---------------------------------------------------------------------------
// In-process local HTTP server (mirrors `local::{ensure_local_http, LocalMcpEndpoint}`)
// ---------------------------------------------------------------------------

/// Address + bearer token of the in-process MCP HTTP server.
///
/// Mirrors [`super::local::LocalMcpEndpoint`] (real build). No value of this
/// type is ever constructed here — [`ensure_local_http`] always errors — but
/// the type must stay nameable for call sites that bind its `Ok` variant.
#[derive(Debug, Clone)]
pub struct LocalMcpEndpoint {
    pub addr: std::net::SocketAddr,
    pub token: String,
}

/// Always errors: there is no MCP server to stand up in this build.
///
/// The sole always-on caller (`inference::provider::claude_code::driver`)
/// already handles the `Err` arm by logging "…CC running without OpenHuman MCP
/// tools" and continuing — so Claude Code still runs, just without our tool
/// surface injected. That call site needs no `#[cfg]`.
pub async fn ensure_local_http() -> anyhow::Result<LocalMcpEndpoint> {
    log::debug!("[mcp_server] {DISABLED_MSG} — local HTTP endpoint not stood up");
    Err(anyhow::anyhow!(DISABLED_MSG))
}

// ---------------------------------------------------------------------------
// Tool catalog (mirrors `tools::tool_specs`)
// ---------------------------------------------------------------------------

/// Empty catalog: this build advertises no tools over MCP.
///
/// `tool_registry::ops::registry_entries` folds this into a `BTreeMap`, so an
/// empty vec simply means the registry gains no `mcp_stdio`-transport entries.
/// No `#[cfg]` needed at that call site.
pub fn tool_specs() -> Vec<McpToolSpec> {
    log::debug!("[mcp_server] {DISABLED_MSG} — advertising empty MCP tool catalog");
    Vec::new()
}
