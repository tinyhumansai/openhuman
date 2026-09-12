use anyhow::Result;
use std::collections::{HashMap, HashSet};

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::retrieval::{
    RetrievalNodeKind, RetrievalResponse, SourceRetrievalQuery,
};
use crate::openhuman::memory::api::provider::{ChunkListRow, ChunkQuery};
use crate::rpc::RpcOutcome;
use tinymemory_api::chunks::SourceKind;

use super::types::{
    ChunkFilter, ChunkRow, ListChunksResponse, RecallResponse, Source, DEFAULT_LIST_LIMIT,
    MAX_LIST_LIMIT, PREVIEW_MAX_CHARS,
};

// ── list_chunks ──────────────────────────────────────────────────────────

pub async fn list_chunks_rpc(
    config: &Config,
    filter: ChunkFilter,
) -> Result<RpcOutcome<ListChunksResponse>, String> {
    let resp = list_chunks_page(config, &filter).await?;

    let n = resp.chunks.len();
    let total = resp.total;
    Ok(RpcOutcome::single_log(
        resp,
        format!("memory_tree::read: list_chunks n={n} total={total}"),
    ))
}

/// One page of chunks plus the unpaged total that labels it, both read through
/// the bound driver's `Chunks` family.
///
/// This was raw SQL, and its doc comment used to enumerate three filters the
/// contract could not express: the `mem_tree_entity_index` join behind
/// `entity_ids`, the *sets* behind `source_kinds` / `source_ids` (the old
/// `ChunkQuery` narrowed to one of each), and the `content LIKE` scan behind
/// `query`. All three are `ChunkQuery` fields now, and `count_chunks` answers
/// the total that comment said no member could.
///
/// **The page and the total are built from the same [`ChunkQuery`] value, and
/// that is the point.** `count_chunks` differs from `list_chunk_details` only
/// in dropping the page bounds — the `WHERE` clause is one builder, driver-side
/// — so a total can no longer disagree with the page it labels. The SQL this
/// replaces derived its count statement by string-replacing the select list out
/// of the listing statement, which is the same guarantee held together by
/// whichever edit touched the string last.
async fn list_chunks_page(
    config: &Config,
    filter: &ChunkFilter,
) -> Result<ListChunksResponse, String> {
    let limit = filter
        .limit
        .unwrap_or(DEFAULT_LIST_LIMIT)
        .clamp(1, MAX_LIST_LIMIT);
    let offset = filter.offset.unwrap_or(0);

    let Some(query) = chunk_query_from_filter(filter, limit, offset) else {
        // Not an empty query — a query that cannot match. See
        // [`chunk_query_from_filter`]: sending it would ask for the whole
        // store.
        log::debug!("[memory_tree::read] list_chunks: filter matches nothing; query skipped");
        return Ok(ListChunksResponse {
            chunks: Vec::new(),
            total: 0,
        });
    };

    // No `spawn_blocking`: the driver owns whether its own reads block, and the
    // module's do not run on this thread at all.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(chunks) = binding.provider().as_chunks() else {
        // Read-only, so an empty page is the honest answer: a driver with no
        // chunk tier holds no rows to list, and no rows to count either.
        log::debug!(
            "[memory_tree::read] list_chunks: driver '{}' does not serve Chunks; reporting empty",
            binding.driver_id()
        );
        return Ok(ListChunksResponse {
            chunks: Vec::new(),
            total: 0,
        });
    };

    // `scope` is `None` here because this listing has never been source-scoped:
    // the SQL it replaces had no allowlist clause at all. Passing the ambient
    // scope would be a new restriction on the Memory tab, not a translation of
    // one.
    let rows = chunks
        .list_chunk_details(&query, None)
        .await
        .map_err(|e| format!("list_chunk_details: {e}"))?;
    let total = chunks
        .count_chunks(&query, None)
        .await
        .map_err(|e| format!("count_chunks: {e}"))?;

    log::debug!(
        "[memory_tree::read] list_chunks: limit={limit} offset={offset} rows={} total={total}",
        rows.len()
    );

    Ok(ListChunksResponse {
        chunks: rows.into_iter().map(chunk_row_from_list_row).collect(),
        total,
    })
}

