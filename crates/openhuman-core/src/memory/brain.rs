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
use tinymemory_api::{Error, Hit, Namespace, StoreItem, WriteOptions};
use tinymemory_integrations::brain::MAX_INGEST_BYTES;
use tinymemory_integrations::documents::RawDocument;
use tinymemory_tools::layout::placement::{brain_node_with, files_source};
use tinymemory_tools::{Brain, BrainSource, BrainSourcesView, MemoryLayout};

use crate::config::schema::MemorySourceKind;
use crate::config::Config;

use super::engine;
use super::lifecycle::jobs;
use tinymemory_api::Result;

/// The brain source a synced item belongs to: the connector it came from,
/// so removing it erases one source. GitHub to `github`, links and feeds to
/// `web`, and local files, whatever their format, to `files`.
#[must_use]
pub fn brain_source(kind: MemorySourceKind) -> BrainSource {
    tinymemory_integrations::brain::brain_source(kind.source_kind())
}

/// The node `item` of `source` is filed at: the source's node, or, for
/// GitHub with `[memory] split_github_by_repo` on, its repository's
/// collection below it.
pub fn brain_node(
    config: &Config,
    layout: &MemoryLayout,
    source: &BrainSource,
    item: &StoreItem,
) -> Result<Namespace> {
    brain_node_with(config.memory.split_github_by_repo, layout, source, item)
}

/// The layout the brain RPCs act on: `identity`'s.
fn layout(
    config: &Config,
    identity: &tinymemory_tools::layout::identity::Identity,
) -> MemoryLayout {
    identity
        .resolve(&crate::memory::scope::identity_pins(config))
        .layout
}

async fn brain(
    config: &Config,
    identity: &tinymemory_tools::layout::identity::Identity,
) -> Result<Brain> {
    let bound = engine::resolve(config).await.engine()?;
    Ok(Brain::new(bound.engine, layout(config, identity)))
}

fn parse_source(raw: &str) -> Result<BrainSource> {
    tinymemory_integrations::brain::parse_source(raw).map_err(|error| match error {
        tinymemory_integrations::documents::Error::Invalid(message) => {
            Error::InvalidRequest(message)
        }
        error => Error::InvalidRequest(error.to_string()),
    })
}

/// `memory_brain_sources`: the brain's sources and their sizes.
pub async fn sources(
    config: &Config,
    identity: &tinymemory_tools::layout::identity::Identity,
) -> Result<BrainSourcesView> {
    brain(config, identity).await?.sources().await
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
    identity: &tinymemory_tools::layout::identity::Identity,
    params: BrainSearchParams,
) -> Result<BrainSearchView> {
    let query = params.query.trim();
    if query.is_empty() {
        return Err(Error::InvalidRequest("the query is blank".into()));
    }
    let source = params.source.as_deref().map(parse_source).transpose()?;
    let limit = super::types::clamp_limit(params.limit);
    let hits = brain(config, identity)
        .await?
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
/// (TinySecurity's protected-path floor: `~/.ssh`, `~/.aws`, `/etc`, ...) on the resolved
/// path (so a symlink cannot reach one either), and, with `[autonomy]`
/// enabled, workspace and trusted-root containment. A relative path lands in
/// `action_dir`.
///
/// [`SecurityPolicy::validate_path`]: crate::security::SecurityPolicy::validate_path
async fn ingest_path(config: &Config, path: &str) -> Result<std::path::PathBuf> {
    let policy = crate::security::SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    );
    policy.validate_path(path.trim()).await.map_err(|error| {
        tracing::warn!("[memory:brain] ingest path refused by the security policy");
        Error::InvalidRequest(format!("cannot read the file: {error}"))
    })
}

/// `memory_brain_ingest`: files a document in the brain and queues its
/// source's belief build. The write waits only for the engine to accept it.
pub async fn ingest(
    config: &Config,
    identity: &tinymemory_tools::layout::identity::Identity,
    params: BrainIngestParams,
) -> Result<BrainIngestView> {
    let source = params.source.as_deref().map(parse_source).transpose()?;
    let mut document = match (params.path.as_deref(), params.text.as_deref()) {
        (Some(path), None) => {
            let resolved = ingest_path(config, path).await?;
            let path = resolved.as_path();
            let size = std::fs::metadata(path)
                .map_err(|error| Error::InvalidRequest(format!("cannot read the file: {error}")))?
                .len();
            tinymemory_integrations::brain::check_ingest_size(size).map_err(
                |error| match error {
                    tinymemory_integrations::documents::Error::TooLarge { .. } => {
                        Error::InvalidRequest(format!(
                            "the file is {size} bytes; the limit is {MAX_INGEST_BYTES}"
                        ))
                    }
                    error => Error::InvalidRequest(error.to_string()),
                },
            )?;
            let bytes = tokio::fs::read(path)
                .await
                .map_err(|error| Error::InvalidRequest(format!("cannot read the file: {error}")))?;
            let mut raw = RawDocument::new(bytes);
            if let Some(name) = path.file_name().and_then(|name| name.to_str()) {
                raw = raw.with_filename(name);
            }
            let mut meta = tinymemory_api::MemoryMeta::default();
            meta.file_path = Some(path.display().to_string());
            tinymemory_integrations::brain::brain_document(
                &crate::modules::memory_converter::converter(config),
                &raw,
                Some(source.unwrap_or_else(files_source)),
                meta,
            )
            .await
            .map_err(|error| Error::InvalidRequest(format!("cannot convert the file: {error}")))?
        }
        (None, Some(text)) => {
            tinymemory_tools::BrainDocument::new(source.unwrap_or_else(files_source), text)
        }
        _ => {
            return Err(Error::InvalidRequest(
                ("pass exactly one of `path` or `text`").into(),
            ));
        }
    };
    if let Some(title) = params.title.filter(|title| !title.trim().is_empty()) {
        document = document.titled(title);
    }
    let filed = document.source.to_string();
    let ingested = brain(config, identity)
        .await?
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
    identity: &tinymemory_tools::layout::identity::Identity,
    params: BrainForgetParams,
) -> Result<BrainForgetView> {
    let source = parse_source(&params.source)?;
    let report = brain(config, identity).await?.forget(&source).await?;
    tracing::info!(source = %source, forgotten = report.forgotten, "[memory:brain] source forgotten");
    Ok(BrainForgetView {
        forgotten: report.forgotten,
    })
}

#[cfg(test)]
#[path = "brain_tests.rs"]
mod tests;
