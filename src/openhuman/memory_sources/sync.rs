//! Per-source sync dispatcher.
//!
//! Thin routing layer: dispatches sync requests to the right backend:
//! - GitHub repos → `memory_sync::sources::github`
//! - Composio sources → `memory_sync::composio`
//! - Folder/RSS/WebPage → per-item ingest via reader + ingest pipeline
//! - Twitter → placeholder
//!
//! Sync runs in a `tokio::spawn`-ed task so the RPC returns immediately.
//! Progress is published as `MemorySyncStageChanged` events.
//!
//! A per-source mutex prevents duplicate concurrent syncs when the user
//! presses the sync button multiple times.

use std::collections::HashSet;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use futures::stream::{self, StreamExt};

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::ingest_document_with_scope;
use crate::openhuman::memory::sync::{emit_sync_stage, MemorySyncStage, MemorySyncTrigger};
use crate::openhuman::memory_sources::readers;
use crate::openhuman::memory_sources::types::{MemorySourceEntry, SourceKind};
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;
use crate::openhuman::memory_sync::composio::{self, ComposioUsage, SyncReason};

const SYNC_CONCURRENCY: usize = 10;

static ACTIVE_SYNCS: std::sync::LazyLock<Mutex<HashSet<String>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashSet::new()));

/// Trigger a sync for one source. Spawns work in the background and
/// returns immediately. Progress is published as `MemorySyncStageChanged`
/// events with `connection_id = Some(source.id)`.
pub async fn sync_source(source: MemorySourceEntry, config: Config) -> Result<(), String> {
    if !source.enabled {
        return Err(format!("source '{}' is disabled", source.id));
    }

    // Per-source mutex: reject if this source is already syncing.
    {
        let mut active = ACTIVE_SYNCS.lock().unwrap_or_else(|e| e.into_inner());
        if !active.insert(source.id.clone()) {
            tracing::debug!(
                source_id = %source.id,
                "[memory_sources:sync] already syncing — skipping duplicate"
            );
            return Ok(());
        }
    }

    let source_id = source.id.clone();
    let kind_str = source.kind.as_str();

    tracing::debug!(
        source_id = %source_id,
        kind = %kind_str,
        "[memory_sources:sync] queueing sync"
    );

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Requested,
        Some(kind_str),
        Some(&source_id),
        Some(format!("sync requested for {} source", kind_str)),
        Some(&source_id),
    );

    tokio::spawn(async move {
        let source_id_for_panic = source.id.clone();
        let kind_for_panic = source.kind.as_str();
        let inner = tokio::spawn(async move {
            // Retry any previously-failed pipeline jobs so the worker
            // resumes processing through all documents.
            if let Ok(retried) = crate::openhuman::memory_queue::store::retry_all_failed(&config) {
                if retried > 0 {
                    tracing::info!(
                        retried = retried,
                        "[memory_sources:sync] retried {retried} failed pipeline job(s)"
                    );
                }
            }

            tracing::debug!(
                source_id = %source.id,
                kind = %source.kind.as_str(),
                "[memory_sources:sync] dispatching by kind"
            );
            let sync_start = std::time::Instant::now();
            // Composio billable-action usage for this run, populated by
            // `sync_composio` (#3111). Stays zero for non-Composio kinds.
            let mut composio_usage = ComposioUsage::default();
            let outcome = match source.kind {
                SourceKind::Composio => {
                    sync_composio(&source, config.clone(), &mut composio_usage).await
                }
                SourceKind::Conversation => sync_items_individually(&source, &config).await,
                SourceKind::GithubRepo => {
                    // GitHub path writes its own detailed audit entry
                    // with token breakdowns; skip the dispatcher-level
                    // audit for this kind.
                    crate::openhuman::memory_sync::sources::github::run_github_sync(
                        &source, &config,
                    )
                    .await
                    .map(|o| o.records_ingested as usize)
                    .map_err(|e| format!("{e:#}"))
                }
                SourceKind::Folder | SourceKind::RssFeed | SourceKind::WebPage => {
                    sync_items_individually(&source, &config).await
                }
                SourceKind::TwitterQuery => Err(
                    "Twitter sync not yet configured. Provide bearer token in settings."
                        .to_string(),
                ),
            };
            let duration_ms = sync_start.elapsed().as_millis() as u64;

            match outcome {
                Ok(items) => {
                    tracing::debug!(
                        source_id = %source.id,
                        kind = %source.kind.as_str(),
                        items = items,
                        "[memory_sources:sync] completed"
                    );
                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Completed,
                        Some(source.kind.as_str()),
                        Some(&source.id),
                        Some(format!("ingested {items} item(s)")),
                        Some(&source.id),
                    );

                    // Write audit entry (GitHub writes its own with
                    // token detail; other kinds get a simpler entry).
                    if source.kind != SourceKind::GithubRepo {
                        use crate::openhuman::memory_sync::sources::audit::{
                            append_audit_entry, SyncAuditEntry,
                        };
                        append_audit_entry(
                            &config,
                            &SyncAuditEntry {
                                timestamp: chrono::Utc::now(),
                                source_id: source.id.clone(),
                                source_kind: source.kind.as_str().to_string(),
                                scope: source
                                    .url
                                    .clone()
                                    .or(source.toolkit.clone())
                                    .unwrap_or_else(|| source.id.clone()),
                                items_fetched: items as u32,
                                batches: 0,
                                input_tokens: 0,
                                output_tokens: 0,
                                estimated_cost_usd: 0.0,
                                composio_actions_called: composio_usage.actions_called,
                                composio_cost_usd: composio_usage.cost_usd,
                                actual_charged_usd: None,
                                duration_ms,
                                success: true,
                                error: None,
                            },
                        );
                    }

                    // Auto-rebuild: if raw files exist but the tree has
                    // no summaries, build the tree now.
                    check_and_rebuild_tree(&source, &config).await;

                    // Auto-snapshot: capture post-sync state for diff tracking.
                    if let Err(e) = crate::openhuman::memory_diff::ops::auto_snapshot_after_sync(
                        &source, &config,
                    )
                    .await
                    {
                        tracing::warn!(
                            source_id = %source.id,
                            error = %e,
                            "[memory_sources:sync] auto-snapshot failed (non-fatal)"
                        );
                    }
                }
                Err(error) => {
                    // Audit failed syncs too.
                    use crate::openhuman::memory_sync::sources::audit::{
                        append_audit_entry, SyncAuditEntry,
                    };
                    append_audit_entry(
                        &config,
                        &SyncAuditEntry {
                            timestamp: chrono::Utc::now(),
                            source_id: source.id.clone(),
                            source_kind: source.kind.as_str().to_string(),
                            scope: source
                                .url
                                .clone()
                                .or(source.toolkit.clone())
                                .unwrap_or_else(|| source.id.clone()),
                            items_fetched: 0,
                            batches: 0,
                            input_tokens: 0,
                            output_tokens: 0,
                            estimated_cost_usd: 0.0,
                            composio_actions_called: composio_usage.actions_called,
                            composio_cost_usd: composio_usage.cost_usd,
                            actual_charged_usd: None,
                            duration_ms,
                            success: false,
                            error: Some(error.clone()),
                        },
                    );

                    // Report internal failures to Sentry; known-expected
                    // conditions (auth/network/rate-limit/missing config) are
                    // classified by `expected_error_kind` and logged-not-reported
                    // so we surface real bugs without Sentry-spamming routine
                    // user/config errors (#3295). The reason is still shown to
                    // the user via the Failed stage event regardless.
                    crate::core::observability::report_error_or_expected(
                        &error,
                        "memory_sources",
                        "sync",
                        &[
                            ("source_id", source.id.as_str()),
                            ("kind", source.kind.as_str()),
                        ],
                    );

                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Failed,
                        Some(source.kind.as_str()),
                        Some(&source.id),
                        Some(error.clone()),
                        Some(&source.id),
                    );
                    tracing::warn!(
                        source_id = %source.id,
                        kind = %source.kind.as_str(),
                        error = %error,
                        "[memory_sources:sync] failed"
                    );
                }
            }
        });

        if let Err(join_err) = inner.await {
            if join_err.is_panic() {
                tracing::error!(
                    source_id = %source_id_for_panic,
                    kind = %kind_for_panic,
                    "[memory_sources:sync] sync task panicked"
                );
            }
        }

        // Release the per-source lock so future syncs can proceed.
        if let Ok(mut active) = ACTIVE_SYNCS.lock() {
            active.remove(&source_id_for_panic);
        }
    });

    Ok(())
}

