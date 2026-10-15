//! The brain: documents every agent shares, filed by the connector they came
//! from.
//!
//! TinyMemory's layout keeps documents at `source:<connector>` nodes below
//! the layout root (`tinymemory_tools::MemoryLayout::brain`), with no agent
//! id. Synced sources ([`super::sources`]) are filed there by what they read
//! ([`brain_source`]); a file ingested from the UI under `files`. Every ingest queues a
//! belief build of the source's scope (`lifecycle::jobs`).
//!
//! The `memory_brain_*` RPCs read and forget per source, under the root of
//! the identity in scope (the default root outside an agent).

use serde::{Deserialize, Serialize};
use tinymemory_api::{ExploreRequest, Facet, Hit, Namespace, StoreItem, WriteOptions};
use tinymemory_integrations::documents::RawDocument;
use tinymemory_tools::layout::placement::{brain_node_with, files_source};
use tinymemory_tools::{Brain, BrainSource, MemoryLayout};

use crate::config::schema::MemorySourceKind;
use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::lifecycle::jobs;
use super::scope::MemoryIdentity;

/// Largest file `memory_brain_ingest` reads, in bytes.
pub const MAX_INGEST_BYTES: u64 = 25 * 1024 * 1024;

/// The brain source a synced item belongs to: the connector it came from,
/// so removing it erases one source. GitHub to `github`, links and feeds to
/// `web`, and local files, whatever their format, to `files`.
#[must_use]
pub fn brain_source(kind: MemorySourceKind) -> BrainSource {
    match kind {
        MemorySourceKind::Github => BrainSource::Github,
        MemorySourceKind::Link | MemorySourceKind::Rss => BrainSource::Web,
        MemorySourceKind::Folder | MemorySourceKind::File => files_source(),
    }
}

/// The node `item` of `source` is filed at: the source's node, or, for
/// GitHub with `[memory] split_github_by_repo` on, its repository's
/// collection below it.
pub fn brain_node(
    config: &Config,
    layout: &MemoryLayout,
    source: &BrainSource,
    item: &StoreItem,
) -> MemoryResult<Namespace> {
    Ok(brain_node_with(
        config.memory.split_github_by_repo,
        layout,
        source,
        item,
    )?)
}

/// The layout the brain RPCs act on: `identity`'s.
fn layout(config: &Config, identity: &MemoryIdentity) -> MemoryLayout {
    identity.resolve(config).layout
}

fn brain(config: &Config, identity: &MemoryIdentity) -> MemoryResult<Brain> {
    let bound = engine::resolve(config).engine()?;
    Ok(Brain::new(bound.engine, layout(config, identity)))
}

fn parse_source(raw: &str) -> MemoryResult<BrainSource> {
    raw.parse()
        .map_err(|error: tinymemory_api::Error| MemoryError::invalid(error.to_string()))
}

/// One brain source and how many documents it holds.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrainSourceCount {
    /// The source id (`pdf`, `notion`, …).
    pub source: String,
    /// Documents stored under it.
    pub documents: u64,
}

/// `memory_brain_sources` result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct BrainSourcesView {
    /// The layout root the brain lives under.
    pub root: String,
    /// Each source with documents, most first.
    pub sources: Vec<BrainSourceCount>,
    /// Documents outside any source node (stored before the brain layout).
    pub unfiled: u64,
}

/// `memory_brain_sources`: the brain's sources and their sizes.
pub async fn sources(config: &Config, identity: &MemoryIdentity) -> MemoryResult<BrainSourcesView> {
    let bound = engine::resolve(config).engine()?;
    let layout = layout(config, identity);
    let page = bound
        .engine
        .explore(ExploreRequest {
            facet: Facet::Namespace,
            filter: layout.brain_filter(None),
            limit: 200,
            scan_limit: 20_000,
        })
        .await?;
    // A source's collections (`source:github/project:…`) count as the
    // source's own documents.
    let source_of = |node: &str| -> Option<String> {
        let node: Namespace = node.parse().ok()?;
        let segment = node.segments().get(layout.root().depth())?;
        (segment.kind() == tinymemory_api::SegmentKind::Source).then(|| segment.id().to_string())
    };
    let mut counts = std::collections::BTreeMap::<String, u64>::new();
    let mut unfiled = 0;
    for bucket in page.buckets {
        match source_of(&bucket.value) {
            Some(source) => *counts.entry(source).or_default() += bucket.count,
            None => unfiled += bucket.count,
        }
    }
    let mut sources: Vec<BrainSourceCount> = counts
        .into_iter()
        .map(|(source, documents)| BrainSourceCount { source, documents })
        .collect();
    sources.sort_by(|a, b| b.documents.cmp(&a.documents).then(a.source.cmp(&b.source)));
    Ok(BrainSourcesView {
        root: layout.root().to_string(),
        sources,
        unfiled: unfiled + page.missing,
    })
}

/// `memory_brain_search` params.
#[derive(Debug, Clone, Deserialize)]
pub struct BrainSearchParams {
    /// What to look for.
    pub query: String,
    /// One source's documents only.
    #[serde(default)]
    pub source: Option<String>,
    /// Most hits (default 10).
    #[serde(default)]
    pub limit: Option<usize>,
}

/// `memory_brain_search` result.
#[derive(Debug, Clone, Serialize)]
pub struct BrainSearchView {
    /// Matching documents, best first.
    pub hits: Vec<Hit>,
}

