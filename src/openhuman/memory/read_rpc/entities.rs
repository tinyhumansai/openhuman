use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::binding::MemoryBinding;
use crate::rpc::RpcOutcome;
use tinymemory_api::error::MemoryError;
use tinymemory_api::provider::chunks::ChunkScore;
use tinymemory_api::provider::types::{EntityOccurrence, ForgetSelector};

use super::types::{DeleteChunkResponse, EntityRef, ScoreBreakdown, ScoreSignal, MAX_LIST_LIMIT};

// ── entity index lookups ────────────────────────────────────────────────
//
// All three read the driver's entity index through `MemoryEntities`, and each
// uses the member written for the *browser's* reading of that index rather than
// the agent's: `top_entities` for what the store holds at all, `chunk_entities`
// for what one chunk is about, `entity_chunk_ids` for the content behind one
// entity. `MemoryEntities::entities` is deliberately not the member for any of
// them — it is namespace-scoped where these are store-wide, hotness-ranked
// where these rank by observation count, and its `EntityHit` carries a
// canonical name where these carry the index's `surface` sample.
//
// None of the three needs `spawn_blocking`: the driver owns whether its own
// reads block, and the module's do not run on this thread at all.

/// One occurrence row in the shape this RPC surface has always returned.
///
/// Field for field, including the one rename: the index's `mentions` is the
/// `COUNT(*)` this handler's `count` has always carried.
fn entity_ref(occurrence: EntityOccurrence) -> EntityRef {
    EntityRef {
        entity_id: occurrence.entity_id,
        kind: occurrence.kind,
        surface: occurrence.surface,
        count: occurrence.mentions,
    }
}

/// Every entity indexed against one chunk, most-observed first.
///
/// Shared by [`entity_index_for_rpc`], which reports the rows, and
/// [`delete_chunk_rpc`], which counts the ones a delete is about to take with
/// it. `op` is the caller's name, so an error still reads as that handler's.
///
/// The batch is one id long on purpose. `chunk_entities` names the chunk on
/// every row precisely because a wider batch answers as one flat list and
/// grouping becomes the caller's job — with a single id every row is this
/// chunk's by construction, so there is nothing to group and nothing that could
/// be mis-indexed by position.
///
/// A driver with no entity tier reports no rows rather than failing: both
/// callers are describing what the index holds, and it holds nothing.
async fn chunk_entity_rows(
    binding: &MemoryBinding,
    chunk_id: &str,
    op: &str,
) -> Result<Vec<EntityOccurrence>, String> {
    let Some(entities) = binding.provider().as_entities() else {
        log::debug!(
            "[memory_tree::read::entity_index] {op}: driver '{}' does not serve Entities; \
             reporting empty",
            binding.driver_id()
        );
        return Ok(Vec::new());
    };
    let ids = [chunk_id.to_string()];
    let rows = entities
        .chunk_entities(&ids, None)
        .await
        .map_err(|e| format!("{op}: {e}"))?;
    log::debug!(
        "[memory_tree::read::entity_index] {op}: driver '{}' rows={} chunk_id={chunk_id}",
        binding.driver_id(),
        rows.len()
    );
    Ok(rows.into_iter().map(|row| row.occurrence).collect())
}

pub async fn entity_index_for_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<Vec<EntityRef>>, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let refs: Vec<EntityRef> = chunk_entity_rows(&binding, &chunk_id, "entity_index_for")
        .await?
        .into_iter()
        .map(entity_ref)
        .collect();

    let n = refs.len();
    Ok(RpcOutcome::single_log(
        refs,
        format!("memory_tree::read: entity_index_for chunk_id={chunk_id} n={n}"),
    ))
}