/// Translate the RPC's [`ChunkFilter`] into the contract's [`ChunkQuery`].
///
/// `None` means **this filter can match nothing, so skip the query entirely**.
/// That is not the same as an unconstrained query, and the difference is the
/// one footgun `ChunkQuery` carries: every `Vec` predicate defaults to empty
/// and an empty predicate is *unfiltered*, so handing over the empty result of
/// a narrowing computation asks for the whole store rather than for nothing.
///
/// Exactly one field here narrows: `source_kinds` arrives as free text and the
/// contract carries a decoded enum, so kinds this build does not know drop out.
/// Dropping them reproduces the old SQL, where an unrecognised value sat in
/// `source_kind IN (…)` and simply matched no row — `["chat", "bogus"]` still
/// returns the chat rows. When *every* value is unrecognised the old clause
/// matched nothing, and `None` is how that answer is spelled here; leaving the
/// vec empty instead would return the whole store.
///
/// An empty `Some(vec![])` is *not* that case: the old SQL skipped the clause
/// for an empty list, so it is unfiltered here too, which is what the empty vec
/// already means.
fn chunk_query_from_filter(filter: &ChunkFilter, limit: u32, offset: u32) -> Option<ChunkQuery> {
    let mut source_kinds: Vec<SourceKind> = Vec::new();
    if let Some(requested) = filter.source_kinds.as_ref() {
        for raw in requested {
            match SourceKind::parse(raw) {
                Ok(kind) => source_kinds.push(kind),
                Err(e) => log::debug!(
                    "[memory_tree::read] list_chunks: dropping unknown source_kind {raw:?} ({e})"
                ),
            }
        }
        if source_kinds.is_empty() && !requested.is_empty() {
            return None;
        }
    }

    let content_contains = filter.query.as_deref().and_then(content_predicate);

    Some(ChunkQuery {
        source_ids: filter.source_ids.clone().unwrap_or_default(),
        entity_ids: filter.entity_ids.clone().unwrap_or_default(),
        source_kinds,
        since_ms: filter.since_ms,
        until_ms: filter.until_ms,
        content_contains,
        limit: Some(limit as usize),
        offset: Some(offset as usize),
        // The predicates this filter does not carry. `exclude_dropped` stays
        // false because the SQL had no lifecycle clause: the Memory tab lists
        // dropped rows and renders their status.
        ..Default::default()
    })
}

