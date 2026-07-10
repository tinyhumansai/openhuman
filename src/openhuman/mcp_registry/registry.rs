//! Multi-registry search + detail dispatch.
//!
//! `registry_search` fans out to every enabled registry in parallel and
//! over-fetches to offset the strict "perfect server" filter, then merges +
//! dedups, badges the canonical vendor server, drops non-perfect rows, applies
//! the transport filter, and floats official servers to the top. (A previous
//! attempt to fetch + cache the *entire* catalog up front was reverted: the full
//! cursor walk exceeded the 30 s RPC budget and blanked the page.)
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

/// Over-fetch factor for the strict catalog. The "perfect server" filter drops
/// most upstream rows, so fetching exactly `page_size` would return a
/// near-empty page. We pull a multiple of the requested size so each returned
/// page lands close to `page_size` usable rows.
const STRICT_OVERFETCH_FACTOR: u32 = 5;
/// Cap on the per-page upstream fetch (registry list endpoints cap `limit`
/// around here anyway).
const MAX_FETCH_PAGE_SIZE: u32 = 100;

/// Raw rows to request upstream for a requested `page_size` of strict results.
fn strict_fetch_size(page_size: u32) -> u32 {
    page_size
        .saturating_mul(STRICT_OVERFETCH_FACTOR)
        .clamp(1, MAX_FETCH_PAGE_SIZE)
}

/// Per-registry search results tagged with their source id, for [`merge_registry_results`].
type LabelledResults = Vec<(&'static str, Result<(Vec<SmitheryServerSummary>, u32)>)>;

/// Search every enabled registry in parallel, over-fetch, merge, then curate.
/// `query` is passed to the registries' own search; `transport` (`"stdio"` |
/// `"hosted"` | `"all"`/`None`) filters the merged rows by how they run.
pub async fn registry_search(
    config: &Config,
    query: Option<&str>,
    transport: Option<&str>,
    page: u32,
    page_size: u32,
) -> Result<(Vec<SmitheryServerSummary>, u32)> {
    let registries = enabled_registries(config);
    let fetch_size = strict_fetch_size(page_size);
    let results = join_all(
        registries
            .iter()
            .map(|r| r.search(config, query, page, fetch_size)),
    )
    .await;

    // A total outage (every registry errored) is distinct from "no perfect
    // servers": return an error so the UI shows its registry error state instead
    // of an empty catalog. `merge_registry_results` logs+skips the individual
    // failures.
    let any_ok = results.iter().any(Result::is_ok);
    let labelled = results
        .into_iter()
        .enumerate()
        .map(|(idx, res)| (registries[idx].source(), res))
        .collect();

    let (mut merged, mut total_pages) = merge_registry_results(labelled);

    if !any_ok && !registries.is_empty() {
        anyhow::bail!("all MCP registries failed to respond");
    }

    // Badge the canonical first-party server, drop non-perfect rows, refine
    // search relevance, filter by transport, float official to the top.
    super::curation::tag_official(&mut merged);
    super::curation::retain_perfect_servers(&mut merged);
    if let Some(q) = query.map(str::trim).filter(|q| !q.is_empty()) {
        merged = refine_by_relevance(merged, q);
    }
    apply_transport(&mut merged, transport);
    super::curation::float_official_first(&mut merged);

    if total_pages == 0 {
        total_pages = page.max(1);
    }
    tracing::debug!(
        "[mcp-registry] search page={page} returned={} total_pages={total_pages} has_query={} transport={:?}",
        merged.len(),
        query.map(|q| !q.trim().is_empty()).unwrap_or(false),
        transport
    );
    Ok((merged, total_pages))
}

/// Drop rows that don't match the requested transport (`"stdio"` | `"hosted"`).
/// `None`/`"all"` keeps everything.
fn apply_transport(servers: &mut Vec<SmitheryServerSummary>, transport: Option<&str>) {
    if let Some(tp) = transport
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != "all")
    {
        servers.retain(|s| match tp {
            "hosted" => s.is_deployed,
            "stdio" => !s.is_deployed,
            _ => true,
        });
    }
}

/// Strip a code-host namespace root so a query matches the meaningful part of a
/// slug, not the shared host. `io.github.06ketan/medium-ops` → `06ketan/medium-ops`.
fn searchable_slug(qualified_name: &str) -> &str {
    const CODE_HOST_PREFIXES: &[&str] = &["io.github.", "io.gitlab."];
    for prefix in CODE_HOST_PREFIXES {
        if let Some(rest) = qualified_name.strip_prefix(prefix) {
            return rest;
        }
    }
    qualified_name
}

