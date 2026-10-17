//! Wire types of the `openhuman.memory_*` RPC surface (see
//! `docs/specs/memory-v2.md`).
//!
//! Contract types (`Hit`, `Citation`, `MemoryMeta`, `MetaFilter`,
//! `EngineDescriptor`, …) are TinyMemory's own and serialise in its snake_case
//! field names; this module only adds the host's request and view shapes.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

pub use tinymemory_api::{
    Citation, EngineDescriptor, FetchMode, Hit, ItemKind, LearningKind, MemoryMeta, MetaFilter,
};

use crate::config::schema::MemorySourceKind;

/// Default `limit` of recall, fetch and list.
pub const DEFAULT_LIMIT: usize = 10;

/// Largest `limit` a caller may ask for.
pub const MAX_LIMIT: usize = 100;

/// Clamps a caller's `limit` into `1..=MAX_LIMIT`.
#[must_use]
pub fn clamp_limit(limit: Option<usize>) -> usize {
    limit.unwrap_or(DEFAULT_LIMIT).clamp(1, MAX_LIMIT)
}

/// A request with no parameters.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct EmptyParams {}

/// `memory_engines_list` result.
#[derive(Debug, Clone, Serialize)]
pub struct EnginesListView {
    /// Every engine this build can construct.
    pub engines: Vec<EngineDescriptor>,
    /// The engine serving now, `null` when memory is off.
    pub active: Option<String>,
}

/// Engine health as the UI shows it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EngineStatus {
    /// Serving.
    Ok,
    /// Serving, with a problem.
    Degraded,
    /// Configured but not answering.
    Down,
    /// No usable engine.
    Off,
}

/// `memory_engine_get` / `memory_engine_set` result.
#[derive(Debug, Clone, Serialize)]
pub struct EngineView {
    /// The configured engine id, `null` when none is selected.
    pub engine: Option<String>,
    /// The engine's endpoint, when known.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    /// Whether a credential is available to the engine.
    pub has_key: bool,
    /// Health.
    pub status: EngineStatus,
    /// Why the status is not `ok`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    /// Fetch modes the engine offers (empty when off).
    pub fetch_modes: Vec<FetchMode>,
}

/// `memory_engine_set` params.
#[derive(Debug, Clone, Deserialize)]
pub struct EngineSetParams {
    /// Engine id.
    pub engine: String,
    /// New endpoint; an empty string clears it.
    #[serde(default)]
    pub endpoint: Option<String>,
    /// New API key; an empty string removes the stored one.
    #[serde(default)]
    pub api_key: Option<String>,
}

/// `memory_recall` params.
#[derive(Debug, Clone, Deserialize)]
pub struct RecallParams {
    /// The question.
    pub question: String,
    /// Metadata filter.
    #[serde(default)]
    pub filter: Option<MetaFilter>,
    /// Most citations to return.
    #[serde(default)]
    pub limit: Option<usize>,
    /// The local days the question is about: memories from them rank
    /// first, nothing is dropped. Read in the user's time zone.
    #[serde(default)]
    pub refers_to: Option<RefersTo>,
}

/// Local calendar days (`YYYY-MM-DD`), inclusive; `to` defaults to `from`.
#[derive(Debug, Clone, Deserialize)]
pub struct RefersTo {
    /// First day.
    pub from: chrono::NaiveDate,
    /// Last day; the same as `from` for a single day.
    #[serde(default)]
    pub to: Option<chrono::NaiveDate>,
}

