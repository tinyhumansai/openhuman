//! Per-source sync orchestration.
//!
//! Dispatches sync requests to the right backend based on source kind:
//! - Composio sources delegate to `memory_sync::composio::run_connection_sync`
//! - Folder/GitHub/RSS/WebPage sources walk items via the reader and
//!   ingest each one through `memory::ingest_pipeline::ingest_document`
//! - Twitter is a placeholder until credentials wiring lands
//!
//! Sync runs in a `tokio::spawn`-ed task so the RPC returns immediately
//! after queueing. Progress is published as `MemorySyncStageChanged`
//! events on the global bus and UI subscribers stream them per source id.

use crate::openhuman::config::Config;
use crate::openhuman::memory::ingest_pipeline::{ingest_document, IngestResult};
use crate::openhuman::memory::sync::{emit_sync_stage, MemorySyncStage, MemorySyncTrigger};
use crate::openhuman::memory_sources::readers;
use crate::openhuman::memory_sources::readers::github;
use crate::openhuman::memory_sources::types::{
    MemorySourceEntry, SourceContent, SourceItem, SourceKind,
};
use crate::openhuman::memory_store::chunks::store::{set_chunk_raw_refs, RawRef};
use crate::openhuman::memory_store::content::raw::{self as raw_store, raw_rel_path, RawItem};
use crate::openhuman::memory_sync::canonicalize::document::DocumentInput;
use crate::openhuman::memory_sync::composio::{self, SyncReason};

