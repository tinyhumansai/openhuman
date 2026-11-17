//! Query matching, stable curation ranking, and fallback scoring for live catalog search.

use crate::config::Config;
use serde_json::{json, Value};

/// Cap on returned matches so a broad query can't flood the agent's context.
pub(super) const MAX_CATALOG_RESULTS: usize = 40;

/// Search the FULL LIVE Composio catalog (via
/// [`crate::integrations::composio::catalog::fetch_live_toolkit_catalog`]) for
/// actions whose slug or description matches every whitespace-separated term
/// in `query` (case-insensitive AND). When `toolkit` is set, only that
/// toolkit is scanned — this is how the builder can search ANY named app
/// (connected or not) rather than only the toolkits already
/// `crate::integrations::composio::contract::agent_ready_toolkits`;
/// with no `toolkit` filter, the search is scoped to that agent-ready set (a
/// bare keyword query with no app named would otherwise have to fan out to
/// every toolkit Composio knows about).
///
/// Curated matches (`is_curated`) are ranked first (a stable sort, so ties
/// preserve fetch order) — never filtered out; a real, uncurated action is
/// just as valid a result, only ranked after the curated ones. A toolkit
/// whose live-catalog fetch fails (no backend session, network error)
/// contributes zero results rather than erroring the whole search.
pub(crate) async fn search_live_catalog(
    config: &Config,
    query: &str,
    toolkit_filter: Option<&str>,
    limit: usize,
) -> Vec<Value> {
    search_catalog(config, query, toolkit_filter, limit)
        .await
        .results
}

/// Cap on fallback (per-keyword) matches — a near-miss query must not flood the
/// agent's context with the whole toolkit, so the OR-scored fallback returns at
/// most this many rows regardless of the primary `limit`.
const MAX_FALLBACK_RESULTS: usize = 10;

/// Outcome of a catalog search: the shaped rows, whether the per-keyword
/// fallback pass fired, and an optional advisory `note` the tool surfaces so an
/// agent never misreads a keyword miss as "the action doesn't exist".
pub(crate) struct CatalogSearchOutcome {
    pub results: Vec<Value>,
    /// True when the per-token OR fallback pass ran (primary AND match was
    /// empty for a multi-word query).
    pub fallback: bool,
    /// Advisory note explaining a near-miss / keyword-based search, if any.
    pub note: Option<String>,
}

/// Shape one live-catalog [`ToolContract`](crate::integrations::composio::catalog::ToolContract)
/// into a search-result row. The SINGLE row-construction site shared by both
/// the primary AND-match path and the per-keyword fallback path, so every row
/// carries the same fields — including WS3's `runtime_gated: true` on an
/// uncurated action of a toolkit that ships a curated-only allowlist.
fn shape_catalog_row(
    tool: &crate::integrations::composio::catalog::ToolContract,
    toolkit: &str,
    toolkit_curated: bool,
) -> Value {
    let mut row = json!({
        "slug": tool.slug,
        "toolkit": toolkit,
        "description": tool.description,
        "required_args": tool.required_args,
        "output_fields": tool.output_fields,
        "primary_array_path": tool.primary_array_path,
        "featured": tool.is_curated,
    });
    // Compact: only present when true.
    if !tool.is_curated && toolkit_curated {
        if let Some(obj) = row.as_object_mut() {
            obj.insert("runtime_gated".to_string(), Value::Bool(true));
        }
    }
    row
}