/// `memory_recall` result.
#[derive(Debug, Clone, Serialize)]
pub struct RecallView {
    /// The synthesised answer; in evidence mode
    /// ([`super::ops::recall_with`]), the recalled evidence as text.
    pub answer: String,
    /// The items it rests on.
    pub citations: Vec<Citation>,
    /// The model that answered, when the engine says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// `memory_fetch` params.
#[derive(Debug, Clone, Deserialize)]
pub struct FetchParams {
    /// The query.
    pub query: String,
    /// Retrieval mode; defaults to the engine's first declared mode.
    #[serde(default)]
    pub mode: Option<FetchMode>,
    /// Metadata filter.
    #[serde(default)]
    pub filter: Option<MetaFilter>,
    /// Page size.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Engine cursor from a previous page.
    #[serde(default)]
    pub cursor: Option<String>,
    /// The local days the query is about (see [`RecallParams::refers_to`]).
    #[serde(default)]
    pub refers_to: Option<RefersTo>,
}

/// `memory_fetch` result.
#[derive(Debug, Clone, Serialize)]
pub struct FetchView {
    /// Matches, best first.
    pub hits: Vec<Hit>,
    /// Cursor of the next page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// `memory_learn` params.
#[derive(Debug, Clone, Deserialize)]
pub struct LearnParams {
    /// What was learned.
    pub text: String,
    /// What kind of learning; defaults to `fact`.
    #[serde(default)]
    pub kind: Option<LearningKind>,
    /// Confidence in `0..=1`; defaults to `0.8`.
    #[serde(default)]
    pub confidence: Option<f32>,
    /// Caller metadata, merged under the host's.
    #[serde(default)]
    pub meta: Option<MemoryMeta>,
}

/// `memory_learn` result.
#[derive(Debug, Clone, Serialize)]
pub struct LearnView {
    /// The stored item's id.
    pub id: String,
}

/// `memory_forget` params.
#[derive(Debug, Clone, Deserialize)]
pub struct ForgetParams {
    /// Item ids.
    pub ids: Vec<String>,
    /// Only ids of items in this reach are forgotten; an id outside it is
    /// left alone as if it named nothing. The `memory` tool always sets the
    /// calling agent's reach.
    #[serde(default)]
    pub reach: Option<tinymemory_api::Reach>,
}

/// `memory_forget` result.
#[derive(Debug, Clone, Serialize)]
pub struct ForgetView {
    /// How many items were removed.
    pub forgotten: usize,
}

/// `memory_erase_all` params.
#[derive(Debug, Clone, Deserialize)]
pub struct EraseAllParams {
    /// Must be `true`: an interlock, because nothing erased comes back.
    #[serde(default)]
    pub confirm: bool,
}

/// `memory_erase_all` result.
#[derive(Debug, Clone, Serialize)]
pub struct EraseAllView {
    /// How many of the engine's scopes (kind stores) were erased.
    pub erased_scopes: usize,
}

/// `memory_items_list` params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ItemsListParams {
    /// Metadata filter.
    #[serde(default)]
    pub filter: Option<MetaFilter>,
    /// Page size.
    #[serde(default)]
    pub limit: Option<usize>,
    /// Engine cursor.
    #[serde(default)]
    pub cursor: Option<String>,
    /// Explorer path, applied on top of `filter` (see [`super::explore`]).
    #[serde(default)]
    pub path: Vec<super::explore::PathStep>,
    /// A view that shows a snippet of each item and reads one whole with
    /// `memory_items_get` when it is opened: a conversation's or chunked
    /// document's text may then be only its start, which an engine can
    /// list without assembling each item (`MemoryEngine::list_preview`).
    #[serde(default)]
    pub preview: bool,
}

/// `memory_items_list` result.
#[derive(Debug, Clone, Serialize)]
pub struct ItemsListView {
    /// Items, newest first, `score = 0`.
    pub items: Vec<Hit>,
    /// Cursor of the next page.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// A source's sync status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceStatus {
    /// Not syncing.
    #[default]
    Idle,
    /// A sync is running.
    Syncing,
    /// The last sync failed.
    Error,
}

/// One document source, as the UI sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SourceView {
    /// Stable id.
    pub id: String,
    /// What it reads.
    pub kind: MemorySourceKind,
    /// Path, URL, `owner/repo`, feed URL or Composio toolkit.
    pub target: String,
    /// Display label.
    pub label: String,
    /// Minutes between scheduled syncs.
    pub schedule_mins: Option<u32>,
    /// When the last sync finished.
    pub last_sync_at: Option<DateTime<Utc>>,
    /// Current status.
    pub status: SourceStatus,
    /// The last sync's failure.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Items stored by the last sync.
    pub items: u64,
    /// The memory node the source stores at (`root` for the shared one).
    pub namespace: String,
}

/// `memory_sources_list` result.
#[derive(Debug, Clone, Serialize)]
pub struct SourcesListView {
    /// Every configured source.
    pub sources: Vec<SourceView>,
}