/// Trigger a sync for one source. Spawns work in the background and
/// returns immediately. Progress is published as `MemorySyncStageChanged`
/// events with `connection_id = Some(source.id)`.
pub async fn sync_source(source: MemorySourceEntry, config: Config) -> Result<(), String> {
    if !source.enabled {
        return Err(format!("source '{}' is disabled", source.id));
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

    // Outer spawn catches panics so a panic in the sync task is surfaced
    // as a tracing::error! log rather than silently dropping the join handle.
    tokio::spawn(async move {
        let source_id_for_panic = source.id.clone();
        let kind_for_panic = source.kind.as_str();
        let inner = tokio::spawn(async move {
            tracing::debug!(
                source_id = %source.id,
                kind = %source.kind.as_str(),
                "[memory_sources:sync] dispatching by kind"
            );
            let outcome = match source.kind {
                SourceKind::Composio => sync_composio(&source, config).await,
                SourceKind::Folder
                | SourceKind::GithubRepo
                | SourceKind::RssFeed
                | SourceKind::WebPage => sync_via_reader(&source, config).await,
                SourceKind::TwitterQuery => Err(
                    "Twitter sync not yet configured. Provide bearer token in settings."
                        .to_string(),
                ),
            };

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
                }
                Err(error) => {
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

async fn sync_via_reader(source: &MemorySourceEntry, config: Config) -> Result<usize, String> {
    let reader = readers::reader_for(&source.kind);

    emit_sync_stage(
        MemorySyncTrigger::Manual,
        MemorySyncStage::Fetching,
        Some(source.kind.as_str()),
        Some(&source.id),
        Some("listing items".to_string()),
    );

    let items = reader.list_items(source, &config).await?;
    let total = items.len();
    tracing::debug!(
        source_id = %source.id,
        kind = %source.kind.as_str(),
        total = total,
        "[memory_sources:sync] reader.list_items returned items"
    );

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

    let mut ingested = 0usize;
    for (idx, item) in items.iter().enumerate() {
        let content = match reader.read_item(source, &item.id, &config).await {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!(
                    item_id = %item.id,
                    error = %e,
                    "[memory_sources:sync] skipping item — read failed"
                );
                continue;
            }
        };

        let doc = DocumentInput {
            provider: format!("memory_sources:{}", source.kind.as_str()),
            title: content.title.clone(),
            body: content.body.clone(),
            modified_at: chrono::Utc::now(),
            source_ref: Some(format!("{}:{}", source.id, item.id)),
        };

        // GitHub items use a clean, repo-scoped chunk source id
        // (`github:<owner>/<repo>:<item_id>`) instead of the opaque
        // `mem_src:src_<uuid>:…` form. Other reader kinds keep the
        // generic composite id.
        let composite_source_id = if source.kind == SourceKind::GithubRepo {
            source
                .url
                .as_deref()
                .and_then(|url| github::chunk_source_id(url, &item.id))
                .unwrap_or_else(|| format!("mem_src:{}:{}", source.id, item.id))
        } else {
            format!("mem_src:{}:{}", source.id, item.id)
        };
        let mut tags = vec![
            "memory_sources".to_string(),
            source.kind.as_str().to_string(),
        ];
        // Prioritise GitHub commit messages and closed/merged issues & PRs
        // when building the memory tree — the scorer boosts PRIORITY_TAG
        // chunks (see `memory_tree::score`).
        if source.kind == SourceKind::GithubRepo && github_item_is_high_priority(&item.id, &content)
        {
            tags.push(crate::openhuman::memory_tree::score::PRIORITY_TAG.to_string());
        }

        match ingest_document(&config, &composite_source_id, "user", tags, doc).await {
            Ok(result) => {
                if !result.already_ingested {
                    ingested += 1;
                }
                // Mirror GitHub items into a browsable, repo-grouped raw
                // archive (`raw/github-com-<owner>-<repo>/{commits,issues,prs}/`)
                // and point the chunks at it. Best-effort: archiving never
                // fails the sync.
                if source.kind == SourceKind::GithubRepo {
                    archive_github_raw(&config, source, item, &content, &result);
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

        // Emit progress every 5 items or at the end
        if (idx + 1) % 5 == 0 || idx + 1 == total {
            emit_sync_stage(
                MemorySyncTrigger::Manual,
                MemorySyncStage::Ingesting,
                Some(source.kind.as_str()),
                Some(&source.id),
                Some(format!("{}/{total} processed", idx + 1)),
            );
        }
    }

    Ok(ingested)
}

/// Whether a GitHub item should be treated as high-priority source
/// material when building the memory tree.
///
/// Commit messages are always high-priority; issues and PRs are
/// high-priority once **closed** (issues/PRs) or **merged** (PRs) — a
/// resolved thread carries the decision/outcome, which is the part worth
/// remembering. Open items stay at the default priority.
fn github_item_is_high_priority(item_id: &str, content: &SourceContent) -> bool {
    if item_id.starts_with("commit:") {
        return true;
    }
    let state_closed = content
        .metadata
        .get("state")
        .and_then(|v| v.as_str())
        .map(|s| s.eq_ignore_ascii_case("closed"))
        .unwrap_or(false);
    let merged = content
        .metadata
        .get("merged")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    state_closed || merged
}

/// Mirror a freshly-ingested GitHub item into the on-disk raw archive and
/// tag its chunks with a `RawRef` pointing at the archive file.
///
/// Layout: `raw/github-com-<owner>-<repo>/{commits,issues,prs}/<ts>_<uid>.md`,
/// where `<uid>` is the commit SHA / issue number / PR number. The body
/// written is the reader's already-rendered markdown (`content.body`), which
/// carries the commit message / issue conversation + metadata / PR body +
/// metadata the memory tree is built from.
///
/// Best-effort: any failure here is logged and swallowed — the chunked
/// content has already been persisted by `ingest_document`.
fn archive_github_raw(
    config: &Config,
    source: &MemorySourceEntry,
    item: &SourceItem,
    content: &SourceContent,
    result: &IngestResult,
) {
    let Some(url) = source.url.as_deref() else {
        return;
    };
    let Some(raw_source_id) = github::repo_archive_source_id(url) else {
        tracing::warn!(
            source_id = %source.id,
            "[memory_sources:github] could not derive raw archive id from url"
        );
        return;
    };
    let Some((kind, uid)) = github::raw_archive_coords(&item.id) else {
        return;
    };
    let created_at_ms = item.updated_at_ms.unwrap_or(0);
    let content_root = config.memory_tree_content_root();

    let raw_item = RawItem {
        uid: &uid,
        created_at_ms,
        markdown: &content.body,
        kind,
    };
    if let Err(e) = raw_store::write_raw_items(
        &content_root,
        &raw_source_id,
        std::slice::from_ref(&raw_item),
    ) {
        tracing::warn!(
            item_id = %item.id,
            error = %e,
            "[memory_sources:github] raw archive write failed"
        );
        return;
    }

    let rel_path = raw_rel_path(&raw_source_id, kind, created_at_ms, &uid);
    let refs = vec![RawRef {
        path: rel_path,
        start: 0,
        end: None,
    }];
    for chunk_id in &result.chunk_ids {
        if let Err(e) = set_chunk_raw_refs(config, chunk_id, &refs) {
            tracing::warn!(
                chunk_id = %chunk_id,
                error = %format!("{e:#}"),
                "[memory_sources:github] set raw ref failed"
            );
        }
    }
    tracing::debug!(
        item_id = %item.id,
        archive = %raw_source_id,
        chunks = result.chunk_ids.len(),
        "[memory_sources:github] archived raw item"
    );
}

#[cfg(test)]
mod tests {
    use super::github_item_is_high_priority;
    use crate::openhuman::memory_sources::types::{ContentType, SourceContent};

    fn content_with(meta: serde_json::Value) -> SourceContent {
        SourceContent {
            id: "x".into(),
            title: "t".into(),
            body: "b".into(),
            content_type: ContentType::Markdown,
            metadata: meta,
        }
    }

    #[test]
    fn commits_are_always_high_priority() {
        let c = content_with(serde_json::json!({}));
        assert!(github_item_is_high_priority("commit:abc123", &c));
    }

    #[test]
    fn closed_issue_is_high_priority_open_is_not() {
        let closed = content_with(serde_json::json!({ "state": "closed" }));
        assert!(github_item_is_high_priority("issue:1", &closed));
        let open = content_with(serde_json::json!({ "state": "open" }));
        assert!(!github_item_is_high_priority("issue:1", &open));
    }

    #[test]
    fn merged_pr_is_high_priority_even_when_open_state() {
        let merged = content_with(serde_json::json!({ "state": "open", "merged": true }));
        assert!(github_item_is_high_priority("pr:7", &merged));
        let unmerged = content_with(serde_json::json!({ "state": "open", "merged": false }));
        assert!(!github_item_is_high_priority("pr:7", &unmerged));
    }

    #[test]
    fn missing_metadata_defaults_to_low_priority() {
        let c = content_with(serde_json::json!({}));
        assert!(!github_item_is_high_priority("issue:9", &c));
    }
}