/// The `content_contains` predicate for a caller-supplied query string, or
/// `None` when the caller typed only whitespace.
///
/// One function because two call sites need the same rule and a drifted pair
/// would show as a search that filters differently from the listing behind it.
///
/// Two things it deliberately does not do. It does not wrap the value in
/// `%…%` — that is the driver's, and so is escaping any `%` or `_` the user
/// typed, which the `format!("%{}%", q)` this replaces did not do at all: a
/// query containing `_` silently matched any character in that position. And a
/// blank query places no predicate rather than matching nothing, which is what
/// the old `if !q.is_empty()` guard did and is why a cleared search box lists
/// the newest rows instead of emptying the table.
fn content_predicate(query: &str) -> Option<String> {
    let trimmed = query.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Render one contract listing row as the wire [`ChunkRow`] the Memory tab
/// reads.
///
/// [`ChunkListRow`] carries no body, deliberately — `ChunkDetail::body`
/// promises `None` means the vault read *failed*, which a page of rows can only
/// honour by reading every file or by lying. Nothing is lost here: the preview
/// has always come from the stored `content` column, which is a ≤500-char
/// preview for a chunk whose body went to the content vault and the whole body
/// for an inline one, and that column is `Chunk::content`. The truncation below
/// is therefore a no-op on already-previewed rows and identical to the SQL's on
/// inline ones.
fn chunk_row_from_list_row(row: ChunkListRow) -> ChunkRow {
    let ChunkListRow {
        chunk,
        content_path,
        lifecycle_status,
        has_embedding,
    } = row;
    let preview: String = chunk.content.chars().take(PREVIEW_MAX_CHARS).collect();
    ChunkRow {
        id: chunk.id,
        source_kind: chunk.metadata.source_kind.as_str().to_string(),
        source_id: chunk.metadata.source_id,
        source_ref: chunk.metadata.source_ref.map(|r| r.value),
        owner: chunk.metadata.owner,
        timestamp_ms: chunk.metadata.timestamp.timestamp_millis(),
        token_count: chunk.token_count,
        // Matches `read_chunk_row` below. The column is `NOT NULL DEFAULT
        // 'admitted'`, so `None` is unreachable through the engine driver; a
        // driver with no lifecycle concept at all is what the `Option` is for.
        lifecycle_status: lifecycle_status.unwrap_or_else(|| "unknown".to_string()),
        content_path,
        content_preview: if preview.is_empty() {
            None
        } else {
            Some(preview)
        },
        has_embedding,
        tags: chunk.metadata.tags,
    }
}

// ── list_sources ─────────────────────────────────────────────────────────

pub async fn list_sources_rpc(
    config: &Config,
    user_email_hint: Option<String>,
) -> Result<RpcOutcome<Vec<Source>>, String> {
    let sources = list_sources(config, user_email_hint.as_deref()).await?;

    let n = sources.len();
    Ok(RpcOutcome::single_log(
        sources,
        format!("memory_tree::read: list_sources n={n}"),
    ))
}

/// The `(source_kind, source_id)` rollup, read through the driver's
/// `source_totals` rather than a host-side `GROUP BY`.
///
/// `display_name` stays here because it is presentation, not storage: turning
/// `gmail:alice@example.com|bob@example.com` into `bob@example.com` needs to
/// know which address is the user's, which is host state. The contract's
/// `SourceTotal` deliberately carries no such field.
async fn list_sources(
    config: &Config,
    user_email_hint: Option<&str>,
) -> Result<Vec<Source>, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(chunks) = binding.provider().as_chunks() else {
        log::debug!(
            "[memory_tree::read] list_sources: driver '{}' does not serve Chunks; reporting empty",
            binding.driver_id()
        );
        return Ok(Vec::new());
    };

    // No host-imposed cap. The `GROUP BY` this replaces had no `LIMIT`, so the
    // only ceiling that ever applied was the store's own, and `usize::MAX` asks
    // for exactly that one instead of inventing a smaller number here.
    // `MAX_LIST_LIMIT` bounds a *page of chunks* and is the wrong unit: for a
    // mailbox every thread is its own source, so a thousand groups is a number
    // real workspaces pass. `scope` is `None` for the same reason the listing's
    // is — this rollup has never been source-scoped.
    let totals = chunks
        .source_totals(usize::MAX, None)
        .await
        .map_err(|e| format!("source_totals: {e}"))?;

    log::debug!(
        "[memory_tree::read] list_sources: groups={} hint={}",
        totals.len(),
        user_email_hint.is_some()
    );

    Ok(totals
        .into_iter()
        .map(|total| {
            let display_name = display_name_for_source(&total.source_id, user_email_hint);
            Source {
                source_id: total.source_id,
                display_name,
                source_kind: total.source_kind.as_str().to_string(),
                // Saturating rather than wrapping. The wire field is `u32` and
                // the count is `u64`; a store that ever reached the ceiling
                // should render as "very many", not as a small number.
                chunk_count: u32::try_from(total.chunk_count).unwrap_or(u32::MAX),
                most_recent_ms: total.most_recent_ms,
            }
        })
        .collect())
}

/// Compute the display name for a source.
///
/// Examples:
/// - `slack:#engineering` → `#engineering`
/// - `gmail:alice@example.com|bob@example.com` (user is alice) → `bob@example.com`
/// - `gmail:alice@example.com|bob@example.com` (user unknown) →
///   `alice@example.com ↔ bob@example.com`
pub fn display_name_for_source(source_id: &str, user_email_hint: Option<&str>) -> String {
    let body = match source_id.split_once(':') {
        Some((_platform, rest)) => rest,
        None => source_id,
    };
    if body.contains('|') {
        let parts: Vec<&str> = body.split('|').collect();
        if let Some(user) = user_email_hint {
            let user_lc = user.trim().to_ascii_lowercase();
            let others: Vec<&str> = parts
                .iter()
                .copied()
                .filter(|p| p.trim().to_ascii_lowercase() != user_lc)
                .collect();
            if !others.is_empty() && others.len() < parts.len() {
                return others.join(", ");
            }
        }
        return parts.join(" ↔ ");
    }
    body.to_string()
}