/// Search the FULL LIVE Composio catalog and return a [`CatalogSearchOutcome`].
///
/// Primary pass: case-insensitive AND — an action matches only if EVERY
/// whitespace-separated term substring-matches its slug, toolkit name, or
/// description (curated matches ranked first, stable sort preserves fetch
/// order). When that yields zero rows for a MULTI-WORD query, a per-keyword OR
/// fallback runs: each action is scored by how many query tokens match its
/// slug/toolkit/description, and the top [`MAX_FALLBACK_RESULTS`] (ranked by
/// hit-count desc, then curated first) are returned with an advisory `note`.
/// This is what keeps a natural-language query like "twitter tweet replies
/// lookup" from returning a bare `count: 0` even though `TWITTER_*` actions
/// exist — the agent gets the nearest keyword matches instead of falsely
/// concluding the action is missing.
pub(crate) async fn search_catalog(
    config: &Config,
    query: &str,
    toolkit_filter: Option<&str>,
    limit: usize,
) -> CatalogSearchOutcome {
    use crate::integrations::composio::catalog::fetch_live_toolkit_catalog;
    // Contract crate — same item the `memory::sync::composio::providers` shim
    // re-exported; see `ListConnectableToolkitsTool::execute` for why (#5560).
    use crate::integrations::composio::contract::agent_ready_toolkits;

    // Curated-catalog lookups below read the module's process snapshot.
    // A failure leaves the lookups reading every toolkit as uncurated, which
    // the search already handles (it falls back to the live catalog).
    let _ = crate::integrations::composio::module_client::ensure_catalogs(config).await;

    let terms: Vec<String> = query
        .split_whitespace()
        .map(|t| t.to_ascii_lowercase())
        .collect();

    // The host's offered-toolkit policy also bounds an explicit search: a denied
    // toolkit's actions are not shown to the agent by naming it.
    let offered = tinyconnectors_bus::vocabulary::OfferedToolkits {
        allow: config.composio.offered_toolkits_allow.clone(),
        deny: config.composio.offered_toolkits_deny.clone(),
    };
    let toolkits: Vec<String> = match toolkit_filter {
        Some(tk) if !tk.trim().is_empty() => {
            let toolkit = tk.trim().to_ascii_lowercase();
            if offered.permits(&toolkit) {
                vec![toolkit]
            } else {
                tracing::debug!(%toolkit, "[flows] catalog search skipped: toolkit not offered");
                Vec::new()
            }
        }
        _ => agent_ready_toolkits(),
    };

    // Fetch every candidate toolkit's live catalog concurrently — a bare
    // keyword query (no `toolkit` filter) fans out across every agent-ready
    // toolkit, and fetching them one at a time would pay for each one's
    // round trip back-to-back (the per-toolkit cache only helps repeats).
    let fetched: Vec<(
        String,
        Option<Vec<crate::integrations::composio::catalog::ToolContract>>,
    )> = futures::future::join_all(toolkits.into_iter().map(|toolkit| async move {
        let catalog = fetch_live_toolkit_catalog(config, &toolkit).await;
        (toolkit, catalog)
    }))
    .await;

    // Drop toolkits whose fetch failed (no backend session / network error) —
    // they contribute zero results rather than erroring the whole search.
    let fetched: Vec<(
        String,
        Vec<crate::integrations::composio::catalog::ToolContract>,
    )> = fetched
        .into_iter()
        .filter_map(|(tk, catalog)| catalog.map(|c| (tk, c)))
        .collect();

    // Does the scanned scope hold ANY actions at all? Distinguishes "keyword
    // miss" (has actions, none matched) from "nothing to search" (empty scope).
    let any_actions = fetched.iter().any(|(_, catalog)| !catalog.is_empty());

    // ── Primary pass: case-insensitive AND across every term ──
    let mut matches: Vec<(bool, Value)> = Vec::new();
    for (toolkit, catalog) in &fetched {
        // WS3 — a toolkit that ships a curated catalog is a hard curated-only
        // allowlist at RUNTIME, so any `featured: false` action of it is
        // rejected on every real run. Compute once per toolkit and flag those
        // rows so the blocker is visible at search time (transcript failure #2).
        let toolkit_curated = crate::integrations::composio::catalog::has_curated_catalog(toolkit);
        for tool in catalog {
            let slug_lc = tool.slug.to_ascii_lowercase();
            let desc_lc = tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let is_match = terms.iter().all(|term| {
                slug_lc.contains(term) || toolkit.contains(term) || desc_lc.contains(term)
            });
            if !is_match {
                continue;
            }
            matches.push((
                tool.is_curated,
                shape_catalog_row(tool, toolkit, toolkit_curated),
            ));
        }
    }

    // Curated (`featured`) results first; stable sort preserves fetch order
    // within each group.
    matches.sort_by_key(|(is_curated, _)| std::cmp::Reverse(*is_curated));
    matches.truncate(limit);
    let primary: Vec<Value> = matches.into_iter().map(|(_, v)| v).collect();

    if !primary.is_empty() {
        return CatalogSearchOutcome {
            results: primary,
            fallback: false,
            note: None,
        };
    }

    // ── Zero primary hits ──
    // Single-token queries keep today's behavior exactly; only attach a light
    // advisory note so a lone keyword miss still explains the search is
    // keyword-based (task WS5.4, optional).
    if terms.len() <= 1 {
        let note = if any_actions {
            Some(format!(
                "No actions matched '{query}'. This search is keyword-based (matches action \
                 slug/name/description) — try a different single keyword (e.g. 'gmail' or \
                 'tweets')."
            ))
        } else {
            None
        };
        return CatalogSearchOutcome {
            results: Vec::new(),
            fallback: false,
            note,
        };
    }

    // ── Fallback pass (multi-word, zero primary hits): per-token OR scoring ──
    // Score each action by how many DISTINCT query tokens match its
    // slug/toolkit/description; keep the primary path's curated boost as the
    // tiebreak. Rows go through the SAME `shape_catalog_row` path as primary.
    let mut scored: Vec<(usize, bool, Value)> = Vec::new();
    for (toolkit, catalog) in &fetched {
        let toolkit_curated = crate::integrations::composio::catalog::has_curated_catalog(toolkit);
        for tool in catalog {
            let slug_lc = tool.slug.to_ascii_lowercase();
            let desc_lc = tool
                .description
                .as_deref()
                .unwrap_or_default()
                .to_ascii_lowercase();
            let hits = terms
                .iter()
                .filter(|term| {
                    slug_lc.contains(*term) || toolkit.contains(*term) || desc_lc.contains(*term)
                })
                .count();
            if hits == 0 {
                continue;
            }
            scored.push((
                hits,
                tool.is_curated,
                shape_catalog_row(tool, toolkit, toolkit_curated),
            ));
        }
    }

    // Most keyword hits first, then curated first; stable sort preserves fetch
    // order within a (hits, curated) group.
    scored.sort_by_key(|(hits, is_curated, _)| std::cmp::Reverse((*hits, *is_curated)));
    scored.truncate(limit.min(MAX_FALLBACK_RESULTS));
    let results: Vec<Value> = scored.into_iter().map(|(_, _, v)| v).collect();

    tracing::debug!(
        target: "flows",
        query,
        fallback = true,
        hits = results.len(),
        "[flows] search_tool_catalog: primary AND-match empty for a multi-word query — ran per-keyword OR fallback"
    );

    if results.is_empty() {
        // Literally zero tokens matched anything: no rows, but a note so the
        // agent doesn't read `count: 0` as "action doesn't exist" (task WS5.3).
        return CatalogSearchOutcome {
            results,
            fallback: true,
            note: Some(format!(
                "No actions matched any keyword in '{query}'. This search is keyword-based \
                 (matches action slug/name/description) — retry with a single keyword (e.g. one \
                 word like 'gmail' or 'tweets') for a full listing."
            )),
        };
    }

    CatalogSearchOutcome {
        results,
        fallback: true,
        note: Some(format!(
            "No exact match for '{query}'. Showing the nearest per-keyword matches — retry with a \
             single keyword (e.g. one word like 'gmail' or 'tweets') for a full listing."
        )),
    }
}
