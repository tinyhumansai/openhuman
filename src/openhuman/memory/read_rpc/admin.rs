use anyhow::{Context, Result};

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::ForgetSelector;
use crate::rpc::RpcOutcome;
// The KV namespace the Composio sync pipelines keep their per-connection
// cursor state under, named at the **contract** (#5560).
//
// It used to be `tinycortex::memory::sync::state::STATE_NAMESPACE`. The
// contract publishes the same string under `composio::KV_NAMESPACE`, and its
// own docs mark it a compatibility surface for exactly the reason this handler
// cares about: the value is on disk, so a wipe that spelled it differently
// would leave every cursor behind while reporting a clean sweep. Taking the
// constant rather than copying the literal is what keeps that impossible —
// and it is the same constant the driver writing those rows reads.
use tinymemory_api::chunks::SourceKind;
use tinymemory_api::composio::KV_NAMESPACE;

use super::types::{
    DeleteSourceResponse, FlushNowResponse, FlushSourceTreeResponse, ResetTreeResponse,
    WipeAllResponse,
};

// ── wipe_all ─────────────────────────────────────────────────────────────

/// Erase the whole memory store, then remove the content directories this host
/// wrote beside it.
///
/// Two halves that read like one operation and deliberately are not.
///
/// The **store** wipe is the driver's: it owns which tables exist and what a
/// row is. It comes back as `PurgeOutcome::rows_deleted`, a sum across every
/// table the driver emptied — which is exactly what this response's
/// `rows_deleted` has always reported (the handler used to sum it here, over
/// nine hand-written `DELETE`s). So the number maps straight across and is
/// deliberately neither re-derived nor re-scaled.
///
/// The **content directories** are this host's: it created them, its config
/// decides where they live, and `purge_all`'s own contract says a driver must
/// not reach into a host-owned path. `dirs_removed` therefore stays here,
/// unchanged, and stays outside the driver call.
pub async fn wipe_all_rpc(config: &Config) -> Result<RpcOutcome<WipeAllResponse>, String> {
    // No `spawn_blocking` around either driver call — the driver owns whether
    // its own reads block, and the module's do not run on this thread at all
    // (the same reasoning `flush_now_rpc` below already carries). The directory
    // removal further down was always async `tokio::fs` and never sat inside
    // the wrapper the table deletes did, so the wrapper leaves with them.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        // Refused, not reported as a wipe that deleted nothing. The caller's
        // next act is telling a user their memory is gone; that is the same
        // reason the contract defaults `purge_all` to `Unsupported` rather than
        // to a zero outcome.
        return Err(format!(
            "wipe_all: driver '{}' does not serve Maintenance",
            binding.driver_id()
        ));
    };
    log::debug!(
        "[memory_tree::read::wipe] purge_all driver={}",
        binding.driver_id()
    );
    let rows_deleted = maintenance
        .purge_all()
        .await
        .map_err(|e| format!("wipe_all: {e}"))?
        .rows_deleted;
    log::debug!("[memory_tree::read::wipe] purge_all rows_deleted={rows_deleted}");

    // Ordered after the purge exactly as it was when both halves ran inside one
    // `spawn_blocking`: sync cursors are only safe to drop once the content they
    // point at has gone.
    let sync_state_cleared = clear_composio_sync_state(config)
        .await
        .map_err(|e| format!("wipe_all: {e}"))?;

    const DIRS: &[&str] = &["raw", "wiki", "chat", "document", "email", "summaries"];
    let content_root = config.memory_tree_content_root();
    let mut dirs_removed: Vec<String> = Vec::new();
    for dir in DIRS {
        let path = content_root.join(dir);
        let remove_result = crate::openhuman::util::retry_with_backoff_async(
            &format!("remove dir {}", dir),
            6,
            200,
            || async {
                tokio::fs::remove_dir_all(&path)
                    .await
                    .context("remove_dir_all")
            },
        )
        .await;

        match remove_result {
            Ok(()) => dirs_removed.push((*dir).to_string()),
            Err(e) => {
                let is_not_found = e
                    .chain()
                    .find_map(|e| e.downcast_ref::<std::io::Error>())
                    .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::NotFound);
                if !is_not_found {
                    log::warn!(
                        "[memory_tree::read::wipe] failed to remove dir={} err={:#}",
                        dir,
                        e
                    );
                }
            }
        }
    }

    let resp = WipeAllResponse {
        rows_deleted,
        dirs_removed,
        sync_state_cleared,
    };

    let log = format!(
        "memory_tree::read: wipe_all rows={} dirs={:?} sync_state={}",
        resp.rows_deleted, resp.dirs_removed, resp.sync_state_cleared
    );
    Ok(RpcOutcome::single_log(resp, log))
}