async fn sync_composio(
    source: &MemorySourceEntry,
    config: Config,
    usage_out: &mut ComposioUsage,
) -> Result<usize, String> {
    let connection_id = source
        .connection_id
        .as_deref()
        .ok_or("composio source missing connection_id")?;

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some("composio"),
        Some(&source.id),
        Some(format!("delegating to composio sync for {connection_id}")),
        Some(&source.id),
    );

    match composio::run_connection_sync(config, connection_id, SyncReason::Manual).await {
        Ok((outcome, usage)) => {
            *usage_out = usage;
            Ok(outcome.items_ingested)
        }
        Err((e, usage)) => {
            *usage_out = usage;
            Err(format!("composio sync failed: {e}"))
        }
    }
}

/// Per-item sync path for Folder/RSS/WebPage sources.
async fn sync_items_individually(
    source: &MemorySourceEntry,
    config: &Config,
) -> Result<usize, String> {
    let reader = readers::reader_for(&source.kind);

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some(source.kind.as_str()),
        Some(&source.id),
        Some("listing items".to_string()),
        Some(&source.id),
    );

    let items = reader.list_items(source, config).await?;
    let total = items.len();

    // Reconcile before re-ingesting: for LOCAL FOLDER sources only, drop chunks
    // (rows + on-disk bodies) for items previously ingested under this source
    // that no longer exist on disk — e.g. a renamed or deleted file. Each item
    // ingests under the content-addressed composite id
    // `mem_src:{source_id}:{item_id}` as a Document, so a rename mints a fresh id
    // and orphans the old chunk + its on-disk body; without this the stale body
    // lingers forever and can only ever be served as a ≤500-char preview (#4689).
    //
    // Runs BEFORE the empty-listing early return so an emptied folder (every file
    // deleted → total == 0) still reconciles instead of leaving all its chunks
    // behind. This is safe on an empty or partial listing because
    // `prune_vanished_items` re-checks each candidate on disk and only deletes
    // files that are provably absent, so a transient listing miss (EACCES /
    // EMFILE / stat stall) is never mistaken for a deletion.
    //
    // Restricted to Folder: for feed / web / conversation sources, absence from
    // the current listing means "rolled off / not re-fetched", NOT "deleted", so
    // pruning them would irrecoverably delete valid archived items.
    if source_supports_prune(&source.kind) {
        if let Some(base_path) = source.path.clone() {
            let config = config.clone();
            let source_id = source.id.clone();
            let live: HashSet<String> = items
                .iter()
                .map(|item| format!("mem_src:{source_id}:{}", item.id))
                .collect();
            if let Err(e) = tokio::task::spawn_blocking(move || {
                prune_vanished_items(&config, &source_id, std::path::Path::new(&base_path), &live)
            })
            .await
            {
                tracing::warn!(error = %e, "[memory_sources:sync] prune join error");
            }
        }
    }

    if total == 0 {
        return Ok(0);
    }

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Stored,
        Some(source.kind.as_str()),
        Some(&source.id),
        Some(format!("{total} item(s) discovered")),
        Some(&source.id),
    );

    let ingested = Arc::new(AtomicUsize::new(0));
    let processed = Arc::new(AtomicUsize::new(0));
    let source_id = source.id.clone();
    let source_kind = source.kind.clone();
    let kind_str = source.kind.as_str().to_string();

    stream::iter(items.iter().enumerate())
        .for_each_concurrent(SYNC_CONCURRENCY, |(_, item)| {
            let config = config.clone();
            let source_kind = source_kind.clone();
            let reader = readers::reader_for(&source_kind);
            let source_clone = source.clone();
            let ingested = Arc::clone(&ingested);
            let processed = Arc::clone(&processed);
            let source_id = source_id.clone();
            let kind_str = kind_str.clone();

            async move {
                let content = match reader.read_item(&source_clone, &item.id, &config).await {
                    Ok(c) => c,
                    Err(e) => {
                        tracing::warn!(
                            item_id = %item.id,
                            error = %e,
                            "[memory_sources:sync] skipping item — read failed"
                        );
                        processed.fetch_add(1, Ordering::Relaxed);
                        return;
                    }
                };

                let doc = DocumentInput {
                    provider: format!("memory_sources:{kind_str}"),
                    title: content.title.clone(),
                    body: content.body.clone(),
                    modified_at: chrono::Utc::now(),
                    source_ref: Some(format!("{source_id}:{}", item.id)),
                };

                let composite_source_id = format!("mem_src:{source_id}:{}", item.id);
                let tags = vec!["memory_sources".to_string(), kind_str.clone()];

                match ingest_document_with_scope(
                    &config,
                    &composite_source_id,
                    "user",
                    tags,
                    doc,
                    None,
                )
                .await
                {
                    Ok(result) => {
                        if !result.already_ingested {
                            ingested.fetch_add(1, Ordering::Relaxed);
                        }
                    }
                    Err(e) => {
                        tracing::warn!(
                            item_id = %item.id,
                            error = %e,
                            "[memory_sources:sync] ingest failed for item"
                        );
                    }
                }

                let done = processed.fetch_add(1, Ordering::Relaxed) + 1;
                let new = ingested.load(Ordering::Relaxed);
                if done.is_multiple_of(10) || done == total {
                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Ingesting,
                        Some(&kind_str),
                        Some(&source_id),
                        Some(format!("{done}/{total} processed ({new} new)")),
                        Some(&source_id),
                    );
                }
            }
        })
        .await;

    Ok(ingested.load(Ordering::Relaxed))
}

