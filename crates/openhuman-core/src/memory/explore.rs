//! The memory explorer: `memory_explore` and `memory_items_get`.
//!
//! An explorer walks what memory holds by TinyMemory's standard facets (kind,
//! source, workspace, folder, file, thread, agent, tool call, tag, …; see
//! `tinymemory_api::explore`). The client keeps only a breadcrumb [`PathStep`]
//! list; the core turns it into a filter with `Facet::narrow`, so drilling
//! down means exactly the same thing in every client and for every engine:
//!
//! 1. `memory_explore {facet, path}` counts the items under `path` per value
//!    of `facet`;
//! 2. picking a bucket appends `{facet, value}` to `path`;
//! 3. `memory_items_list {path}` pages through the items there, and
//!    `memory_items_get {ids}` reads one whole.

use serde::{Deserialize, Serialize};
use tinymemory_api::{ExplorePage, Facet, Hit, ItemId, MetaFilter, Result};
use tinymemory_tools::ops::ExploreOptions;

use crate::config::Config;

use super::engine;
pub use tinymemory_tools::ops::{narrowed, PathStep, DEFAULT_BUCKETS, MAX_PATH_STEPS};

/// `memory_explore` params.
#[derive(Debug, Clone, Deserialize)]
pub struct ExploreParams {
    /// The facet to group by.
    pub facet: Facet,
    /// Where in the explorer: each step narrows the items grouped.
    #[serde(default)]
    pub path: Vec<PathStep>,
    /// An extra metadata filter, applied before the path.
    #[serde(default)]
    pub filter: Option<MetaFilter>,
    /// Most buckets (default [`DEFAULT_BUCKETS`]).
    #[serde(default)]
    pub limit: Option<usize>,
    /// Most items a listing-based engine reads.
    #[serde(default)]
    pub scan_limit: Option<usize>,
}

/// `memory_items_get` params.
#[derive(Debug, Clone, Deserialize)]
pub struct ItemsGetParams {
    /// Item ids.
    pub ids: Vec<String>,
    /// Only items in this reach are returned; unset reads every namespace.
    #[serde(default)]
    pub reach: Option<tinymemory_api::Reach>,
}

/// `memory_items_get` result.
#[derive(Debug, Clone, Serialize)]
pub struct ItemsGetView {
    /// The items found, in the order asked; unknown ids are left out.
    pub items: Vec<Hit>,
}

/// `memory_explore`.
pub async fn explore(config: &Config, params: ExploreParams) -> Result<ExplorePage> {
    let bound = engine::resolve(config).await.engine()?;
    let facet = params.facet;
    let depth = params.path.len();
    let filter = narrowed(params.filter, &params.path)?;
    let page = super::ops::memory_ops(config)?
        .explore(
            bound.engine.as_ref(),
            ExploreOptions {
                facet: params.facet,
                path: Vec::new(),
                filter: Some(filter),
                limit: params.limit,
                scan_limit: params.scan_limit,
            },
        )
        .await?;
    tracing::debug!(
        engine = %bound.id,
        facet = facet.as_str(),
        depth,
        buckets = page.buckets.len(),
        total = page.total,
        truncated = page.truncated,
        "[memory:explore] explored"
    );
    Ok(page)
}

/// `memory_items_get`.
pub async fn items_get(config: &Config, params: ItemsGetParams) -> Result<ItemsGetView> {
    let bound = engine::resolve(config).await.engine()?;
    let ids: Vec<ItemId> = params.ids.into_iter().map(ItemId::new).collect();
    let asked = ids.len();
    let items = super::ops::memory_ops(config)?
        .items_get(bound.engine.as_ref(), ids, params.reach)
        .await?;
    tracing::debug!(
        engine = %bound.id,
        asked,
        found = items.len(),
        "[memory:explore] items read"
    );
    Ok(ItemsGetView { items })
}

#[cfg(test)]
#[path = "explore_tests.rs"]
mod tests;
