//! Product compute and notification adapters for tinycortex sealing.

use anyhow::{Context, Result};
use async_trait::async_trait;
use chrono::Duration;

use crate::core::event_bus::{publish_global, DomainEvent};
use crate::openhuman::config::Config;
use crate::openhuman::memory_store::content::wiki_git::{SummaryCommitBatch, SummaryCommitEntry};
use crate::openhuman::memory_store::trees::types::{Buffer, SummaryNode, Tree};
use crate::openhuman::memory_tree::score::embed::{build_write_embedder, Embedder as HostEmbedder};
use crate::openhuman::memory_tree::tree::bucket_seal::LabelStrategy;

use super::{memory_config_from, HostSummariser};

struct EmbedderBridge<'a>(&'a dyn HostEmbedder);

#[async_trait]
impl tinycortex::memory::score::embed::Embedder for EmbedderBridge<'_> {
    fn name(&self) -> &'static str {
        self.0.name()
    }

    async fn embed(&self, text: &str) -> Result<Vec<f32>> {
        let vector = self.0.embed(text).await.map_err(|error| {
            let failure = crate::openhuman::memory_tree::health::classify_embed_error(&error);
            anyhow::Error::new(failure).context(format!("seal embedding failed: {error:#}"))
        })?;
        crate::openhuman::memory_tree::score::embed::pack_checked(&vector)
            .context("seal embedding dimension check")?;
        crate::openhuman::memory_tree::health::clear_semantic_recall_degraded();
        Ok(vector)
    }
}

struct Observer<'a> {
    config: &'a Config,
}

impl tinycortex::memory::tree::SealObserver for Observer<'_> {
    fn progress(&self, tree: &Tree, step: &str, level: u32, item_count: Option<u32>) {
        publish_global(DomainEvent::MemoryTreeBuildProgress {
            phase: "seal".to_string(),
            step: step.to_string(),
            tree_scope: Some(tree.scope.clone()),
            level: Some(level),
            item_count,
            detail: None,
        });
    }

    fn summary_committed(
        &self,
        tree: &Tree,
        node: &SummaryNode,
        content_path: &str,
        reason: &str,
    ) -> Result<()> {
        crate::openhuman::memory_store::content::wiki_git::commit_summaries(
            &self.config.memory_tree_content_root(),
            &SummaryCommitBatch {
                reason: reason.to_string(),
                tree_id: tree.id.clone(),
                tree_scope: tree.scope.clone(),
                entries: vec![SummaryCommitEntry {
                    summary_id: node.id.clone(),
                    content_path: content_path.to_string(),
                    level: node.level,
                    child_count: node.child_ids.len(),
                    token_count: node.token_count,
                    time_range_start: node.time_range_start,
                    time_range_end: node.time_range_end,
                }],
            },
        )
    }
}

pub async fn seal_one_level(
    config: &Config,
    tree: &Tree,
    buffer: &Buffer,
    strategy: &LabelStrategy,
    enqueue_follow_ups: bool,
) -> Result<String> {
    if let Err(error) = crate::openhuman::memory_store::content::obsidian::ensure_obsidian_defaults(
        &config.memory_tree_content_root(),
    ) {
        log::warn!("[tree::bucket_seal] obsidian defaults failed: {error:#}");
    }
    let host_embedder = build_write_embedder(config)?;
    let embedder_bridge = host_embedder.as_deref().map(EmbedderBridge);
    let summariser = HostSummariser::new(config.clone());
    let observer = Observer { config };
    let strategy = match strategy {
        LabelStrategy::ExtractFromContent(extractor) => {
            tinycortex::memory::tree::LabelStrategy::ExtractFromContent(extractor.clone())
        }
        LabelStrategy::UnionFromChildren => {
            tinycortex::memory::tree::LabelStrategy::UnionFromChildren
        }
        LabelStrategy::Empty => tinycortex::memory::tree::LabelStrategy::Empty,
    };
    tinycortex::memory::tree::seal_one_level_with_services(
        &memory_config_from(config, config.workspace_dir.clone()),
        tree,
        buffer,
        &tinycortex::memory::tree::SealServices {
            summariser: &summariser,
            embedder: embedder_bridge
                .as_ref()
                .map(|bridge| bridge as &dyn tinycortex::memory::score::embed::Embedder),
            observer: &observer,
        },
        &strategy,
        enqueue_follow_ups,
    )
    .await
}