pub async fn chunks_for_entity_rpc(
    config: &Config,
    entity_id: String,
) -> Result<RpcOutcome<Vec<String>>, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let chunk_ids = match binding.provider().as_entities() {
        // A bound appears where the SQL had none, because `entity_chunk_ids`
        // requires one and the contract gives a driver no way to say
        // "unbounded". That is the right shape for a member reached over a bus:
        // the previous query would happily stream every chunk an entity was
        // ever seen in, as one RPC response, over a wire the caller cannot
        // interrupt.
        //
        // `MAX_LIST_LIMIT` is the cap this module already applies to every
        // other list it serves, so the ceiling is the surface's own rather than
        // a number invented here. It is a ceiling and not a page: this handler
        // has never taken a `limit` and its response is a bare `Vec<String>`
        // with nowhere to report truncation, so widening the wire to say
        // "there were more" is a change this migration has no mandate to make.
        // An entity observed in more than a thousand chunks is a graph query,
        // not a list.
        Some(entities) => entities
            .entity_chunk_ids(&entity_id, MAX_LIST_LIMIT as usize)
            .await
            .map_err(|e| format!("chunks_for_entity: {e}"))?,
        // Read-only, so an empty list is the honest answer: a driver with no
        // entity tier indexed this entity nowhere, which is a true statement
        // about it rather than a fault the caller can act on.
        None => {
            log::debug!(
                "[memory_tree::read::chunks_for_entity] driver '{}' does not serve Entities; \
                 reporting empty",
                binding.driver_id()
            );
            Vec::new()
        }
    };

    let n = chunk_ids.len();
    log::debug!(
        "[memory_tree::read::chunks_for_entity] driver '{}' entity_id={entity_id} n={n} limit={}",
        binding.driver_id(),
        MAX_LIST_LIMIT
    );
    Ok(RpcOutcome::single_log(
        chunk_ids,
        format!("memory_tree::read: chunks_for_entity entity_id={entity_id} n={n}"),
    ))
}

pub async fn top_entities_rpc(
    config: &Config,
    kind: Option<String>,
    limit: u32,
) -> Result<RpcOutcome<Vec<EntityRef>>, String> {
    let limit = limit.clamp(1, MAX_LIST_LIMIT);
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let refs: Vec<EntityRef> = match binding.provider().as_entities() {
        Some(entities) => {
            let answered = entities.top_entities(kind.as_deref(), limit as usize).await;
            match answered {
                Ok(rows) => rows.into_iter().map(entity_ref).collect(),
                // The one behaviour delta this migration was warned about, and
                // it is decided in favour of the wire this handler already has.
                // The member validates `kind` and answers `Invalid` for one it
                // does not recognise; the SQL it replaces compared the string
                // against the stored column, so an unknown kind matched no rows
                // and the handler returned an empty list. A migration that
                // turns a quiet empty result into a user-visible error is
                // changing the product, not moving a call, so the variant is
                // mapped back — narrowly, and never silently.
                //
                // Narrowly means two things. Only `Invalid` degrades, so a
                // backend failure still propagates. And only when a `kind` was
                // actually supplied: `Invalid` with no filter to be invalid
                // about is a driver fault, and swallowing that is exactly what
                // this map-back is careful not to do.
                Err(MemoryError::Invalid(reason)) if kind.is_some() => {
                    log::debug!(
                        "[memory_tree::read::top_entities] driver '{}' rejected kind={:?} \
                         ({reason}); reporting empty, which is what the SQL this replaced returned",
                        binding.driver_id(),
                        kind
                    );
                    Vec::new()
                }
                Err(e) => return Err(format!("top_entities: {e}")),
            }
        }
        // Read-only, so an empty ranking is the honest answer: a driver with no
        // entity tier has nothing indexed to rank.
        None => {
            log::debug!(
                "[memory_tree::read::top_entities] driver '{}' does not serve Entities; \
                 reporting empty",
                binding.driver_id()
            );
            Vec::new()
        }
    };

    let n = refs.len();
    log::debug!(
        "[memory_tree::read::top_entities] driver '{}' kind={:?} limit={limit} n={n}",
        binding.driver_id(),
        kind
    );
    Ok(RpcOutcome::single_log(
        refs,
        format!("memory_tree::read: top_entities n={n}"),
    ))
}

