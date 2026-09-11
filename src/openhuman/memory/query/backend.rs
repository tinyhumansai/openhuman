//! High-level memory query backend.
//!
//! This module is the orchestration-facing read surface over the summary tree.
//! It deliberately lives under `memory/query` rather than `memory_tree/tree`
//! so the tree module can stay focused on generic structure, policy,
//! summarisation, and read/write mechanics.
//!
//! # Everything here goes through the bound driver
//!
//! These were direct calls into `tinymemory_core::tree::retrieval`, which
//! opened the workspace store in this process. They now resolve the guarded
//! driver and use the `MemoryRetrieval` family, so the loaded module is the
//! only reader — see `docs/specs/2026-08-13-memory-module-port.md` §2.1.
//!
//! `None` is passed for every `scope` argument, and that is not "unrestricted":
//! the guard intersects it with the ambient per-turn allowlist before the call
//! reaches the driver, so naming a scope here could only ever narrow what the
//! turn may see.

use anyhow::Result;

use crate::openhuman::memory::api::chunks::SourceKind;
use crate::openhuman::memory::api::provider::{
    MemoryProvider, RetrievalHit, RetrievalResponse, SourceRetrievalQuery,
};
use crate::openhuman::memory::guard::MemoryGuard;
use crate::openhuman::memory::ops::guard::active_memory_guard;

/// The retrieval family on the active driver, or a caller-facing error.
async fn retrieval() -> Result<std::sync::Arc<MemoryGuard>> {
    let guard = active_memory_guard()
        .await
        .map_err(|e| anyhow::anyhow!("memory query: {e}"))?;
    if guard.as_retrieval().is_none() {
        return Err(anyhow::anyhow!(
            "memory query: memory driver does not support the retrieval family"
        ));
    }
    Ok(guard)
}

/// Query the per-source summary trees. The global (time-axis) and topic
/// (subject-axis) trees were removed; source trees plus the entity index are
/// the substrate, so this is the only remaining tree-query backend.
pub async fn query_source_scope(
    scope: Option<&str>,
    time_window_days: Option<u32>,
    query: Option<&str>,
    limit: usize,
) -> Result<RetrievalResponse> {
    let guard = retrieval().await?;
    let request = SourceRetrievalQuery {
        source_id: scope.map(str::to_string),
        source_kind: None,
        time_window_days,
        query: query.map(str::to_string),
        limit,
    };
    Ok(guard
        .as_retrieval()
        .expect("checked above")
        .retrieve_source(&request, None)
        .await?)
}

pub async fn query_source_kind(
    source_kind: Option<SourceKind>,
    time_window_days: Option<u32>,
    query: Option<&str>,
    limit: usize,
) -> Result<RetrievalResponse> {
    let guard = retrieval().await?;
    let request = SourceRetrievalQuery {
        source_id: None,
        source_kind,
        time_window_days,
        query: query.map(str::to_string),
        limit,
    };
    Ok(guard
        .as_retrieval()
        .expect("checked above")
        .retrieve_source(&request, None)
        .await?)
}

pub async fn drill_down(
    node_id: &str,
    max_depth: u32,
    query: Option<&str>,
    limit: Option<usize>,
) -> Result<Vec<RetrievalHit>> {
    let guard = retrieval().await?;
    Ok(guard
        .as_retrieval()
        .expect("checked above")
        // `None` here is not "unrestricted": the guard resolves the ambient
        // task-local scope for a caller that names none, and forwards it
        // explicitly. This is host-side code, so the task-local is present.
        .retrieve_children(node_id, max_depth, query, limit, None)
        .await?)
}

pub async fn fetch_leaves(chunk_ids: &[String]) -> Result<Vec<RetrievalHit>> {
    let guard = retrieval().await?;
    Ok(guard
        .as_retrieval()
        .expect("checked above")
        .retrieve_leaves(chunk_ids, None)
        .await?)
}
