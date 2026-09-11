//! JSON-RPC handler bodies for Phase 4 retrieval tools (#710).
//!
//! Shapes mirror the contract's — `RetrievalResponse` and `Vec<RetrievalHit>` /
//! `Vec<EntityMatch>` all serialise directly, without an extra envelope.
//!
//! # All five read through `MemoryRetrieval`
//!
//! Member for member: `retrieve_source`, `cover_window`, `retrieve_children`,
//! `retrieve_leaves`, `search_entities`. Nothing here reaches the engine.
//!
//! The four hit-returning handlers were held back while the pinned artifact was
//! TinyMemory v1.3.0, whose `RetrievalHit` had no `tree_kind`. The field is
//! `skip_serializing_if = "Option::is_none"`, so routing them then would have
//! decoded every hit's kind as `None` and dropped the key from four public
//! responses. v1.4.0 (tinymemory#95) carries it, and `modules::registry` plus
//! `ARTIFACT_CAPABILITIES_PIN` both name that release, so the loss is no longer
//! reachable: the engine's own `RetrievalHit::tree_kind` is a **non-optional**
//! `TreeKind`, the driver builds the contract's hit by serialising the engine's,
//! and a `#[serde(rename_all = "snake_case")]` enum against the contract's open
//! `String` encodes identically. Every hit that reaches this file therefore
//! carries `Some(kind)` and re-emits the same key with the same value
//! (`source`, `topic`, `global`, `flavoured`). A `None` would take a driver that
//! never ran the engine at all, and for that one the contract's reading — no
//! tree, so no kind to report — is why the key is omitted rather than sent as
//! `null`; inventing `source` there would be the silent-wrong-data case the
//! placeholder rule already rejects for summaries.
//!
//! `EntityMatch` is the same kind of identity: the engine's `kind` is a
//! snake_case enum where the contract's is an open `String`.
//!
//! The envelopes match field for field and in declaration order —
//! `RetrievalResponse` against the engine's `QueryResponse` (`hits`, `total`,
//! `truncated`) and `RetrievalHit` against the engine's (`node_id` through
//! `source_ref`) — so the JSON these handlers emit is byte-identical to what
//! they emitted while they called the engine directly. The frontend reads it.
//!
//! # The scope argument is the source gate, and is never a literal `None`
//!
//! `binding.provider()` is the **unguarded** driver: nothing between this file
//! and the store re-applies the per-profile `memory_sources` allowlist, so the
//! scope each call passes is the gate. The engine's `*_scoped` entry points
//! existed for the same reason — their ambient twins read the ENGINE's
//! task-local, which a separately compiled module cannot see, and an absent
//! scope means unrestricted, i.e. the gate failing open.
//!
//! Every call below passes `source_scope::as_bus_scope()`, which renders this
//! host's own task-local in the contract's vocabulary. `None` from it means
//! genuinely unrestricted and must stay `None`: an **empty** `SourceScope`
//! denies every source-attributed row, so mapping "no restriction" onto one
//! would invert the policy. A literal `None` written at the call site instead
//! is the open failure this seam exists to prevent.
//!
//! `search_entities` is the one member that takes no scope on either side — the
//! entity index is not source-attributed — so there is nothing to pass.

use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
// The contract's retrieval vocabulary, not the engine's: these handlers return
// what the driver handed back. The two encode identically (see the module
// docs), so this is a Rust-type change and not a wire one.
use crate::openhuman::memory::api::provider::retrieval::{
    CoverWindowQuery, EntityMatch, RetrievalHit, RetrievalResponse, SourceRetrievalQuery,
};
use crate::openhuman::memory::source_scope::as_bus_scope;
use crate::rpc::RpcOutcome;
use tinymemory_api::chunks::SourceKind;

// ── query_source ──────────────────────────────────────────────────────

