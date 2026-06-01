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

/// Query the per-source summary trees. The global (time-axis) and topic
/// (subject-axis) trees were removed; source trees plus the entity index are
/// the substrate, so this is the only remaining tree-query backend.
pub async fn query_source_scope(
    config: &Config,
    scope: Option<&str>,
    time_window_days: Option<u32>,
    query: Option<&str>,
    limit: usize,
) -> Result<QueryResponse> {
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
