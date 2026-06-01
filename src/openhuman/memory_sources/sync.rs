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
use crate::openhuman::memory_sync::composio::{self, SyncReason};

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
            let outcome = match source.kind {
                SourceKind::Composio => sync_composio(&source, config.clone()).await,
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
                            actual_charged_usd: None,
                            duration_ms,
                            success: false,
                            error: Some(error.clone()),
                        },
                    );

                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Failed,
                        Some(source.kind.as_str()),
                        Some(&source.id),
                        Some(error.clone()),
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

async fn sync_composio(source: &MemorySourceEntry, config: Config) -> Result<usize, String> {
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
    );

    let outcome = composio::run_connection_sync(config, connection_id, SyncReason::Manual)
        .await
        .map_err(|e| format!("composio sync failed: {e}"))?;

    Ok(outcome.items_ingested)
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
    );

    let items = reader.list_items(source, config).await?;
    let total = items.len();

    if total == 0 {
        return Ok(0);
    }

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Stored,
        Some(source.kind.as_str()),
        Some(&source.id),
        Some(format!("{total} item(s) discovered")),
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
                if done % 10 == 0 || done == total {
                    emit_sync_stage(
                        MemorySyncTrigger::Manual,
                        MemorySyncStage::Ingesting,
                        Some(&kind_str),
                        Some(&source_id),
                        Some(format!("{done}/{total} processed ({new} new)")),
                    );
                }
            }
        })
        .await;

    Ok(ingested.load(Ordering::Relaxed))
}

/// Derive the tree scope(s) for a source and rebuild from raw if needed.
async fn check_and_rebuild_tree(source: &MemorySourceEntry, config: &Config) {
    use crate::openhuman::memory_sync::sources::rebuild::{needs_rebuild, rebuild_tree_from_raw};

    let scopes = derive_scopes(source, config);
    for scope in scopes {
        if !needs_rebuild(config, &scope) {
            continue;
        }
        tracing::info!(
            source_id = %source.id,
            scope = %scope,
            "[memory_sources:sync] auto-rebuilding tree from raw"
        );
        match rebuild_tree_from_raw(config, &scope).await {
            Ok(outcome) => {
                tracing::info!(
                    scope = %scope,
                    files = outcome.files_read,
                    batches = outcome.batches,
                    cost = %format!(
                        "${:.4}",
                        outcome.actual_charged_usd.unwrap_or(outcome.estimated_cost_usd)
                    ),
                    cost_is_actual = outcome.actual_charged_usd.is_some(),
                    "[memory_sources:sync] rebuild complete"
                );
            }
            Err(e) => {
                tracing::warn!(
                    scope = %scope,
                    error = %format!("{e:#}"),
                    "[memory_sources:sync] rebuild failed"
                );
            }
        }
    }
}

/// Derive the tree scope string(s) that a source maps to.
fn derive_scopes(source: &MemorySourceEntry, config: &Config) -> Vec<String> {
    use crate::openhuman::memory_sources::readers::github;
    use crate::openhuman::memory_store::content::raw::slug_account_email;

    match source.kind {
        SourceKind::GithubRepo => {
            // GitHub sync already builds its own tree — but check anyway.
            source
                .url
                .as_deref()
                .and_then(github::repo_chunk_scope)
                .into_iter()
                .collect()
        }
        SourceKind::Composio => {
            // Composio sources scope by toolkit + connection email.
            // Gmail: "gmail:<slug_account_email>"
            // Others: "composio:<toolkit>:<connection_id>"
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
                                    l.trim_start_matches("scope:")
                                        .trim()
                                        .trim_matches('"')
                                        .to_string()
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
