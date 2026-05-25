//! Multi-registry dispatch entry point.
//!
//! `registry_search` fans out to every registry in
//! [`super::registries::enabled_registries`], runs them in parallel, and
//! returns merged results (failed registries are logged and skipped so one
//! flaky upstream doesn't blank the UI).
//!
//! `registry_get` routes by [`super::types::SmitheryServerDetail::source`].
//! The caller can pass an explicit source prefix using
//! `"<source>::<qualified_name>"` (e.g. `"mcp_official::io.github.foo/bar"`).
//! Without a prefix we ask every registry and return the first hit.

use anyhow::Result;
use futures::future::join_all;

use crate::openhuman::config::Config;

use super::registries::{enabled_registries, registry_for_source};
use super::types::{SmitheryServerDetail, SmitheryServerSummary};

const SOURCE_SEPARATOR: &str = "::";

/// Search every enabled registry in parallel; merge results. `total_pages`
/// is the max page count reported across registries (best-effort upper
/// bound).
pub async fn registry_search(
    config: &Config,
    query: Option<&str>,
    page: u32,
    page_size: u32,
) -> Result<(Vec<SmitheryServerSummary>, u32)> {
    let registries = enabled_registries();
    let queries = registries
        .iter()
        .map(|r| r.search(config, query, page, page_size));
    let results = join_all(queries).await;

    let mut merged: Vec<SmitheryServerSummary> = Vec::new();
    let mut total_pages: u32 = 0;
    for (idx, res) in results.into_iter().enumerate() {
        let source = registries[idx].source();
        match res {
            Ok((mut servers, pages)) => {
                tracing::debug!(
                    "[mcp-registry] {source} search ok servers={} pages={pages}",
                    servers.len()
                );
                merged.append(&mut servers);
                total_pages = total_pages.max(pages);
            }
            Err(err) => {
                tracing::warn!("[mcp-registry] {source} search failed: {err}");
            }
        }
    }

    if total_pages == 0 {
        total_pages = page.max(1);
    }
    Ok((merged, total_pages))
}

/// Fetch a server detail. If `qualified_name` starts with `"<source>::"` we
/// route directly to that registry; otherwise every enabled registry is
/// tried in order and the first success wins.
pub async fn registry_get(config: &Config, qualified_name: &str) -> Result<SmitheryServerDetail> {
    if let Some((source, rest)) = qualified_name.split_once(SOURCE_SEPARATOR) {
        if let Some(registry) = registry_for_source(source) {
            tracing::debug!("[mcp-registry] get routed source={source} qualified={rest}");
            return registry.get(config, rest).await;
        }
        tracing::warn!(
            "[mcp-registry] get: unknown source prefix {source:?} — falling back to all registries"
        );
    }

    let mut last_err: Option<anyhow::Error> = None;
    for registry in enabled_registries() {
        match registry.get(config, qualified_name).await {
            Ok(detail) => return Ok(detail),
            Err(err) => {
                tracing::debug!(
                    "[mcp-registry] {} get miss for {qualified_name}: {err}",
                    registry.source()
                );
                last_err = Some(err);
            }
        }
    }
    Err(last_err.unwrap_or_else(|| anyhow::anyhow!("no registries enabled")))
}