/// `memory_brain_search`.
pub async fn search(
    config: &Config,
    identity: &MemoryIdentity,
    params: BrainSearchParams,
) -> MemoryResult<BrainSearchView> {
    let query = params.query.trim();
    if query.is_empty() {
        return Err(MemoryError::invalid("the query is blank"));
    }
    let source = params.source.as_deref().map(parse_source).transpose()?;
    let limit = super::types::clamp_limit(params.limit);
    let hits = brain(config, identity)?
        .search(query, source.as_ref(), limit)
        .await?;
    tracing::debug!(hits = hits.len(), "[memory:brain] search");
    Ok(BrainSearchView { hits })
}

/// `memory_brain_ingest` params: a file on disk, or text.
#[derive(Debug, Clone, Deserialize)]
pub struct BrainIngestParams {
    /// A file to read and convert.
    #[serde(default)]
    pub path: Option<String>,
    /// Text to file directly.
    #[serde(default)]
    pub text: Option<String>,
    /// The source to file under; unset files it under `files`.
    #[serde(default)]
    pub source: Option<String>,
    /// A title.
    #[serde(default)]
    pub title: Option<String>,
}

/// `memory_brain_ingest` result.
#[derive(Debug, Clone, Serialize)]
pub struct BrainIngestView {
    /// The stored document's id.
    pub id: String,
    /// The source it was filed under.
    pub source: String,
    /// Whether it was already stored.
    pub replayed: bool,
}

/// Resolves an ingest `path` through the security policy, the same check
/// the file tools make ([`SecurityPolicy::validate_path`]): no null bytes or
/// `..` traversal, the credential-store and system-root floor
/// (`is_always_forbidden`: `~/.ssh`, `~/.aws`, `/etc`, ...) on the resolved
/// path (so a symlink cannot reach one either), and, with `[autonomy]`
/// enabled, workspace and trusted-root containment. A relative path lands in
/// `action_dir`.
///
/// [`SecurityPolicy::validate_path`]: crate::security::SecurityPolicy::validate_path
async fn ingest_path(config: &Config, path: &str) -> MemoryResult<std::path::PathBuf> {
    let policy = crate::security::SecurityPolicy::from_full_config(config);
    policy.validate_path(path.trim()).await.map_err(|error| {
        tracing::warn!("[memory:brain] ingest path refused by the security policy");
        MemoryError::invalid(format!("cannot read the file: {error}"))
    })
}

/// `memory_brain_ingest`: files a document in the brain and queues its
/// source's belief build. The write waits only for the engine to accept it.
pub async fn ingest(
    config: &Config,
    identity: &MemoryIdentity,
    params: BrainIngestParams,
) -> MemoryResult<BrainIngestView> {
    let source = params.source.as_deref().map(parse_source).transpose()?;
    let mut document = match (params.path.as_deref(), params.text.as_deref()) {
        (Some(path), None) => {
            let resolved = ingest_path(config, path).await?;
            let path = resolved.as_path();
            let size = std::fs::metadata(path)
                .map_err(|error| MemoryError::invalid(format!("cannot read the file: {error}")))?
                .len();
            if size > MAX_INGEST_BYTES {
                return Err(MemoryError::invalid(format!(
                    "the file is {size} bytes; the limit is {MAX_INGEST_BYTES}"
                )));
            }
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|error| MemoryError::invalid(format!("cannot read the file: {error}")))?;
            let mut raw = RawDocument::new(bytes);
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                raw = raw.with_filename(name);
            }
            let mut meta = tinymemory_api::MemoryMeta::default();
            meta.file_path = Some(path.display().to_string());
            tinymemory_integrations::brain::brain_document(
                &super::convert::converter(config),
                &raw,
                Some(source.unwrap_or_else(files_source)),
                meta,
            )
            .await
            .map_err(|error| MemoryError::invalid(format!("cannot convert the file: {error}")))?
        }
        (None, Some(text)) => {
            tinymemory_tools::BrainDocument::new(source.unwrap_or_else(files_source), text)
        }
        _ => return Err(MemoryError::invalid("pass exactly one of `path` or `text`")),
    };
    if let Some(title) = params.title.filter(|title| !title.trim().is_empty()) {
        document = document.titled(title);
    }
    let filed = document.source.to_string();
    let ingested = brain(config, identity)?
        .ingest_with(document, WriteOptions::accepted())
        .await?;
    jobs::enqueue(
        config,
        layout(config, identity).root(),
        ingested.job.into_iter().collect(),
    )
    .await;
    tracing::debug!(source = %filed, replayed = ingested.receipt.replayed, "[memory:brain] ingested");
    Ok(BrainIngestView {
        id: ingested.receipt.id.to_string(),
        source: filed,
        replayed: ingested.receipt.replayed,
    })
}

/// `memory_brain_forget` params.
#[derive(Debug, Clone, Deserialize)]
pub struct BrainForgetParams {
    /// The source whose documents are forgotten.
    pub source: String,
}

/// `memory_brain_forget` result.
#[derive(Debug, Clone, Serialize)]
pub struct BrainForgetView {
    /// Documents forgotten.
    pub forgotten: usize,
}

/// `memory_brain_forget`: forgets every document of one source.
pub async fn forget(
    config: &Config,
    identity: &MemoryIdentity,
    params: BrainForgetParams,
) -> MemoryResult<BrainForgetView> {
    let source = parse_source(&params.source)?;
    let report = brain(config, identity)?.forget(&source).await?;
    tracing::info!(source = %source, forgotten = report.forgotten, "[memory:brain] source forgotten");
    Ok(BrainForgetView {
        forgotten: report.forgotten,
    })
}

#[cfg(test)]
#[path = "brain_tests.rs"]
mod tests;