/// Request body for `memory_tree_query_source`. All fields are optional;
/// see [`MemoryRetrieval::retrieve_source`] for selection semantics.
///
/// [`MemoryRetrieval::retrieve_source`]: crate::openhuman::memory::api::provider::retrieval::MemoryRetrieval::retrieve_source
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct QuerySourceRequest {
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub time_window_days: Option<u32>,
    /// Phase 4 (#710) — optional natural-language query string. When
    /// provided, candidates are reranked by cosine similarity to the
    /// query's embedding rather than sorted by recency. Legacy rows
    /// with no stored embedding fall to the bottom.
    #[serde(default)]
    pub query: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// JSON-RPC handler body for `memory_tree_query_source`. Parses the request,
/// reads through the bound driver's `MemoryRetrieval` family, and wraps the
/// outcome with a PII-redacted log line.
pub async fn query_source_rpc(
    config: &Config,
    req: QuerySourceRequest,
) -> Result<RpcOutcome<RetrievalResponse>, String> {
    // Parsed before the driver is resolved, so an unknown kind stays a caller
    // error naming the offending value rather than a driver round trip that
    // matches nothing and reads as an empty store.
    let source_kind = match req.source_kind.as_deref() {
        Some(s) => Some(SourceKind::parse(s).map_err(|e| format!("query_source: {e}"))?),
        None => None,
    };
    let query = SourceRetrievalQuery {
        source_id: req.source_id.clone(),
        source_kind,
        time_window_days: req.time_window_days,
        query: req.query.clone(),
        // 0 is the engine's "no caller preference" sentinel, not a request for
        // zero rows, and the driver forwards `limit` verbatim — so the
        // absent-limit default stays the engine's, exactly as it was on the
        // direct call.
        limit: req.limit.unwrap_or(0),
    };
    // The explicit scope, never `None` — see the module docs.
    let scope = as_bus_scope();

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let resp = match binding.provider().as_retrieval() {
        Some(retrieval) => retrieval
            .retrieve_source(&query, scope.as_ref())
            .await
            .map_err(|e| format!("query_source: {e}"))?,
        // Read-only, so an empty page is the honest answer: a driver with no
        // retrieval family keeps no summary tree to rank, which is a true
        // statement about it rather than a fault the caller can act on.
        None => {
            log::debug!(
                "[memory-tree][rpc] query_source: driver '{}' does not serve Retrieval; reporting empty",
                binding.driver_id()
            );
            RetrievalResponse::default()
        }
    };
    let n = resp.hits.len();
    // Omit scope / source_id from the log — can carry PII. Log counts only.
    Ok(RpcOutcome::single_log(
        resp,
        format!(
            "memory_tree: query_source has_source_id={} source_kind={:?} has_query={} hits={}",
            req.source_id.is_some(),
            req.source_kind,
            req.query.is_some(),
            n
        ),
    ))
}

// ── cover_window ──────────────────────────────────────────────────────

/// Request body for `memory_tree_cover_window`. `since_ms`/`until_ms` are the
/// inclusive window bounds in epoch-milliseconds; the source filter mirrors
/// `query_source`. See [`MemoryRetrieval::cover_window`] for cover semantics.
///
/// [`MemoryRetrieval::cover_window`]: crate::openhuman::memory::api::provider::retrieval::MemoryRetrieval::cover_window
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct CoverWindowRequest {
    pub since_ms: i64,
    pub until_ms: i64,
    #[serde(default)]
    pub source_id: Option<String>,
    #[serde(default)]
    pub source_kind: Option<String>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// JSON-RPC handler body for `memory_tree_cover_window`. Parses the request,
/// reads through the bound driver, logs PII-redacted counts.
///
/// An inverted window is the driver's rejection to make, not this handler's:
/// the bound engine already refuses one naming both bounds, and duplicating the
/// guard here would let the two disagree about what a valid window is.
pub async fn cover_window_rpc(
    config: &Config,
    req: CoverWindowRequest,
) -> Result<RpcOutcome<RetrievalResponse>, String> {
    log::debug!(
        "[rpc][memory_tree] cover_window enter since_ms={} until_ms={} has_source_id={} has_source_kind={} has_limit={}",
        req.since_ms,
        req.until_ms,
        req.source_id.is_some(),
        req.source_kind.is_some(),
        req.limit.is_some()
    );
    let source_kind = match req.source_kind.as_deref() {
        Some(s) => {
            log::trace!("[rpc][memory_tree] cover_window parse_source_kind");
            Some(SourceKind::parse(s).map_err(|e| format!("cover_window: {e}"))?)
        }
        None => None,
    };
    log::trace!(
        "[rpc][memory_tree] cover_window dispatch limit={:?}",
        req.limit
    );
    let window = CoverWindowQuery {
        since_ms: req.since_ms,
        until_ms: req.until_ms,
        source_id: req.source_id.clone(),
        source_kind,
        // Forwarded as the caller sent it: the driver maps an absent limit onto
        // the engine's 0 sentinel, which is what `None` has always meant here.
        limit: req.limit,
    };
    // The explicit scope, never `None` — see the module docs.
    let scope = as_bus_scope();

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let resp = match binding.provider().as_retrieval() {
        Some(retrieval) => retrieval
            .cover_window(&window, scope.as_ref())
            .await
            .map_err(|e| format!("cover_window: {e}"))?,
        // Read-only: a driver with no retrieval family has no nodes to cover
        // the window with, which is a true statement about it.
        None => {
            log::debug!(
                "[memory-tree][rpc] cover_window: driver '{}' does not serve Retrieval; reporting empty",
                binding.driver_id()
            );
            RetrievalResponse::default()
        }
    };
    let n = resp.hits.len();
    log::debug!(
        "[rpc][memory_tree] cover_window exit hits={} total={}",
        n,
        resp.total
    );
    // Omit scope / source_id from the log — can carry PII. Counts only.
    Ok(RpcOutcome::single_log(
        resp,
        format!(
            "memory_tree: cover_window since_ms={} until_ms={} has_source_id={} source_kind={:?} hits={}",
            req.since_ms,
            req.until_ms,
            req.source_id.is_some(),
            req.source_kind,
            n
        ),
    ))
}

// ── search_entities ───────────────────────────────────────────────────

/// Request body for `memory_tree_search_entities`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchEntitiesRequest {
    pub query: String,
    #[serde(default)]
    pub kinds: Option<Vec<String>>,
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Response envelope for `memory_tree_search_entities`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SearchEntitiesResponse {
    pub matches: Vec<EntityMatch>,
}

/// JSON-RPC handler body for `memory_tree_search_entities`. Validates the
/// optional `kinds` filter against [`EntityKind`], then reads the entity index
/// through the bound driver's `MemoryRetrieval` family.
pub async fn search_entities_rpc(
    config: &Config,
    req: SearchEntitiesRequest,
) -> Result<RpcOutcome<SearchEntitiesResponse>, String> {
    // Capture logging-friendly summary BEFORE we move fields out of `req`.
    let query_len = req.query.len();
    let has_kinds = req.kinds.is_some();
    // Passed through unparsed. This used to run `EntityKind::parse` here so an
    // unknown kind stayed a caller error naming the offending value rather than
    // a driver round trip that matched nothing and read as an empty index — the
    // ambiguity the contract requires this filter to be validated against.
    //
    // The driver already does exactly that: it parses each requested kind and
    // answers `MemoryError::Invalid("unknown entity kind: {kind}")`, naming the
    // value. So the host-side pass bought nothing but a compile-time link to
    // the engine's `EntityKind` (#5560) — and that enum is `#[non_exhaustive]`
    // and has grown twice, so a host-side copy of the vocabulary would drift
    // and start rejecting kinds the driver accepts. Validation belongs where
    // the vocabulary is defined.
    //
    // Parsing was also a no-op for valid input: it is exact-match, so `as_str`
    // re-emitted what a caller that got it right already sent.
    let kinds: Option<Vec<String>> = req.kinds;
    // 0 is the engine's "no caller preference" sentinel, not a request for zero
    // rows, and the driver forwards `limit` verbatim — so the absent-limit
    // default stays the engine's, exactly as it was on the direct call.
    let limit = req.limit.unwrap_or(0);

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let matches = match binding.provider().as_retrieval() {
        Some(retrieval) => retrieval
            .search_entities(&req.query, kinds.as_deref(), limit)
            .await
            .map_err(|e| format!("search_entities: {e}"))?,
        // Read-only, so an empty match list is the honest answer: a driver with
        // no retrieval family keeps no entity index to search, which is a true
        // statement about it rather than a fault the caller can act on.
        None => {
            log::debug!(
                "[memory-tree][rpc] search_entities: driver '{}' does not serve Retrieval; reporting empty",
                binding.driver_id()
            );
            Vec::new()
        }
    };
    let n = matches.len();
    // Don't log the raw search query — can be an email, handle, etc. Log
    // only its length and the kind filter.
    Ok(RpcOutcome::single_log(
        SearchEntitiesResponse { matches },
        format!("memory_tree: search_entities query_len={query_len} has_kinds={has_kinds} n={n}"),
    ))
}

// ── drill_down ────────────────────────────────────────────────────────

/// Request body for `memory_tree_drill_down`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillDownRequest {
    pub node_id: String,
    #[serde(default)]
    pub max_depth: Option<u32>,
    /// When set, visited children are reranked by cosine similarity between
    /// the query embedding and each child's stored embedding. Legacy children
    /// without an embedding sort to the bottom.
    #[serde(default)]
    pub query: Option<String>,
    /// Optional cap on the returned hit count, applied AFTER rerank so the
    /// top-K is relevance-based when `query` is provided.
    #[serde(default)]
    pub limit: Option<usize>,
}

/// Response envelope for `memory_tree_drill_down`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct DrillDownResponse {
    pub hits: Vec<RetrievalHit>,
}

