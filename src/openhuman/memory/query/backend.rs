//! High-level memory query backend.
//!
//! This module is the orchestration-facing read surface over the summary tree.
//! It deliberately lives under `memory/query` rather than `memory_tree/tree`
//! so the tree module can stay focused on generic structure, policy,
//! summarisation, and read/write mechanics.

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_tree::retrieval::{self, QueryResponse, RetrievalHit};
use crate::openhuman::memory_tree::tree::TreeProfile;

pub async fn query_profile(
    config: &Config,
    profile: TreeProfile,
    scope: Option<&str>,
    time_window_days: Option<u32>,
    query: Option<&str>,
    limit: usize,
) -> Result<QueryResponse> {
    match profile {
        TreeProfile::Source => {
            retrieval::source::query_source(
                config,
                scope,
                None::<SourceKind>,
                time_window_days,
                query,
                limit,
            )
            .await
        }
        TreeProfile::Topic => {
            let entity_id =
                scope.ok_or_else(|| anyhow::anyhow!("topic query requires scope/entity_id"))?;
            retrieval::topic::query_topic(config, entity_id, time_window_days, query, limit).await
        }
        TreeProfile::Global => {
            retrieval::global::query_global(config, time_window_days.unwrap_or(7)).await
        }
    }
}

pub async fn query_source_kind(
    config: &Config,
    source_kind: Option<SourceKind>,
    time_window_days: Option<u32>,
    query: Option<&str>,
    limit: usize,
) -> Result<QueryResponse> {
    retrieval::source::query_source(config, None, source_kind, time_window_days, query, limit).await
}

pub async fn drill_down(
    config: &Config,
    node_id: &str,
    max_depth: u32,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<RetrievalHit>> {
    retrieval::drill_down::drill_down(config, node_id, max_depth, query, limit).await
}

pub async fn fetch_leaves(config: &Config, chunk_ids: &[String]) -> Result<Vec<RetrievalHit>> {
    retrieval::fetch::fetch_leaves(config, chunk_ids).await
}