pub async fn seal_document_subtree(
    config: &Config,
    tree: &Tree,
    doc_id: &str,
    version_ms: Option<i64>,
    chunk_ids: &[String],
    strategy: &LabelStrategy,
) -> Result<String> {
    if let Err(error) = crate::openhuman::memory_store::content::obsidian::ensure_obsidian_defaults(
        &config.memory_tree_content_root(),
    ) {
        log::warn!("[tree::bucket_seal] obsidian defaults failed: {error:#}");
    }
    let host_embedder = build_write_embedder(config)?;
    let embedder_bridge = host_embedder.as_deref().map(EmbedderBridge);
    let summariser = HostSummariser::new(config.clone());
    let observer = Observer { config };
    let strategy = match strategy {
        LabelStrategy::ExtractFromContent(extractor) => {
            tinycortex::memory::tree::LabelStrategy::ExtractFromContent(extractor.clone())
        }
        LabelStrategy::UnionFromChildren => {
            tinycortex::memory::tree::LabelStrategy::UnionFromChildren
        }
        LabelStrategy::Empty => tinycortex::memory::tree::LabelStrategy::Empty,
    };
    tinycortex::memory::tree::seal_document_subtree_with_services(
        &memory_config_from(config, config.workspace_dir.clone()),
        tree,
        doc_id,
        version_ms,
        chunk_ids,
        &tinycortex::memory::tree::SealServices {
            summariser: &summariser,
            embedder: embedder_bridge
                .as_ref()
                .map(|bridge| bridge as &dyn tinycortex::memory::score::embed::Embedder),
            observer: &observer,
        },
        &strategy,
    )
    .await
}

pub async fn cascade_tree(
    config: &Config,
    tree: &Tree,
    start_level: u32,
    force: bool,
    strategy: &LabelStrategy,
) -> Result<Vec<String>> {
    let host_embedder = build_write_embedder(config)?;
    let embedder_bridge = host_embedder.as_deref().map(EmbedderBridge);
    let summariser = HostSummariser::new(config.clone());
    let observer = Observer { config };
    let strategy = match strategy {
        LabelStrategy::ExtractFromContent(extractor) => {
            tinycortex::memory::tree::LabelStrategy::ExtractFromContent(extractor.clone())
        }
        LabelStrategy::UnionFromChildren => {
            tinycortex::memory::tree::LabelStrategy::UnionFromChildren
        }
        LabelStrategy::Empty => tinycortex::memory::tree::LabelStrategy::Empty,
    };
    tinycortex::memory::tree::cascade_all_from_with_services(
        &memory_config_from(config, config.workspace_dir.clone()),
        tree,
        start_level,
        force,
        &tinycortex::memory::tree::SealServices {
            summariser: &summariser,
            embedder: embedder_bridge
                .as_ref()
                .map(|bridge| bridge as &dyn tinycortex::memory::score::embed::Embedder),
            observer: &observer,
        },
        &strategy,
        false,
    )
    .await
}

pub async fn flush_stale_tree_buffers(
    config: &Config,
    max_age: Duration,
    strategy: &LabelStrategy,
) -> Result<usize> {
    let host_embedder = build_write_embedder(config)?;
    let embedder_bridge = host_embedder.as_deref().map(EmbedderBridge);
    let summariser = HostSummariser::new(config.clone());
    let observer = Observer { config };
    let strategy = match strategy {
        LabelStrategy::ExtractFromContent(extractor) => {
            tinycortex::memory::tree::LabelStrategy::ExtractFromContent(extractor.clone())
        }
        LabelStrategy::UnionFromChildren => {
            tinycortex::memory::tree::LabelStrategy::UnionFromChildren
        }
        LabelStrategy::Empty => tinycortex::memory::tree::LabelStrategy::Empty,
    };
    tinycortex::memory::tree::flush_stale_buffers_with_services(
        &memory_config_from(config, config.workspace_dir.clone()),
        max_age,
        &tinycortex::memory::tree::SealServices {
            summariser: &summariser,
            embedder: embedder_bridge
                .as_ref()
                .map(|bridge| bridge as &dyn tinycortex::memory::score::embed::Embedder),
            observer: &observer,
        },
        &strategy,
    )
    .await
}