/// Clear the composio sync-state namespace from the bound driver's key/value
/// tier, reporting how many records went.
///
/// # What this replaced
///
/// A `rusqlite::Connection` opened directly on a path rebuilt from
/// `workspace_dir` — a second, unpoliced door into the store alongside
/// `with_connection`, and one that hard-coded both the file layout and the
/// `kv_namespace` table name. Both are the driver's business; the namespace is
/// read with `MemoryGraph::kv_list` and each record removed with
/// `MemoryGraph::kv_delete`.
///
/// # What replaced the "database file is missing" skip
///
/// The old code checked `path.exists()` first and returned `0` when it did not,
/// because opening a `Connection` on a missing file would have *created* an
/// empty database as a side effect of asking. Over the contract there is no
/// file to ask about — the driver owns its storage and when it comes into
/// being — so the equivalent question is asked of the driver instead, and it
/// has two answers that both land on the same `0`:
///
/// - a driver serving no `Graph` family has no key/value tier to clear, which
///   is reported here rather than refused (nothing was stored through this
///   host's kv path either, so "nothing removed" is true of it); and
/// - a driver whose namespace is empty lists nothing and deletes nothing.
///
/// # Not atomic, unlike the single `DELETE` it replaces
///
/// The contract addresses key/value records one at a time, so this is a list
/// followed by N deletes rather than one statement, and a record written into
/// the namespace between the two survives. That is acceptable here and only
/// here: the sole caller is a whole-store wipe, which has already stopped
/// meaning anything if a sync is still writing into the store it is erasing.
pub(crate) async fn clear_composio_sync_state(config: &Config) -> Result<u64, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(graph) = binding.provider().as_graph() else {
        log::debug!(
            "[memory_tree::read::wipe] clear_composio_sync_state: driver '{}' \
             does not serve Graph; nothing to clear",
            binding.driver_id()
        );
        return Ok(0);
    };

    // `usize::MAX` because the whole namespace is the target — the same
    // unbounded listing `memory::ops::kv_graph::kv_list_namespace` already
    // does. There is no prefix: the namespace *is* the filter, matching the
    // `WHERE namespace = ?1` the single `DELETE` used.
    let records = graph
        .kv_list(Some(KV_NAMESPACE), None, usize::MAX)
        .await
        .map_err(|e| format!("clear_composio_sync_state: kv_list: {e}"))?;
    log::debug!(
        "[memory_tree::read::wipe] clear_composio_sync_state namespace={} listed={}",
        KV_NAMESPACE,
        records.len()
    );

    let mut removed: u64 = 0;
    for record in &records {
        // Counted from the driver's answer rather than from the listing, so the
        // number stays "records actually removed" — exactly what the single
        // `DELETE`'s changed-row count fed into `sync_state_cleared`.
        if graph
            .kv_delete(Some(KV_NAMESPACE), &record.key)
            .await
            .map_err(|e| format!("clear_composio_sync_state: kv_delete: {e}"))?
        {
            removed += 1;
        }
    }
    log::debug!(
        "[memory_tree::read::wipe] clear_composio_sync_state namespace={} removed={}",
        KV_NAMESPACE,
        removed
    );
    Ok(removed)
}

// ── reset_tree ───────────────────────────────────────────────────────────