/// Whether a source kind has authoritative present/absent semantics on the
/// local disk, so that "absent from the current listing" genuinely means
/// "deleted" and can drive a prune (#4689).
///
/// Only `Folder` qualifies: for `RssFeed` / `WebPage`, `list_items` returns a
/// rolling, `max_items`-truncated window, so an item missing from it merely
/// rolled off the feed and must never be deleted; `Conversation` threads have
/// no delete-follows-listing contract. Restricting prune here prevents turning
/// an append-only archive into a destructive mirror of the latest window.
fn source_supports_prune(kind: &SourceKind) -> bool {
    matches!(kind, SourceKind::Folder)
}

/// Delete chunks (rows + on-disk bodies) for items previously ingested under
/// `source_id` whose backing file no longer exists under `base_path` — the
/// reconcile step that keeps a folder resync from orphaning renamed or deleted
/// files (#4689).
///
/// Safety: a candidate is deleted ONLY when its file is provably absent
/// (`symlink_metadata` returns `NotFound`). A transient listing miss (the reader
/// dropped a still-present file on an `EACCES` / `EMFILE` / stat stall) leaves
/// the file on disk, so the re-check keeps it — absence from `live` alone is
/// never sufficient to delete.
///
/// Blocking DB + FS work; call from `spawn_blocking`. Chunks land under the
/// content (`Document`) `SourceKind`, not the outer `memory_sources` source kind.
fn prune_vanished_items(
    config: &Config,
    source_id: &str,
    base_path: &std::path::Path,
    live: &HashSet<String>,
) {
    use crate::openhuman::memory_store::chunks::store as chunk_store;
    use crate::openhuman::memory_store::chunks::types::SourceKind as ChunkSourceKind;

    let prefix = format!("mem_src:{source_id}:");
    let previously = match chunk_store::list_source_ids_with_prefix(
        config,
        ChunkSourceKind::Document,
        &prefix,
    ) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(
                source_id = %source_id,
                error = %format!("{e:#}"),
                "[memory_sources:sync] prune: failed to list previously-ingested items"
            );
            return;
        }
    };

    let mut removed_chunks = 0usize;
    let mut removed_items = 0usize;
    for stale in previously.into_iter().filter(|sid| !live.contains(sid)) {
        // Recover the item's relative path from the composite id and confirm the
        // file is genuinely gone before deleting. Anything other than a definite
        // NotFound (present file, or an ambiguous EACCES/IO error) is treated as
        // "keep" so a transient listing miss can never delete live data.
        let Some(rel) = stale.strip_prefix(&prefix) else {
            continue;
        };
        // Defense-in-depth: `rel` comes from a stored composite id. If it were
        // ever empty, absolute, or contained `..`, `base_path.join(rel)` could
        // resolve outside the source folder (on Unix an absolute `rel` silently
        // discards `base_path`), and a `NotFound` there would delete real chunk
        // rows. The current folder reader can't produce such ids, so keep any
        // such candidate rather than risk deleting on a path we can't vouch for.
        if rel.is_empty() || rel.contains("..") || std::path::Path::new(rel).is_absolute() {
            tracing::warn!(
                source_id = %source_id,
                "[memory_sources:sync] prune: skipping candidate with unsafe relative path"
            );
            continue;
        }
        match std::fs::symlink_metadata(base_path.join(rel)) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => { /* absent → prune */ }
            _ => continue,
        }
        match chunk_store::delete_chunks_by_source(config, ChunkSourceKind::Document, &stale) {
            Ok(n) => {
                removed_chunks += n;
                removed_items += 1;
            }
            Err(e) => tracing::warn!(
                source_id = %source_id,
                error = %format!("{e:#}"),
                "[memory_sources:sync] prune: delete failed for a vanished item"
            ),
        }
    }
    if removed_items > 0 {
        tracing::info!(
            source_id = %source_id,
            items = removed_items,
            chunks = removed_chunks,
            "[memory_sources:sync] pruned chunks for vanished items"
        );
    }
}