/// JSON-RPC handler body for `memory_tree_drill_down`.
///
/// Reads through the contract's `retrieve_children`, which is this tool's
/// member despite the name: the tree family's `drill_down` returns a node and
/// its direct children, where this returns ranked hits several levels deep.
pub async fn drill_down_rpc(
    config: &Config,
    req: DrillDownRequest,
) -> Result<RpcOutcome<DrillDownResponse>, String> {
    let depth = req.max_depth.unwrap_or(1);
    // The explicit scope, never `None` — see the module docs.
    let scope = as_bus_scope();

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let hits = match binding.provider().as_retrieval() {
        Some(retrieval) => retrieval
            .retrieve_children(
                &req.node_id,
                depth,
                req.query.as_deref(),
                req.limit,
                scope.as_ref(),
            )
            .await
            .map_err(|e| format!("drill_down: {e}"))?,
        // Read-only, and an empty vector is already this handler's answer for a
        // node with no children — a driver with no retrieval family has no tree
        // to walk, so the degrade is indistinguishable from the ordinary miss.
        None => {
            log::debug!(
                "[memory-tree][rpc] drill_down: driver '{}' does not serve Retrieval; reporting empty",
                binding.driver_id()
            );
            Vec::new()
        }
    };
    let n = hits.len();
    // node_id can embed source scope (e.g. "chat:slack:#eng:0") which may
    // carry workspace hints — log only the structural prefix.
    let node_kind_prefix = req
        .node_id
        .split_once(':')
        .map(|(k, _)| k)
        .unwrap_or("unknown");
    Ok(RpcOutcome::single_log(
        DrillDownResponse { hits },
        format!(
            "memory_tree: drill_down node_kind={} depth={} has_query={} limit={:?} n={}",
            node_kind_prefix,
            depth,
            req.query.is_some(),
            req.limit,
            n
        ),
    ))
}

