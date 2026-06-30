//! Multi-registry dispatch entry point.
//!
//! `registry_search` fans out to every registry in
//! [`super::registries::enabled_registries`], runs them in parallel, and
//! returns merged results (failed registries are logged and skipped so one
//! flaky upstream doesn't blank the UI). The official modelcontextprotocol.io
//! registry is listed first so its results take priority.
//!
//! `registry_get` routes by [`super::types::SmitheryServerDetail::source`].
//! The caller can pass an explicit source prefix using
//! `"<source>::<qualified_name>"` (e.g. `"mcp_official::io.github.foo/bar"`).
//! Without a prefix we ask every registry and return the first hit.

use std::collections::HashSet;

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
    let registries = enabled_registries(config);
    let queries = registries
        .iter()
        .map(|r| r.search(config, query, page, page_size));
    let results = join_all(queries).await;

    let labelled = results
        .into_iter()
        .enumerate()
        .map(|(idx, res)| (registries[idx].source(), res))
        .collect();

    let (mut merged, mut total_pages) = merge_registry_results(labelled);

    // Keep the full deduped catalog browsable — no one-per-service collapse
    // (it hides genuinely different community servers and barely trims noise).
    // Just badge the canonical first-party server for each known service so the
    // official one is easy to spot without throwing any alternatives away.
    super::curation::tag_official(&mut merged);

    if total_pages == 0 {
        total_pages = page.max(1);
    }
    Ok((merged, total_pages))
}

/// Merge per-registry search results into one list, dropping exact
/// `qualified_name` duplicates. Registries are passed in priority order
/// (official before Smithery), and the first occurrence of a slug wins — so a
/// package listed on both registries collapses to the higher-priority copy and
/// the UI never shows the same slug twice. `total_pages` is the max reported
/// across the registries that succeeded. Failed registries are logged and
/// skipped so one flaky upstream can't blank the catalog.
fn merge_registry_results(
    results: Vec<(&'static str, Result<(Vec<SmitheryServerSummary>, u32)>)>,
) -> (Vec<SmitheryServerSummary>, u32) {
    let mut merged: Vec<SmitheryServerSummary> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_pages: u32 = 0;
    let mut dropped: usize = 0;

    for (source, res) in results {
        match res {
            Ok((servers, pages)) => {
                tracing::debug!(
                    "[mcp-registry] {source} search ok servers={} pages={pages}",
                    servers.len()
                );
                for server in servers {
                    if seen.insert(server.qualified_name.clone()) {
                        merged.push(server);
                    } else {
                        dropped += 1;
                    }
                }
                total_pages = total_pages.max(pages);
            }
            Err(err) => {
                tracing::warn!("[mcp-registry] {source} search failed: {err}");
            }
        }
    }

    if dropped > 0 {
        tracing::debug!("[mcp-registry] dropped {dropped} cross-registry duplicate slug(s)");
    }
    (merged, total_pages)
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
    for registry in enabled_registries(config) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn summary(qualified_name: &str, source: &str) -> SmitheryServerSummary {
        SmitheryServerSummary {
            qualified_name: qualified_name.to_string(),
            display_name: qualified_name.to_string(),
            description: None,
            icon_url: None,
            use_count: 0,
            is_deployed: false,
            source: source.to_string(),
            official: false,
            extra: Default::default(),
        }
    }

    #[test]
    fn merge_keeps_higher_priority_duplicate_and_drops_the_rest() {
        // `dup/server` is listed on both registries; official is passed first.
        let results = vec![
            (
                "mcp_official",
                Ok((
                    vec![
                        summary("dup/server", "mcp_official"),
                        summary("off/only", "mcp_official"),
                    ],
                    3,
                )),
            ),
            (
                "smithery",
                Ok((
                    vec![
                        summary("dup/server", "smithery"),
                        summary("smi/only", "smithery"),
                    ],
                    5,
                )),
            ),
        ];

        let (merged, total_pages) = merge_registry_results(results);

        let slugs: Vec<_> = merged.iter().map(|s| s.qualified_name.as_str()).collect();
        assert_eq!(slugs, vec!["dup/server", "off/only", "smi/only"]);
        // The surviving duplicate is the official copy (first occurrence wins).
        let dup = merged
            .iter()
            .find(|s| s.qualified_name == "dup/server")
            .unwrap();
        assert_eq!(dup.source, "mcp_official");
        // total_pages is the max across registries.
        assert_eq!(total_pages, 5);
    }

    #[test]
    fn merge_skips_failed_registries_without_blanking_results() {
        let results = vec![
            ("mcp_official", Err(anyhow::anyhow!("upstream 500"))),
            ("smithery", Ok((vec![summary("smi/only", "smithery")], 2))),
        ];

        let (merged, total_pages) = merge_registry_results(results);

        assert_eq!(merged.len(), 1);
        assert_eq!(merged[0].qualified_name, "smi/only");
        assert_eq!(total_pages, 2);
    }
}