// ── chunk_score ─────────────────────────────────────────────────────────
//
// The admission verdict is the driver's and only the driver's: it is a row the
// scorer wrote at ingest, under the policy in force *then*, and re-deriving it
// today would answer under today's policy. `MemoryChunks::chunk_score` is that
// door, and `DEFAULT_DROP_THRESHOLD` crosses on the contract beside it rather
// than being copied here — a host-side `0.3` is the kind of copy that goes
// wrong silently, leaving a stale line drawn under freshly-labelled rows.
//
// No `spawn_blocking` on either of the two handlers below: the driver owns
// whether its own reads block, and the module's do not run on this thread at
// all.

/// One chunk's score row, or `None` when the scorer never wrote one.
///
/// Shared by [`chunk_score_rpc`], which renders it, and [`score_row_count`],
/// which only counts it — the same split [`chunk_entity_rows`] makes for the
/// entity index, and for the same reason: one read, two readings. `op` is the
/// caller's name, so an error still reads as that handler's.
///
/// A driver with no chunk tier reports no row rather than failing: both callers
/// are describing what the store recorded about one chunk, and it recorded
/// nothing. A driver that *has* the tier but keeps no admission record answers
/// `Unsupported`, which propagates — "this driver does not score" is not the
/// same claim as "this chunk was not scored", and only the second is what
/// `None` means here.
async fn chunk_score_row(
    binding: &MemoryBinding,
    chunk_id: &str,
    op: &str,
) -> Result<Option<ChunkScore>, String> {
    let Some(chunks) = binding.provider().as_chunks() else {
        log::debug!(
            "[memory_tree::read::chunk_score] {op}: driver '{}' does not serve Chunks; \
             reporting no score",
            binding.driver_id()
        );
        return Ok(None);
    };
    chunks
        .chunk_score(chunk_id)
        .await
        .map_err(|e| format!("{op}: {e}"))
}

/// The seven signals the score panel renders, in the order it renders them.
///
/// The weights are **this handler's**, not the driver's: no contract member
/// reports them, and they are what the panel has always shown beside each
/// signal, so they stay written out here exactly as they were. `weight` on
/// `llm_importance` is the one that varies, and it varies on the same predicate
/// the response's `llm_consulted` carries.
fn score_breakdown(row: ChunkScore) -> ScoreBreakdown {
    let llm_consulted = row.signals.llm_importance > 0.0;
    let signals = vec![
        ScoreSignal {
            name: "token_count".into(),
            weight: 1.0,
            value: row.signals.token_count,
        },
        ScoreSignal {
            name: "unique_words".into(),
            weight: 1.0,
            value: row.signals.unique_words,
        },
        ScoreSignal {
            name: "metadata_weight".into(),
            weight: 1.5,
            value: row.signals.metadata_weight,
        },
        ScoreSignal {
            name: "source_weight".into(),
            weight: 1.5,
            value: row.signals.source_weight,
        },
        ScoreSignal {
            name: "interaction".into(),
            weight: 3.0,
            value: row.signals.interaction,
        },
        ScoreSignal {
            name: "entity_density".into(),
            weight: 1.0,
            value: row.signals.entity_density,
        },
        ScoreSignal {
            name: "llm_importance".into(),
            weight: if llm_consulted { 2.0 } else { 0.0 },
            value: row.signals.llm_importance,
        },
    ];
    ScoreBreakdown {
        signals,
        total: row.total,
        threshold: tinymemory_bus::provider::chunks::DEFAULT_DROP_THRESHOLD,
        kept: !row.dropped,
        llm_consulted,
    }
}

pub async fn chunk_score_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<Option<ScoreBreakdown>>, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let result = chunk_score_row(&binding, &chunk_id, "chunk_score")
        .await?
        .map(score_breakdown);
    Ok(RpcOutcome::single_log(
        result,
        format!("memory_tree::read: chunk_score id={chunk_id}"),
    ))
}