/// Drop everything derived from stored content and schedule its re-derivation
/// via the bound driver — deletion and rebuild must be one operation, so it
/// belongs to the driver that owns the derived tables.
pub async fn reset_tree_rpc(config: &Config) -> Result<RpcOutcome<ResetTreeResponse>, String> {
    // The derived-index reset — the table deletes, the chunk requeue and the
    // re-extraction enqueue — is the driver's now (`Maintenance::
    // reset_derived_index`), where the tables live. What stays here is what is
    // genuinely the host's: the rendered wiki summaries below are files this
    // host wrote under its own content root, and the driver has no business
    // knowing they exist.
    //
    // No host-side worker wake follows the call: the member's contract makes
    // the wake part of the operation itself — a reset that requeued without
    // waking would look identical to one that did nothing until the next
    // scheduled window, and the caller has no way to ask for the wake alone.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        return Err(format!(
            "reset_tree: driver '{}' does not serve Maintenance",
            binding.driver_id()
        ));
    };
    let outcome = maintenance
        .reset_derived_index()
        .await
        .map_err(|e| format!("reset_tree: {e}"))?;
    let (tree_rows_deleted, chunks_requeued, jobs_enqueued) = (
        outcome.rows_deleted,
        outcome.chunks_requeued,
        outcome.jobs_enqueued,
    );

    let summaries_dir = config
        .memory_tree_content_root()
        .join("wiki")
        .join("summaries");
    let remove_result = crate::openhuman::util::retry_with_backoff_async(
        "remove wiki/summaries",
        6,
        200,
        || async {
            tokio::fs::remove_dir_all(&summaries_dir)
                .await
                .context("remove_dir_all")
        },
    )
    .await;

    match remove_result {
        Ok(()) => log::debug!("[memory_tree::read::reset_tree] removed wiki/summaries"),
        Err(e) => {
            let is_not_found = e
                .chain()
                .find_map(|e| e.downcast_ref::<std::io::Error>())
                .is_some_and(|ioe| ioe.kind() == std::io::ErrorKind::NotFound);
            if !is_not_found {
                log::warn!(
                    "[memory_tree::read::reset_tree] failed to remove wiki/summaries: {:#}",
                    e
                )
            }
        }
    }

    let resp = ResetTreeResponse {
        tree_rows_deleted,
        chunks_requeued,
        jobs_enqueued,
    };

    let log = format!(
        "memory_tree::read: reset_tree tree_rows={} chunks={} jobs={}",
        resp.tree_rows_deleted, resp.chunks_requeued, resp.jobs_enqueued
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── flush_source_tree ────────────────────────────────────────────────────

/// Force a flush of one source's summary tree.
///
/// # Served by the driver, and `TreeFactory` is not a seam gap (#5560)
///
/// Served by the bound driver's `MemoryTree::flush_source_tree`. The wire
/// member is `tinymemory_bus::names::FLUSH_SOURCE_TREE`, which the pinned
/// 131-method contract carries, so this is a contract call and not a method
/// that answers `Unsupported` at run time.
///
/// This used to hold a live `Tree` object from the engine's
/// `tree_source::get_or_create_source_tree`, because both things it did next
/// wanted the object rather than a namespace: `TreeFactory::from_tree(&tree)
/// .label_strategy(&cfg)` picked the labelling policy from the tree's own
/// kind and scope, and `force_flush_tree` took `&tree.id`. **Neither is an
/// upstream ask** — they are not narrower doors waiting to be opened, they are
/// the two halves of a handle-passing shape the contract deliberately does not
/// have, and the member below replaces both.
///
/// tinymemory v1.7.0 replaced that with a member answering the seal count and
/// making the labelling decision driver-side — which is where it came from
/// anyway — so no tree handle crosses the seam. Sealing and cascading are one
/// call there rather than two here, which also closes the window the old
/// two-step left open: a tree sealed but not yet cascaded reads as an empty
/// tree to every structural query.
///
/// Two behaviours survive the rewrite deliberately. The `ACTIVE` re-entrancy
/// latch is still host-side, because it guards *this* handler against a second
/// concurrent call rather than guarding the store. And a scope with nothing
/// buffered is still a zero-count success rather than an error — now by the
/// contract's own rule rather than by this function's convention.
pub async fn flush_source_tree_rpc(
    config: &Config,
    source_scope: &str,
) -> Result<RpcOutcome<FlushSourceTreeResponse>, String> {
    use std::collections::HashSet;
    use std::sync::Mutex;

    static ACTIVE: std::sync::LazyLock<Mutex<HashSet<String>>> =
        std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

    /// Releases the re-entrancy latch on every exit, including the error ones.
    ///
    /// The hand-rolled version this replaces removed the scope only after a
    /// successful flush, so a driver error latched that scope out for the rest
    /// of the process — the retry a caller would naturally make came back
    /// "already running" forever. Dropping it in a guard is the fix.
    struct Latch(String);
    impl Drop for Latch {
        fn drop(&mut self) {
            ACTIVE
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .remove(&self.0);
        }
    }

    let scope = source_scope.to_string();
    {
        let mut active = ACTIVE.lock().unwrap_or_else(|e| e.into_inner());
        if !active.insert(scope.clone()) {
            return Ok(RpcOutcome::single_log(
                FlushSourceTreeResponse {
                    tree_scope: scope,
                    seals_fired: 0,
                },
                "memory_tree::read: flush_source_tree already running for this scope".to_string(),
            ));
        }
    }
    let _latch = Latch(scope.clone());

    // Asked of the driver (`Tree::flush_source_tree`). The seal and the cascade
    // are one call there rather than two here, which closes the window the old
    // two-step left: a tree sealed but not cascaded reads as empty to every
    // structural query. No `spawn_blocking` either — the driver owns whether
    // its own reads block.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(tree) = binding.provider().as_tree() else {
        return Err(format!(
            "flush_source_tree: driver '{}' does not serve Tree",
            binding.driver_id()
        ));
    };

    // A scope with nothing buffered is `Ok(0)` by the contract, not an error,
    // which is the behaviour this handler already had for an unknown scope.
    let seals_fired = tree
        .flush_source_tree(&scope)
        .await
        .map_err(|e| format!("flush_source_tree: {e}"))?;

    let resp = FlushSourceTreeResponse {
        tree_scope: scope,
        seals_fired: u32::try_from(seals_fired).unwrap_or(u32::MAX),
    };
    let log = format!(
        "memory_tree::read: flush_source_tree scope={} seals={}",
        resp.tree_scope, resp.seals_fired
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── flush_now ─────────────────────────────────────────────────────────────

/// Flush buffered work old enough to be written out, via the bound driver —
/// flush deduplication is keyed engine-side, so only the driver can promise
/// one enqueue per window.
pub async fn flush_now_rpc(config: &Config) -> Result<RpcOutcome<FlushNowResponse>, String> {
    // Asked of the driver (`Maintenance::flush_pending`): the buffer walk, the
    // window-keyed dedupe and the enqueue were engine mechanics the host was
    // re-implementing. No `spawn_blocking` — the driver owns whether its own
    // reads block. The wire shape is unchanged: `enqueued: false` still means
    // "already scheduled this window" when `stale_buffers` is non-zero.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        return Err(format!(
            "flush_now: driver '{}' does not serve Maintenance",
            binding.driver_id()
        ));
    };
    let outcome = maintenance
        .flush_pending()
        .await
        .map_err(|e| format!("flush_now: {e}"))?;
    let resp = FlushNowResponse {
        enqueued: outcome.enqueued,
        stale_buffers: u32::try_from(outcome.stale_buffers).unwrap_or(u32::MAX),
    };

    let log = format!(
        "memory_tree::read: flush_now enqueued={} stale_buffers={}",
        resp.enqueued, resp.stale_buffers
    );
    Ok(RpcOutcome::single_log(resp, log))
}