// ── search / recall ──────────────────────────────────────────────────────

/// Substring browse over the chunk tier.
///
/// Straight to `list_chunk_details` with `content_contains` and a limit, and
/// nothing else — the old path built a full [`ChunkFilter`], ran the listing,
/// and threw the total away, which is a `COUNT(*)` over the whole match set per
/// keystroke.
///
/// **What this actually scans is the text the driver holds inline**, which for
/// a chunk whose body went to the content vault is the stored ≤500-char
/// preview and not the whole document. That is not a narrowing: the SQL this
/// replaces scanned `mem_tree_chunks.content`, the same column. It is worth
/// stating because the RPC is named `search` — this narrows a browse, and
/// "search my memory" is `MemoryRecall`, which [`recall_rpc`] below uses.
pub async fn search_rpc(
    config: &Config,
    query: String,
    k: u32,
) -> Result<RpcOutcome<Vec<ChunkRow>>, String> {
    let limit = k.clamp(1, MAX_LIST_LIMIT);
    let content_contains = content_predicate(&query);
    // Captured before `query` is shadowed by the `ChunkQuery` below. The log
    // line reports the length of the search TEXT, never of the built query, and
    // it is deliberately the length rather than the text itself — a search term
    // is user-authored content and does not belong in a log.
    let query_len = query.len();

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(chunks) = binding.provider().as_chunks() else {
        log::debug!(
            "[memory_tree::read] search: driver '{}' does not serve Chunks; reporting empty",
            binding.driver_id()
        );
        return Ok(RpcOutcome::single_log(
            Vec::new(),
            format!("memory_tree::read: search query_len={query_len} n=0"),
        ));
    };

    // `content_contains` and a limit, and nothing else: no source predicate, no
    // time window, no offset. `scope` is `None` for the same reason the listing
    // passes `None` — the SQL this replaces carried no allowlist clause.
    let query = ChunkQuery {
        content_contains,
        limit: Some(limit as usize),
        ..Default::default()
    };
    let rows = chunks
        .list_chunk_details(&query, None)
        .await
        .map_err(|e| format!("list_chunk_details: {e}"))?;

    let hits: Vec<ChunkRow> = rows.into_iter().map(chunk_row_from_list_row).collect();
    let n = hits.len();
    log::debug!("[memory_tree::read] search: limit={limit} n={n}");
    Ok(RpcOutcome::single_log(
        hits,
        format!("memory_tree::read: search query_len={query_len} n={n}"),
    ))
}