/// Keep only rows matching every query token across the display name, the
/// namespace-stripped slug, and the description — so searching "github" doesn't
/// match every `io.github.<user>/*` community server just by its namespace.
/// Safe-by-default: if the refinement would empty the page, the unrefined rows
/// are returned (a looser list beats a blank one).
fn refine_by_relevance(
    servers: Vec<SmitheryServerSummary>,
    query: &str,
) -> Vec<SmitheryServerSummary> {
    let tokens: Vec<String> = query
        .to_lowercase()
        .split_whitespace()
        .map(String::from)
        .collect();
    if tokens.is_empty() {
        return servers;
    }
    let refined: Vec<SmitheryServerSummary> = servers
        .iter()
        .filter(|s| {
            let haystack = format!(
                "{} {} {}",
                s.display_name.to_lowercase(),
                searchable_slug(&s.qualified_name).to_lowercase(),
                s.description.as_deref().unwrap_or("").to_lowercase()
            );
            tokens.iter().all(|t| haystack.contains(t.as_str()))
        })
        .cloned()
        .collect();
    if refined.is_empty() {
        servers
    } else {
        refined
    }
}

/// Merge per-registry search results into one list, dropping exact
/// `qualified_name` duplicates. Registries are passed in priority order
/// (official before Smithery), and the first occurrence of a slug wins.
/// `total_pages` is the max reported across registries that succeeded; failed
/// registries are logged and skipped so one flaky upstream can't blank results.
fn merge_registry_results(results: LabelledResults) -> (Vec<SmitheryServerSummary>, u32) {
    let mut merged: Vec<SmitheryServerSummary> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();
    let mut total_pages: u32 = 0;

    for (source, res) in results {
        match res {
            Ok((servers, pages)) => {
                for server in servers {
                    if seen.insert(server.qualified_name.clone()) {
                        merged.push(server);
                    }
                }
                total_pages = total_pages.max(pages);
            }
            Err(err) => {
                tracing::warn!("[mcp-registry] {source} search failed: {err}");
            }
        }
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
            website_url: None,
            auth_kind: None,
            extra: Default::default(),
        }
    }

    #[test]
    fn strict_fetch_size_overfetches_and_caps() {
        assert_eq!(strict_fetch_size(10), 50);
        assert_eq!(strict_fetch_size(30), MAX_FETCH_PAGE_SIZE);
        assert_eq!(strict_fetch_size(1_000_000), MAX_FETCH_PAGE_SIZE);
        assert_eq!(strict_fetch_size(0), 1);
    }

    #[test]
    fn merge_keeps_higher_priority_duplicate_and_drops_the_rest() {
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
        assert_eq!(
            merged
                .iter()
                .find(|s| s.qualified_name == "dup/server")
                .unwrap()
                .source,
            "mcp_official"
        );
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

    #[test]
    fn apply_transport_filters_by_run_kind() {
        let mut servers = vec![
            SmitheryServerSummary {
                is_deployed: true,
                ..summary("a/hosted", "mcp_official")
            },
            summary("b/stdio", "mcp_official"),
        ];

        let mut hosted = servers.clone();
        apply_transport(&mut hosted, Some("hosted"));
        assert_eq!(hosted.len(), 1);
        assert_eq!(hosted[0].qualified_name, "a/hosted");

        apply_transport(&mut servers, Some("stdio"));
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].qualified_name, "b/stdio");
    }

    #[test]
    fn refine_excludes_namespace_only_matches_but_never_empties() {
        let real = SmitheryServerSummary {
            display_name: "GitHub".to_string(),
            description: Some("Official GitHub MCP server".to_string()),
            ..summary("io.github.github/github-mcp-server", "mcp_official")
        };
        let community = SmitheryServerSummary {
            display_name: "medium-ops".to_string(),
            description: Some("Medium CLI. No API keys.".to_string()),
            ..summary("io.github.06ketan/medium-ops", "mcp_official")
        };

        // "github" keeps the real one, drops the namespace-only community row.
        let hits = refine_by_relevance(vec![real.clone(), community.clone()], "github");
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].qualified_name, "io.github.github/github-mcp-server");

        // A query that matches nothing returns the unrefined rows (never blank).
        let none = refine_by_relevance(vec![community.clone()], "github");
        assert_eq!(none.len(), 1);
    }
}
