//! Upstream MCP registries.
//!
//! Primary: the official [modelcontextprotocol.io registry](https://registry.modelcontextprotocol.io/docs).
//! Fallback: Smithery.ai (legacy, kept for servers not yet listed on the official registry).
//!
//! All registries implement the [`Registry`] trait and return results in the
//! canonical [`super::types::SmitheryServerSummary`] /
//! [`super::types::SmitheryServerDetail`] shapes (named after Smithery for
//! backwards compatibility with the existing wire contract — non-Smithery
//! registries adapt their responses into the same shape and tag the `source`
//! field so the frontend can render provenance).
//!
//! [`enabled_registries`] returns every registry that should participate in a
//! query. The official registry is listed first so its results appear at the
//! top of merged search results and its `get` resolves first.

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

/// Every registry adapter, unconditionally. Used to resolve a `source` id back
/// to its adapter even when that registry isn't part of the default search set
/// (e.g. `registry_get` for an already-installed Smithery server).
fn all_registries() -> Vec<Box<dyn Registry>> {
    vec![
        Box::new(mcp_official::McpOfficialRegistry),
        Box::new(smithery::SmitheryRegistry),
    ]
}

/// Registries that participate in catalog **search** for this user. The
/// official modelcontextprotocol.io registry is always on; Smithery is included
/// only when a Smithery API key is configured.
///
/// Why gate Smithery: its servers don't run standalone — they're reached
/// through Smithery's gateway (`server.smithery.ai/<qn>/mcp?api_key=…&profile=…`)
/// using the user's Smithery account, with per-server credentials configured on
/// smithery.ai. Without a key they can't connect in-app, so listing thousands
/// of them only yields un-installable rows with a misleading "sign in" banner.
/// Surface them only once the user has opted in by setting a key.
pub fn enabled_registries(config: &Config) -> Vec<Box<dyn Registry>> {
    let mut registries: Vec<Box<dyn Registry>> = vec![Box::new(mcp_official::McpOfficialRegistry)];
    if smithery::smithery_api_key(config).is_some() {
        registries.push(Box::new(smithery::SmitheryRegistry));
    }
    registries
}

/// Resolve a registry by [`Registry::source`] id. Searches *all* adapters (not
/// just the search-enabled ones) so a source-routed `registry_get` for an
/// already-installed server resolves regardless of the Smithery-key gate.
pub fn registry_for_source(source: &str) -> Option<Box<dyn Registry>> {
    all_registries().into_iter().find(|r| r.source() == source)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_for_source_resolves_every_adapter_even_when_search_gated() {
        // `registry_for_source` must resolve Smithery regardless of the
        // search-time key gate, so detail lookups for an already-installed
        // Smithery server keep working when no key is set.
        assert_eq!(
            registry_for_source(SOURCE_MCP_OFFICIAL).map(|r| r.source()),
            Some(SOURCE_MCP_OFFICIAL)
        );
        assert_eq!(
            registry_for_source(SOURCE_SMITHERY).map(|r| r.source()),
            Some(SOURCE_SMITHERY)
        );
        assert!(registry_for_source("nope").is_none());
    }
}