pub async fn recall_rpc(
    config: &Config,
    query: String,
    k: u32,
) -> Result<RpcOutcome<RecallResponse>, String> {
    let limit = k.clamp(1, MAX_LIST_LIMIT) as usize;
    log::debug!(
        "[memory_tree::read::recall] query_len={} k={}",
        query.len(),
        limit
    );

    // Through the bound driver's `MemoryRetrieval` rather than the engine
    // (#5560), and with an explicit scope.
    //
    // `binding.provider()` is the UNGUARDED driver: nothing between here and
    // the store re-applies the active profile's `memory_sources` allowlist, so
    // the scope this call passes is the gate. The engine's `*_scoped` entry
    // points existed for the same reason — their ambient twins read the
    // ENGINE's task-local, which a separately compiled module cannot see, and
    // an absent scope means unrestricted, i.e. the gate failing open.
    //
    // `as_bus_scope()` renders this host's own task-local in the contract's
    // vocabulary. `None` from it means genuinely unrestricted and must stay
    // `None` — an *empty* `SourceScope` denies every source-attributed row, so
    // mapping "no restriction" onto one would invert the policy. Mirrors
    // `query_source_rpc` in `tree/retrieval/rpc.rs`.
    let scope = crate::openhuman::memory::source_scope::as_bus_scope();
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let resp = match binding.provider().as_retrieval() {
        Some(retrieval) => retrieval
            .retrieve_source(
                &SourceRetrievalQuery {
                    source_id: None,
                    source_kind: None,
                    time_window_days: None,
                    query: Some(query.clone()),
                    limit,
                },
                scope.as_ref(),
            )
            .await
            .map_err(|e| format!("recall query_source: {e}"))?,
        // Read-only, so an empty page is the honest answer: a driver with no
        // retrieval family keeps no summary tree to rank.
        None => {
            log::debug!(
                "[memory_tree::read::recall] driver '{}' does not serve Retrieval; reporting empty",
                binding.driver_id()
            );
            RetrievalResponse::default()
        }
    };

    let mut chunk_rows: Vec<ChunkRow> = Vec::new();
    let mut scores: Vec<f32> = Vec::new();
    let leaves: Vec<(String, f32)> = resp
        .hits
        .into_iter()
        .filter(|h| matches!(h.node_kind, RetrievalNodeKind::Summary) && h.level == 1)
        .flat_map(|h| {
            h.child_ids
                .into_iter()
                .map(move |id| (id, h.score))
                .collect::<Vec<_>>()
        })
        .collect();
    // Hydrated in ONE call, not one per leaf: `list_chunk_details` takes the ids
    // as a predicate. Three things about that result are load-bearing.
    //
    // The empty case is skipped rather than sent. An empty `ChunkQuery::ids` is
    // *unfiltered*, so asking for no ids would hydrate the whole store.
    //
    // The result may be SHORTER than the ids sent — an id the store does not
    // hold contributes no row, exactly as the per-id `query_row(…).ok()` this
    // replaces skipped a miss — and it arrives in the query's own newest-first
    // order, not in leaf order. So it is re-mapped by id below. Indexing it by
    // position against the input would silently re-order the recall response
    // and mis-pair every row with a neighbour's score.
    //
    // `has_embedding` changes meaning here, and it is worth being loud about.
    // The row-by-row SQL read `mem_tree_chunks.embedding`, the inline column the
    // sidecar migration left behind — nothing has written it since, so the flag
    // was false for every chunk whether or not it had been embedded. The
    // contract's flag is an `EXISTS` over `mem_tree_chunk_embeddings` and is
    // signature-blind, so an embedded chunk now reports true. It is the same
    // flag `read_chunk_row` below already returns through `chunk_detail`, so
    // the list and the single-row read stop disagreeing about one chunk.
    if !leaves.is_empty() {
        // Deduped, because two level-1 summaries can point at one chunk and the
        // predicate only needs to name it once — but order-preserving, so the
        // log below reads against the leaves it came from. Scoped so the borrow
        // of `leaves` ends here; the walk that pairs rows with scores consumes
        // it.
        let ids: Vec<String> = {
            let mut seen: HashSet<&str> = HashSet::with_capacity(leaves.len());
            let mut ids = Vec::with_capacity(leaves.len());
            for (id, _) in &leaves {
                if seen.insert(id.as_str()) {
                    ids.push(id.clone());
                }
            }
            ids
        };
        let id_count = ids.len();
        let query = ChunkQuery {
            // Sized to the ids asked for. Left unset, the driver applies its
            // own default page, which would drop leaves past it — and the page
            // order is not leaf order, so which ones it dropped would look
            // arbitrary.
            limit: Some(id_count),
            ids,
            ..Default::default()
        };

        let binding = crate::openhuman::memory::binding::for_config(config)?;
        let rows = match binding.provider().as_chunks() {
            Some(chunks) => chunks
                .list_chunk_details(&query, None)
                .await
                .map_err(|e| format!("list_chunk_details: {e}"))?,
            None => {
                log::debug!(
                    "[memory_tree::read::recall] driver '{}' does not serve Chunks; no hydration",
                    binding.driver_id()
                );
                Vec::new()
            }
        };

        let by_id: HashMap<String, ChunkRow> = rows
            .into_iter()
            .map(chunk_row_from_list_row)
            .map(|row| (row.id.clone(), row))
            .collect();
        log::debug!(
            "[memory_tree::read::recall] hydrate: leaves={} ids={id_count} rows={}",
            leaves.len(),
            by_id.len()
        );
        let (paired_rows, paired_scores) = pair_leaves_with_rows(&leaves, &by_id);
        chunk_rows = paired_rows;
        scores = paired_scores;
    }
    chunk_rows.truncate(limit);
    scores.truncate(limit);

    let n = chunk_rows.len();
    Ok(RpcOutcome::single_log(
        RecallResponse {
            chunks: chunk_rows,
            scores,
        },
        format!("memory_tree::read: recall n={n}"),
    ))
}