// ── fetch_leaves ──────────────────────────────────────────────────────

/// Request body for `memory_tree_fetch_leaves`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchLeavesRequest {
    pub chunk_ids: Vec<String>,
}

/// Response envelope for `memory_tree_fetch_leaves`.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FetchLeavesResponse {
    pub hits: Vec<RetrievalHit>,
}

/// JSON-RPC handler body for `memory_tree_fetch_leaves`.
///
/// Ids that do not resolve are omitted by the driver, so the response may be
/// shorter than the request and callers must not index by position. A chunk
/// whose source falls outside the scope is omitted the same way, which is what
/// stops naming a chunk id directly from reading around the source gate.
pub async fn fetch_leaves_rpc(
    config: &Config,
    req: FetchLeavesRequest,
) -> Result<RpcOutcome<FetchLeavesResponse>, String> {
    // The explicit scope, never `None` — see the module docs. It matters most
    // here: this member takes ids the caller chose.
    let scope = as_bus_scope();

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let hits = match binding.provider().as_retrieval() {
        Some(retrieval) => retrieval
            .retrieve_leaves(&req.chunk_ids, scope.as_ref())
            .await
            .map_err(|e| format!("fetch_leaves: {e}"))?,
        // Read-only, and omitting unresolvable ids is already this member's
        // contract — a driver with no retrieval family resolves none of them.
        None => {
            log::debug!(
                "[memory-tree][rpc] fetch_leaves: driver '{}' does not serve Retrieval; reporting empty",
                binding.driver_id()
            );
            Vec::new()
        }
    };
    let n = hits.len();
    Ok(RpcOutcome::single_log(
        FetchLeavesResponse { hits },
        format!("memory_tree: fetch_leaves n={n}"),
    ))
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
