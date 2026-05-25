//! Upstream MCP registries. Today: Smithery.ai and the official
//! [modelcontextprotocol/registry](https://github.com/modelcontextprotocol/registry).
//!
//! All registries implement the [`Registry`] trait and return results in the
//! canonical [`super::types::SmitheryServerSummary`] /
//! [`super::types::SmitheryServerDetail`] shapes (named after Smithery for
//! backwards compatibility with the existing wire contract — non-Smithery
//! registries adapt their responses into the same shape and tag the `source`
//! field so the frontend can render provenance).
//!
//! [`enabled_registries`] returns every registry that should participate in a
//! query. Today both registries are always enabled; this will become
//! config-driven once the wider scope lands.

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::config::Config;

use super::types::{SmitheryServerDetail, SmitheryServerSummary};

pub mod mcp_official;
pub mod smithery;

/// Canonical id for an upstream registry. Echoed back in
/// [`SmitheryServerSummary::source`] / [`SmitheryServerDetail::source`].
pub const SOURCE_SMITHERY: &str = "smithery";
pub const SOURCE_MCP_OFFICIAL: &str = "mcp_official";

/// An upstream MCP server directory.
#[async_trait]
pub trait Registry: Send + Sync {
    /// Canonical identifier (see `SOURCE_*` constants). Returned on every
    /// result so the frontend can attribute and the install path can route
    /// `registry_get` back to the correct upstream.
    fn source(&self) -> &'static str;

    /// Search the registry. `page` is 1-indexed; registries that use
    /// cursor-based pagination map their own cursor space onto page numbers
    /// internally.
    ///
    /// Returns `(servers, total_pages_known)`. `total_pages_known` is the
    /// best-effort upper bound — registries that can't compute it report
    /// the current page number.
    async fn search(
        &self,
        config: &Config,
        query: Option<&str>,
        page: u32,
        page_size: u32,
    ) -> Result<(Vec<SmitheryServerSummary>, u32)>;

    /// Fetch one server's full detail by qualified name.
    async fn get(&self, config: &Config, qualified_name: &str) -> Result<SmitheryServerDetail>;
}

/// All registries currently enabled for the user. Today: Smithery + official.
pub fn enabled_registries() -> Vec<Box<dyn Registry>> {
    vec![
        Box::new(smithery::SmitheryRegistry),
        Box::new(mcp_official::McpOfficialRegistry),
    ]
}

/// Resolve a registry by [`Registry::source`] id. Used by `registry_get` to
/// route a fetch back to the upstream that produced the qualified name.
pub fn registry_for_source(source: &str) -> Option<Box<dyn Registry>> {
    enabled_registries()
        .into_iter()
        .find(|r| r.source() == source)
}