// ── delete_chunk ────────────────────────────────────────────────────────

/// Whether the driver holds a score row for this chunk, as a rowcount.
///
/// The admission record is keyed by chunk id, so "is there a row" *is* the
/// number the `DELETE` this replaced returned. Read through the same
/// [`chunk_score_row`] door [`chunk_score_rpc`] uses, so the panel and the
/// delete cannot disagree about whether one chunk was ever scored.
async fn score_row_count(binding: &MemoryBinding, chunk_id: &str) -> Result<u32, String> {
    let row = chunk_score_row(binding, chunk_id, "delete_chunk").await?;
    Ok(u32::from(row.is_some()))
}

pub async fn delete_chunk_rpc(
    config: &Config,
    chunk_id: String,
) -> Result<RpcOutcome<DeleteChunkResponse>, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;

    // Resolved on `provider()` and **refused** when the family is absent, not
    // degraded. The read handlers above answer empty for a missing family
    // because "this driver indexes nothing" is a true answer to what they were
    // asked; this is a delete, and its empty answer — `deleted: false` — is
    // byte-identical to "that chunk was already gone". The contract makes the
    // same call in `forget_matching`'s own errors: on a delete, a zero the
    // caller reads as "already gone" is worse than a refusal.
    let Some(sources) = binding.provider().as_sources() else {
        return Err(format!(
            "delete_chunk: driver '{}' does not serve Sources",
            binding.driver_id()
        ));
    };

    // Both side-row counts are observed BEFORE the delete, because
    // `ForgetOutcome` does not carry them: it reports `chunks_removed` and
    // `trees_cleaned`, and its own docs say the per-chunk side rows go
    // *together with* the chunk rather than being counted apart from it.
    // `DeleteChunkResponse` has reported these two numbers since it existed and
    // the frontend logs both, so they are read here rather than dropped to
    // zero — a zero after a successful delete reads as "there was nothing to
    // clean up", which is a different claim from "nobody counted".
    //
    // The cost, stated rather than hidden: these are a pre-delete observation,
    // not the transactional rowcounts the hand-written SQL returned. A writer
    // landing between the read and the delete would move them. Both are
    // diagnostics about one chunk a user is looking at, so a race no user can
    // provoke is the cheaper price than a number that stops being reported.
    let entity_index_rows_removed = chunk_entity_rows(&binding, &chunk_id, "delete_chunk")
        .await?
        .iter()
        .map(|occurrence| occurrence.mentions)
        .fold(0u32, u32::saturating_add);
    let score_rows_removed = score_row_count(&binding, &chunk_id).await?;

    let selector = ForgetSelector::Chunk {
        chunk_id: chunk_id.clone(),
    };
    let outcome = sources
        .forget_matching(&selector)
        .await
        .map_err(|e| format!("delete_chunk: {e}"))?;

    // No host-side file removal any more, and no `spawn_blocking` to host it.
    // The driver's by-id delete collects `content_path` inside the transaction
    // and unlinks after the commit, together with the chunk's embedding row,
    // its reembed-skipped row, and any raw-ref files nothing else points at —
    // the last three of which the hand-written SQL this replaces left behind.
    let resp = DeleteChunkResponse {
        deleted: outcome.chunks_removed > 0,
        score_rows_removed,
        entity_index_rows_removed,
    };
    log::debug!(
        "[memory_tree::read::delete] driver '{}' id={chunk_id} chunks_removed={} \
         trees_cleaned={} score_rows={score_rows_removed} \
         entity_rows={entity_index_rows_removed}",
        binding.driver_id(),
        outcome.chunks_removed,
        outcome.trees_cleaned
    );
    Ok(RpcOutcome::single_log(
        resp.clone(),
        format!(
            "memory_tree::read: delete_chunk id={chunk_id} deleted={} score_rows={} entity_rows={}",
            resp.deleted, resp.score_rows_removed, resp.entity_index_rows_removed
        ),
    ))
}
