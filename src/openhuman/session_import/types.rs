//! Serde types for the one-time session import into TinyAgents stores.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::openhuman::inference::provider::ChatMessage;

/// Schema version of the importer. Bump when the record shapes change and a
/// re-import should be forced.
pub const IMPORT_VERSION: u32 = 1;

/// Store namespaces / stream layout under `{workspace}/tinyagents_store/`.
///
/// TinyAgents store names are slash-free (ASCII alphanumerics plus `-_.`
/// only — the crate's path-traversal guard), so streams use dot-separated
/// names rather than the `thread/{id}/messages` shape sketched in the design
/// doc.
pub const KV_SUBDIR: &str = "kv";
pub const JOURNAL_SUBDIR: &str = "journal";
pub const NS_SESSIONS: &str = "sessions";
pub const NS_MIGRATIONS: &str = "migrations";
pub const NS_MIGRATION_ITEMS: &str = "migration_items";
pub const MARKER_KEY: &str = "session_import_v1";

/// Options accepted by `session_import.run`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ImportOptions {
    /// Plan only: read and report, write nothing.
    #[serde(default)]
    pub dry_run: bool,
    /// Optional glob over session stems (e.g. `1719*_orchestrator`); when
    /// set, only matching sources are considered and the global marker is
    /// neither honoured as a skip nor written.
    #[serde(default)]
    pub only: Option<String>,
    /// Re-import even when the global marker or an unchanged item ledger
    /// entry would normally skip the work.
    #[serde(default)]
    pub force: bool,
    /// Per-item info logging instead of debug.
    #[serde(default)]
    pub verbose: bool,
}

/// What kind of source a scanned item came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SourceKind {
    /// Current flat `session_raw/{stem}.jsonl`.
    Jsonl,
    /// Legacy date-folder `session_raw/{DDMMYYYY}/{stem}.jsonl`.
    JsonlLegacyDir,
    /// Markdown-only session (no JSONL twin anywhere).
    Markdown,
}

/// Per-item action taken (or planned, in dry-run).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ItemAction {
    Imported,
    WouldImport,
    SkippedUnchanged,
    Failed,
}

/// Per-source report line in the summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemReport {
    pub session_key: String,
    /// Source path relative to the workspace root.
    pub source: String,
    pub kind: SourceKind,
    pub action: ItemAction,
    /// Journal stream the messages went to (absent for failures).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stream: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<String>,
    pub messages: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
}

/// Whole-run summary returned by `session_import.run`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ImportSummary {
    pub dry_run: bool,
    /// True when the global marker short-circuited the scan.
    pub already_done: bool,
    pub scanned: usize,
    pub imported: usize,
    pub skipped: usize,
    pub failed: usize,
    pub messages_written: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<ItemReport>,
}

/// Usage roll-up carried on the session descriptor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescriptorUsage {
    pub input: u64,
    pub output: u64,
    pub cached_input: u64,
    pub cost_usd: f64,
}

/// Source pointers preserved on the descriptor (workspace-relative).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescriptorSource {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jsonl: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub md: Option<String>,
}

/// Import provenance block on the descriptor.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DescriptorImport {
    pub version: u32,
    pub imported_at: String,
    pub warnings: usize,
}

/// The `sessions/{session_key}` compatibility descriptor: maps the OpenHuman
/// session key to TinyAgents-side identifiers and back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub session_key: String,
    /// Parent session key derived from the `__` stem chain (`None` = root).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_session_key: Option<String>,
    /// From `_meta.thread_id`, or synthesized `imported-{session_key}`.
    pub thread_id: String,
    /// True when `thread_id` was synthesized because the source had none.
    #[serde(default)]
    pub thread_id_synthesized: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task_id: Option<String>,
    /// Run-ledger `agent_runs.id`s joined via `thread_id` (best-effort).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub run_ids: Vec<String>,
    /// Journal stream holding this session's messages.
    pub stream: String,
    pub dispatcher: String,
    pub agent_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub provider: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub created: String,
    pub updated: String,
    pub turn_count: usize,
    pub usage: DescriptorUsage,
    pub source: DescriptorSource,
    pub import: DescriptorImport,
}

/// One message journal entry.
///
/// `ChatMessage` marks `id`/`extra_metadata` `skip_serializing` (they must
/// not reach providers), so the journal carries its own full-fidelity record
/// of what `read_transcript()` returns — including the reconstructed
/// `openhuman_turn_usage` metadata.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JournalMessage {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub role: String,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub extra_metadata: Option<Value>,
}

impl From<&ChatMessage> for JournalMessage {
    fn from(msg: &ChatMessage) -> Self {
        Self {
            id: msg.id.clone(),
            role: msg.role.clone(),
            content: msg.content.clone(),
            extra_metadata: msg.extra_metadata.clone(),
        }
    }
}

impl From<JournalMessage> for ChatMessage {
    fn from(rec: JournalMessage) -> Self {
        ChatMessage {
            id: rec.id,
            role: rec.role,
            content: rec.content,
            extra_metadata: rec.extra_metadata,
        }
    }
}

/// Item-ledger record under `migration_items/{sha256(relative source path)}`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ItemLedgerRecord {
    pub version: u32,
    pub session_key: String,
    /// Workspace-relative source path.
    pub source: String,
    pub size: u64,
    pub mtime_ms: u64,
    pub stream: String,
    pub messages: usize,
    pub imported_at: String,
}