/// Pair hydrated rows back to the leaves that asked for them, in **leaf order**.
///
/// The driver answers in its own newest-first order and may answer with fewer
/// rows than ids were sent, so the pairing has to go through the id. Walking
/// `leaves` rather than `rows` is what keeps three promises at once: the order
/// recall documents, the `chunks[i]` / `scores[i]` alignment the response shape
/// implies, and one row per leaf for a chunk that two level-1 summaries both
/// point at — which is what the per-id loop this replaces produced.
///
/// Extracted rather than inlined because indexing the driver's answer by
/// position against the input is the failure this exists to prevent, and an
/// invariant worth naming is worth being able to test.
fn pair_leaves_with_rows(
    leaves: &[(String, f32)],
    by_id: &HashMap<String, ChunkRow>,
) -> (Vec<ChunkRow>, Vec<f32>) {
    let mut rows = Vec::with_capacity(leaves.len());
    let mut scores = Vec::with_capacity(leaves.len());
    for (chunk_id, score) in leaves {
        if let Some(row) = by_id.get(chunk_id) {
            rows.push(row.clone());
            scores.push(*score);
        }
    }
    (rows, scores)
}

// ── small helpers ───────────────────────────────────────────────────────

/// One chunk rendered for inspection.
///
/// Reads through the bound driver's [`MemoryChunks::chunk_detail`], which
/// returns the row, its vault body, content path, lifecycle state and embedding
/// presence in **one** call. It used to make four separate engine calls in
/// process; four bus round trips per rendered row would have been the direct
/// translation, and this is used to render lists.
///
/// [`MemoryChunks::chunk_detail`]:
///     crate::openhuman::memory::api::provider::MemoryChunks::chunk_detail
pub async fn read_chunk_row(chunk_id: &str) -> Result<Option<ChunkRow>> {
    use crate::openhuman::memory::api::provider::MemoryProvider;

    let guard = crate::openhuman::memory::ops::guard::active_memory_guard()
        .await
        .map_err(|e| anyhow::anyhow!("read_chunk_row: {e}"))?;
    let Some(detail) = guard
        .as_chunks()
        .ok_or_else(|| anyhow::anyhow!("read_chunk_row: driver has no chunk family"))?
        .chunk_detail(chunk_id)
        .await?
    else {
        return Ok(None);
    };

    let chunk = detail.chunk;
    // A failed vault read falls back to the row's own content, as before —
    // `body: None` means "could not read", not "empty".
    let body = detail.body.unwrap_or_else(|| chunk.content.clone());
    let preview: String = body.chars().take(PREVIEW_MAX_CHARS).collect();
    Ok(Some(ChunkRow {
        id: chunk.id,
        source_kind: chunk.metadata.source_kind.as_str().to_string(),
        source_id: chunk.metadata.source_id,
        source_ref: chunk.metadata.source_ref.map(|r| r.value),
        owner: chunk.metadata.owner,
        timestamp_ms: chunk.metadata.timestamp.timestamp_millis(),
        token_count: chunk.token_count,
        lifecycle_status: detail
            .lifecycle_status
            .unwrap_or_else(|| "unknown".to_string()),
        content_path: detail.content_path,
        content_preview: if preview.is_empty() {
            None
        } else {
            Some(preview)
        },
        has_embedding: detail.has_embedding,
        tags: chunk.metadata.tags,
    }))
}

#[cfg(test)]
#[path = "chunks_tests.rs"]
mod tests;