// ── delete_source ──────────────────────────────────────────────────────────

/// Fully delete one document source by its **exact** `source_id`.
///
/// Unlike [`super::entities::delete_chunk_rpc`] (which removes a single chunk
/// and leaves the source tree intact), this asks the bound driver to forget
/// everything filed under one `(source_kind, source_id)` pair, so the whole
/// logical source is purged: every chunk plus its score / entity-index /
/// embedding / reembed-skip side rows and chunk content files, the ingest
/// dedup gate, and — when the source becomes fully orphaned — its source
/// summary tree (summaries, summary embeddings + reembed-skip, tree-keyed
/// entity-index, buffers, the tree row, and summary content files). This
/// prevents stale summaries of a deleted note/event/meeting from resurfacing
/// in semantic recall.
///
/// Matching is **exact** (never a prefix) — that is what distinguishes
/// [`ForgetSelector::Source`] from [`ForgetSelector::SourcePrefix`] — so
/// sibling sources sharing a prefix are untouched. Idempotent: an unknown
/// `source_id` removes nothing and returns `deleted = false`.
///
/// Legacy cleanup is part of the same selector rather than a second host call.
/// A source whose chunks were already removed earlier by the per-chunk path
/// keeps a now-stale summary tree, which a chunk delete will not touch because
/// it only cascades trees for chunks it deletes in the same call. The driver's
/// exact-source arm sweeps that orphaned tree afterwards and reports it
/// separately as `ForgetOutcome::trees_cleaned`, which is why the two counts
/// are kept apart: a call that removes no chunk can still have done real work.
///
/// Scope: this deletes the exact document source and its **source-scoped**
/// orphan tree. It intentionally does NOT tear down shared collection /
/// `path_scope` trees (e.g. Notion `notion:{connection}`), which may summarise
/// many documents; per-document pruning inside a shared collection summary is
/// out of scope here.
pub async fn delete_source_rpc(
    config: &Config,
    source_id: String,
) -> Result<RpcOutcome<DeleteSourceResponse>, String> {
    let source_id = source_id.trim().to_string();
    if source_id.is_empty() {
        return Err("delete_source: source_id must be a non-empty string".to_string());
    }

    // No `spawn_blocking`: the driver owns whether its own deletes block, and
    // the module's do not run on this thread at all.
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(sources) = binding.provider().as_sources() else {
        // Refused rather than answered `deleted: false`. On a delete, a caller
        // that reads "nothing matched" concludes the content was already gone;
        // that is the same reason the contract refuses an unrecognised
        // `source_kind` instead of returning an outcome of zero.
        return Err(format!(
            "delete_source: driver '{}' does not serve Sources",
            binding.driver_id()
        ));
    };
    let outcome = sources
        .forget_matching(&ForgetSelector::Source {
            // A wire string, not the enum: the set of source kinds belongs to
            // this host's sync machinery and grows without a contract change.
            source_kind: SourceKind::Document.as_str().to_string(),
            source_id: source_id.clone(),
        })
        .await
        .map_err(|e| format!("delete_source: {e}"))?;

    let resp = DeleteSourceResponse {
        // `deleted` is true if we removed chunks OR cleaned a stale orphaned
        // tree (the legacy-cleanup case has chunks_removed == 0 but still did
        // work). `trees_cleaned` is a count where this used to read a `bool`,
        // because a prefix or owner selector can strand several trees; for the
        // exact-source arm here it is still only ever 0 or 1.
        deleted: outcome.chunks_removed > 0 || outcome.trees_cleaned > 0,
        chunks_removed: outcome.chunks_removed,
    };
    let log = format!(
        // Redact the source id: it can embed user-linked identifiers.
        "memory_tree::read: delete_source source_id_hash={} deleted={} chunks_removed={} trees_cleaned={}",
        crate::openhuman::util::redact::redact(&source_id),
        resp.deleted,
        resp.chunks_removed,
        outcome.trees_cleaned
    );
    Ok(RpcOutcome::single_log(resp, log))
}

#[cfg(test)]
#[path = "admin_tests.rs"]
mod tests;
