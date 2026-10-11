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
use tinymemory_tools::{Brain, BrainSource, MemoryLayout};

use crate::config::schema::MemorySourceKind;
use crate::config::Config;

use super::engine;
use super::error::{MemoryError, MemoryResult};
use super::lifecycle::jobs;
use super::scope;

/// Largest file `memory_brain_ingest` reads, in bytes.
pub const MAX_INGEST_BYTES: u64 = 25 * 1024 * 1024;

/// The brain source of local files and uploads, of every format.
// TODO(tinymemory#214): `BrainSource::Files` once the pin carries it; the
// node (`source:files`) is the same.
#[must_use]
pub fn files_source() -> BrainSource {
    BrainSource::Other("files".to_string())
}

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

/// Canonical form of a connector slug found on a legacy brain node
/// (`google_drive` becomes `googledrive`), so documents filed by the removed
/// Composio sync still migrate to the node they were filed under.
fn legacy_connector_slug(slug: &str) -> String {
    let key = slug.trim().to_ascii_lowercase();
    match key.as_str() {
        "feishu" | "lark" => "larksuite".to_string(),
        "google_calendar" => "googlecalendar".to_string(),
        "google_drive" => "googledrive".to_string(),
        "google_sheets" => "googlesheets".to_string(),
        _ => key,
    }
}

/// The brain source a document filed under the old per-type layout belongs
/// to now, for the migration that moves it: `old_source_id` is the id of
/// the `source:<id>` node it sits at, `item` the document itself.
///
/// - Per-format nodes (`pdf`, `markdown`, `docx`, `xlsx`, `pptx`, `code`,
///   `other`) held local files: `files`.
/// - `web` held links and feeds, but also HTML files from a folder or an
///   upload; a document with a file kind or a file path is a file.
/// - Any other node was a connector: its canonical slug (`google_drive`
///   becomes `googledrive`), so `notion`, `github` and `gmail` stay put.
#[must_use]
pub fn legacy_brain_node(old_source_id: &str, item: &StoreItem) -> BrainSource {
    let id = legacy_connector_slug(old_source_id);
    match id.as_str() {
        "files" | "pdf" | "markdown" | "md" | "docx" | "xlsx" | "pptx" | "code" | "other" => {
            files_source()
        }
        "web" => {
            let meta = item.meta();
            let file = matches!(
                meta.source.kind,
                tinymemory_api::SourceKind::File | tinymemory_api::SourceKind::Folder
            ) || meta.file_path.is_some();
            if file {
                files_source()
            } else {
                BrainSource::Web
            }
        }
        _ => id.parse().unwrap_or_else(|_| files_source()),
    }
}

/// The repository a GitHub document belongs to, as the collection id
/// `<owner>--<repo>` (lowercase, as GitHub's names are case-insensitive):
/// from its `repo` (`owner/name` or a URL), else its URL. An owner name
/// never holds `--` nor ends in `-`, so the first `--` always splits the
/// two and no two repositories share an id (`foo-bar/repo` and
/// `foo/bar-repo` stay apart).
#[must_use]
pub fn github_collection(item: &StoreItem) -> Option<String> {
    let meta = item.meta();
    [meta.repo.as_deref(), meta.url.as_deref()]
        .into_iter()
        .flatten()
        .find_map(|raw| {
            let path = raw
                .trim()
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .trim_start_matches("www.")
                .trim_start_matches("github.com/");
            // A URL's query or fragment is not part of the path.
            let path = path.split(['?', '#']).next().unwrap_or_default();
            let mut parts = path.split('/').filter(|part| !part.is_empty());
            let (owner, repo) = (parts.next()?, parts.next()?);
            let repo = repo.trim_end_matches(".git");
            (!owner.contains('.') && !repo.is_empty())
                .then(|| format!("{owner}--{repo}").to_ascii_lowercase())
        })
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
    brain_node_with(config.memory.split_github_by_repo, layout, source, item)
}

/// [`brain_node`] with `[memory] split_github_by_repo` given, for a caller
/// that holds the setting rather than the config (the layout migration).
pub fn brain_node_with(
    split_github_by_repo: bool,
    layout: &MemoryLayout,
    source: &BrainSource,
    item: &StoreItem,
) -> MemoryResult<Namespace> {
    if split_github_by_repo && *source == BrainSource::Github {
        if let Some(repo) = github_collection(item) {
            return Ok(layout.brain_collection(source, &repo)?);
        }
    }
    Ok(layout.brain(source)?)
}

/// `item` placed in the brain at `node`, with no agent id (the brain
/// belongs to every agent).
#[must_use]
pub fn file_into(node: Namespace, mut item: StoreItem) -> StoreItem {
    let meta = item.meta_mut();
    meta.namespace = node;
    meta.agent_id = None;
    item
}

/// The layout the brain RPCs act on: the in-scope identity's.
fn layout(config: &Config) -> MemoryLayout {
    scope::resolve_current(config).layout
}

fn brain(config: &Config) -> MemoryResult<Brain> {
    let bound = engine::resolve(config).engine()?;
    Ok(Brain::new(bound.engine, layout(config)))
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
pub async fn sources(config: &Config) -> MemoryResult<BrainSourcesView> {
    let bound = engine::resolve(config).engine()?;
    let layout = layout(config);
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
pub async fn search(config: &Config, params: BrainSearchParams) -> MemoryResult<BrainSearchView> {
    let query = params.query.trim();
    if query.is_empty() {
        return Err(MemoryError::invalid("the query is blank"));
    }
    let source = params.source.as_deref().map(parse_source).transpose()?;
    let limit = super::types::clamp_limit(params.limit);
    let hits = brain(config)?.search(query, source.as_ref(), limit).await?;
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
    let policy = crate::security::SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
        &config.action_dir,
    );
    policy.validate_path(path.trim()).await.map_err(|error| {
        tracing::warn!("[memory:brain] ingest path refused by the security policy");
        MemoryError::invalid(format!("cannot read the file: {error}"))
    })
}

/// `memory_brain_ingest`: files a document in the brain and queues its
/// source's belief build. The write waits only for the engine to accept it.
pub async fn ingest(config: &Config, params: BrainIngestParams) -> MemoryResult<BrainIngestView> {
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
    let ingested = brain(config)?
        .ingest_with(document, WriteOptions::accepted())
        .await?;
    jobs::enqueue(
        config,
        layout(config).root(),
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
pub async fn forget(config: &Config, params: BrainForgetParams) -> MemoryResult<BrainForgetView> {
    let source = parse_source(&params.source)?;
    let report = brain(config)?.forget(&source).await?;
    tracing::info!(source = %source, forgotten = report.forgotten, "[memory:brain] source forgotten");
    Ok(BrainForgetView {
        forgotten: report.forgotten,
    })
}

#[cfg(test)]
#[path = "brain_tests.rs"]
mod tests;