/// `memory_sources_add` params.
#[derive(Debug, Clone, Deserialize)]
pub struct SourcesAddParams {
    /// What it reads.
    pub kind: String,
    /// Path, URL, `owner/repo`, feed URL or Composio toolkit.
    pub target: String,
    /// Display label; defaults to the target.
    #[serde(default)]
    pub label: Option<String>,
    /// Minutes between scheduled syncs.
    #[serde(default)]
    pub schedule_mins: Option<u32>,
    /// The memory node to store the source's documents at
    /// (`agent:researcher`); unset stores them at the root.
    #[serde(default)]
    pub namespace: Option<String>,
}

/// `memory_sources_add` result.
#[derive(Debug, Clone, Serialize)]
pub struct SourceAddedView {
    /// The new source.
    pub source: SourceView,
}

/// `memory_sources_remove` params.
#[derive(Debug, Clone, Deserialize)]
pub struct SourcesRemoveParams {
    /// Source id.
    pub id: String,
    /// Also forget everything the source stored.
    #[serde(default)]
    pub forget_items: Option<bool>,
}

/// `memory_sources_remove` result.
#[derive(Debug, Clone, Serialize)]
pub struct SourceRemovedView {
    /// Whether a source was removed.
    pub removed: bool,
}

/// `memory_sources_sync` params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SourcesSyncParams {
    /// One source; every source when omitted.
    #[serde(default)]
    pub id: Option<String>,
}

/// `memory_sources_sync` result.
#[derive(Debug, Clone, Serialize)]
pub struct SourcesSyncView {
    /// Ids of the sources whose sync started.
    pub started: Vec<String>,
}

/// Item counts a legacy store would import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportCounts {
    /// Documents, and every memory-tree chunk source.
    pub documents: u64,
    /// Conversations.
    pub conversations: u64,
    /// Learnings: distilled facts, profile facets, extracted events, turn
    /// lessons, graph relations, and the goals and persona files.
    pub learnings: u64,
}

/// `memory_import_scan` result.
#[derive(Debug, Clone, Serialize)]
pub struct ImportScanView {
    /// Whether a v1 store exists in this workspace.
    pub found: bool,
    /// What it holds.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<ImportCounts>,
}

/// `memory_import_start` params.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImportStartParams {
    /// Must be `true`: importing uploads local data to the engine.
    #[serde(default)]
    pub consent: bool,
}

/// Where an import stands, and its progress (`memory_import_*` state): the
/// module's own types, so the persisted file and the wire share one shape.
pub use tinymemory_integrations::import::{ImportPhase, ImportState};

/// `memory_import_start` / `_status` result.
#[derive(Debug, Clone, Serialize)]
pub struct ImportStateView {
    /// The state.
    pub state: ImportState,
}

/// A recall citation attached to a chat reply so the UI can show where a
/// memory-informed answer came from. Same JSON shape the chat surface has
/// always rendered: `key` is the item kind, `namespace` its source kind,
/// `timestamp` when it was observed, `snippet` a short excerpt.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TurnCitation {
    /// Item id.
    pub id: String,
    /// Item kind (`document`, `conversation`, `learning`).
    pub key: String,
    /// Source kind (`folder`, `agent`, …).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub namespace: Option<String>,
    /// Relevance, when the engine reports one.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub score: Option<f64>,
    /// When the item was observed (RFC 3339), or empty.
    pub timestamp: String,
    /// Short excerpt.
    pub snippet: String,
}

impl From<&Hit> for TurnCitation {
    fn from(hit: &Hit) -> Self {
        Self {
            id: hit.id.0.clone(),
            key: hit.kind.as_str().to_string(),
            namespace: Some(hit.meta.source.kind.as_str().to_string()),
            score: Some(f64::from(hit.score)).filter(|score| *score > 0.0),
            timestamp: hit
                .meta
                .observed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
            snippet: hit.text.chars().take(TURN_CITATION_SNIPPET_CHARS).collect(),
        }
    }
}

impl From<&Citation> for TurnCitation {
    fn from(citation: &Citation) -> Self {
        Self {
            id: citation.id.0.clone(),
            key: citation.kind.as_str().to_string(),
            namespace: Some(citation.meta.source.kind.as_str().to_string()),
            score: citation.score.map(f64::from),
            timestamp: citation
                .meta
                .observed_at
                .map(|at| at.to_rfc3339())
                .unwrap_or_default(),
            snippet: citation
                .snippet
                .chars()
                .take(TURN_CITATION_SNIPPET_CHARS)
                .collect(),
        }
    }
}

/// Longest snippet a [`TurnCitation`] carries.
pub const TURN_CITATION_SNIPPET_CHARS: usize = 280;