/// Derive the tree scope(s) for a source and reconcile any raw files that
/// are not yet covered by tree summaries (incremental — see
/// `memory_sync::sources::rebuild`).
pub(crate) async fn check_and_rebuild_tree(source: &MemorySourceEntry, config: &Config) {
    use crate::openhuman::memory_sync::sources::rebuild::{needs_rebuild, rebuild_tree_from_raw};

    let scopes = derive_scopes(source, config);
    for scope in scopes {
        if !needs_rebuild(config, &scope.tree_scope, &scope.archive_source_id) {
            continue;
        }
        tracing::info!(
            source_id = %source.id,
            scope = %scope.tree_scope,
            archive = %scope.archive_source_id,
            "[memory_sources:sync] reconciling uncovered raw files into tree"
        );
        match rebuild_tree_from_raw(config, &scope.tree_scope, &scope.archive_source_id).await {
            Ok(outcome) => {
                tracing::info!(
                    scope = %scope.tree_scope,
                    files = outcome.files_read,
                    batches = outcome.batches,
                    cost = %format!(
                        "${:.4}",
                        outcome.actual_charged_usd.unwrap_or(outcome.estimated_cost_usd)
                    ),
                    cost_is_actual = outcome.actual_charged_usd.is_some(),
                    "[memory_sources:sync] reconcile complete"
                );
            }
            Err(e) => {
                tracing::warn!(
                    scope = %scope.tree_scope,
                    error = %format!("{e:#}"),
                    "[memory_sources:sync] reconcile failed"
                );
            }
        }
    }
}

/// A source's tree scope paired with its raw-archive source id. The two
/// slugify to DIFFERENT directories for GitHub (`github:owner/repo` vs
/// `github.com/owner/repo`) — conflating them makes reconcile scan an
/// empty directory while the real archive sits uncovered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct SourceScope {
    /// Tree registry key, e.g. `"github:owner/repo"`.
    pub tree_scope: String,
    /// Raw-archive id whose slug names `raw/<slug>/`, e.g.
    /// `"github.com/owner/repo"`. Equal to `tree_scope` for sources that
    /// archive under their scope (gmail).
    pub archive_source_id: String,
}

/// Derive the tree scope(s) + raw-archive id(s) that a source maps to.
pub(crate) fn derive_scopes(source: &MemorySourceEntry, config: &Config) -> Vec<SourceScope> {
    use crate::openhuman::memory_sources::readers::github;

    match source.kind {
        SourceKind::GithubRepo => {
            let Some(url) = source.url.as_deref() else {
                return Vec::new();
            };
            match (
                github::repo_chunk_scope(url),
                github::repo_archive_source_id(url),
            ) {
                (Some(tree_scope), Some(archive_source_id)) => vec![SourceScope {
                    tree_scope,
                    archive_source_id,
                }],
                _ => Vec::new(),
            }
        }
        SourceKind::Composio => {
            // Composio sources scope by toolkit + connection email.
            // Gmail: "gmail:<slug_account_email>" — archive dir shares
            // the scope. Others: no raw archive to reconcile yet.
            let toolkit = source.toolkit.as_deref().unwrap_or("unknown");
            match toolkit {
                "gmail" | "GMAIL" => {
                    // The scope for gmail is "gmail:<slugified_email>".
                    // We scan the raw directory to find it.
                    let content_root = config.memory_tree_content_root();
                    let raw_dir = content_root.join("raw");
                    if let Ok(entries) = std::fs::read_dir(&raw_dir) {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                e.file_name()
                                    .to_str()
                                    .map(|n| n.starts_with("gmail-"))
                                    .unwrap_or(false)
                            })
                            .filter_map(|e| {
                                // Read _source.md to get the scope.
                                let source_md = e.path().join("_source.md");
                                let content = std::fs::read_to_string(&source_md).ok()?;
                                content.lines().find(|l| l.starts_with("scope:")).map(|l| {
                                    let scope = l
                                        .trim_start_matches("scope:")
                                        .trim()
                                        .trim_matches('"')
                                        .to_string();
                                    SourceScope {
                                        tree_scope: scope.clone(),
                                        archive_source_id: scope,
                                    }
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_store::chunks::store as chunk_store;
    use crate::openhuman::memory_store::chunks::types::{
        chunk_id, Chunk, Metadata, SourceKind as ChunkSourceKind,
    };
    use crate::openhuman::memory_store::content::stage_chunks;
    use chrono::TimeZone;
    use tempfile::TempDir;

    fn doc_chunk(source_id: &str) -> Chunk {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: chunk_id(ChunkSourceKind::Document, source_id, 0, "body"),
            content: format!("body of {source_id}"),
            metadata: Metadata {
                source_kind: ChunkSourceKind::Document,
                source_id: source_id.into(),
                owner: "user".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: None,
                path_scope: None,
            },
            token_count: 2,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    fn seed(cfg: &Config, chunk: &Chunk) {
        let staged =
            stage_chunks(&cfg.memory_tree_content_root(), std::slice::from_ref(chunk)).unwrap();
        chunk_store::with_connection(cfg, |conn| {
            let tx = conn.unchecked_transaction()?;
            chunk_store::upsert_staged_chunks_tx(&tx, &staged)?;
            tx.commit()?;
            Ok(())
        })
        .unwrap();
    }

    #[test]
    fn source_supports_prune_only_for_folder() {
        assert!(source_supports_prune(&SourceKind::Folder));
        assert!(!source_supports_prune(&SourceKind::RssFeed));
        assert!(!source_supports_prune(&SourceKind::WebPage));
        assert!(!source_supports_prune(&SourceKind::Conversation));
    }

    #[test]
    fn prune_vanished_items_removes_only_files_absent_from_disk() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        // Folder base holds only b.md on disk; a.md was renamed/deleted.
        let base = tmp.path().join("folder");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("b.md"), b"b").unwrap();

        seed(&cfg, &doc_chunk("mem_src:src1:a.md"));
        seed(&cfg, &doc_chunk("mem_src:src1:b.md"));
        assert_eq!(chunk_store::count_chunks(&cfg).unwrap(), 2);

        let live: HashSet<String> = ["mem_src:src1:b.md".to_string()].into_iter().collect();
        prune_vanished_items(&cfg, "src1", &base, &live);

        let remaining = chunk_store::list_source_ids_with_prefix(
            &cfg,
            ChunkSourceKind::Document,
            "mem_src:src1:",
        )
        .unwrap();
        assert_eq!(remaining, vec!["mem_src:src1:b.md".to_string()]);
        assert_eq!(chunk_store::count_chunks(&cfg).unwrap(), 1);
    }

    #[test]
    fn prune_vanished_items_keeps_still_present_file_missed_by_listing() {
        // Safety guard (#4689 review): a transient listing miss must not delete a
        // file that is still on disk. 'a.md' is absent from `live` but present on
        // disk → it must be kept.
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        let base = tmp.path().join("folder");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("a.md"), b"still here").unwrap();

        seed(&cfg, &doc_chunk("mem_src:src3:a.md"));
        // 'a.md' dropped from the listing (e.g. EACCES/EMFILE) though it exists.
        let live: HashSet<String> = HashSet::new();
        prune_vanished_items(&cfg, "src3", &base, &live);

        // Not deleted — the on-disk re-check kept it.
        assert_eq!(chunk_store::count_chunks(&cfg).unwrap(), 1);
    }

    #[test]
    fn prune_vanished_items_prunes_when_folder_emptied() {
        // Emptied folder: listing is empty (live = {}) and no files remain on
        // disk, so the previously-ingested chunk must be pruned (the reason prune
        // now runs before the total == 0 early return).
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        let base = tmp.path().join("empty_folder");
        std::fs::create_dir_all(&base).unwrap();

        seed(&cfg, &doc_chunk("mem_src:src4:gone.md"));
        let live: HashSet<String> = HashSet::new();
        prune_vanished_items(&cfg, "src4", &base, &live);
        assert_eq!(chunk_store::count_chunks(&cfg).unwrap(), 0);
    }

    #[test]
    fn prune_vanished_items_keeps_candidate_with_unsafe_relative_path() {
        // Defense-in-depth: a stored id whose relative path is absolute must not
        // be pruned even when its file is "absent" (join would escape the base).
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        let base = tmp.path().join("folder");
        std::fs::create_dir_all(&base).unwrap();

        seed(&cfg, &doc_chunk("mem_src:src5:/etc/hostname"));
        let live: HashSet<String> = HashSet::new();
        prune_vanished_items(&cfg, "src5", &base, &live);
        // Not deleted — the unsafe-path guard skipped it.
        assert_eq!(chunk_store::count_chunks(&cfg).unwrap(), 1);
    }

    #[test]
    fn list_source_ids_with_prefix_isolates_sibling_prefixes() {
        // `mem_src:src1:` must not match `mem_src:src10:` items.
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();

        seed(&cfg, &doc_chunk("mem_src:src1:a.md"));
        seed(&cfg, &doc_chunk("mem_src:src1:c.md"));
        seed(&cfg, &doc_chunk("mem_src:src10:b.md"));

        let mut got = chunk_store::list_source_ids_with_prefix(
            &cfg,
            ChunkSourceKind::Document,
            "mem_src:src1:",
        )
        .unwrap();
        got.sort();
        assert_eq!(
            got,
            vec![
                "mem_src:src1:a.md".to_string(),
                "mem_src:src1:c.md".to_string()
            ]
        );
    }

    #[test]
    fn prune_vanished_items_is_noop_when_all_live() {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();

        seed(&cfg, &doc_chunk("mem_src:src2:a.md"));
        let live: HashSet<String> = ["mem_src:src2:a.md".to_string()].into_iter().collect();
        prune_vanished_items(&cfg, "src2", tmp.path(), &live);
        assert_eq!(chunk_store::count_chunks(&cfg).unwrap(), 1);
    }
}
