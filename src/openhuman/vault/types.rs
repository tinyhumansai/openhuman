use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Status of a running or completed vault sync operation.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum VaultSyncStatus {
    #[default]
    Idle,
    Running,
    Completed,
    Failed,
}

/// Live progress for a vault sync operation.
///
/// Held in the global registry while a sync is in flight, and retained after
/// completion so the frontend can poll the final outcome without timing
/// concerns. The next `vault.sync` call for the same vault overwrites this.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VaultSyncState {
    pub vault_id: String,
    pub status: VaultSyncStatus,
    /// Files seen by the directory walker (updated after discovery phase).
    pub scanned: u64,
    /// Files successfully ingested so far.
    pub ingested: u64,
    /// Files skipped because hash+mtime was unchanged.
    pub unchanged: u64,
    /// Files removed from the vault (source file gone).
    pub removed: u64,
    /// Files that failed ingestion.
    pub failed: u64,
    /// Files skipped (unsupported extension or too large).
    pub skipped_unsupported: u64,
    /// Total files queued for ingestion (set after discovery; 0 while walking).
    pub total: u64,
    /// Unix milliseconds when this sync started.
    pub started_at_ms: i64,
    /// Unix milliseconds when this sync finished; `None` while still running.
    pub finished_at_ms: Option<i64>,
    /// Wall-clock duration in ms; 0 while running; set from VaultSyncReport on completion.
    pub duration_ms: i64,
    /// Accumulated error strings.
    pub errors: Vec<String>,
}

/// A user-registered local folder whose files are mirrored into memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vault {
    pub id: String,
    pub name: String,
    pub root_path: String,
    #[serde(default)]
    pub host_os: Option<String>,
    pub namespace: String,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub file_count: u64,
    #[serde(default)]
    pub write_state: VaultWriteState,
    #[serde(default)]
    pub write_state_reason: Option<String>,
}

/// Whether OpenHuman can write approved markdown artifacts back into a vault.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum VaultWriteState {
    Writable,
    ReadOnly,
    #[default]
    Unavailable,
}

/// Per-file ledger entry used for dedup on re-sync.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultFile {
    pub vault_id: String,
    pub rel_path: String,
    pub document_id: String,
    pub content_hash: String,
    pub mtime_ms: i64,
    pub bytes: u64,
    pub ingested_at: DateTime<Utc>,
    pub status: VaultFileStatus,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum VaultFileStatus {
    Ok,
    Skipped,
    Failed,
}

impl VaultFileStatus {
    pub(crate) fn as_str(&self) -> &'static str {
        match self {
            Self::Ok => "ok",
            Self::Skipped => "skipped",
            Self::Failed => "failed",
        }
    }

    pub(crate) fn parse(raw: &str) -> Self {
        match raw {
            "skipped" => Self::Skipped,
            "failed" => Self::Failed,
            _ => Self::Ok,
        }
    }
}

/// Summary returned from `vault.sync`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct VaultSyncReport {
    pub vault_id: String,
    pub scanned: u64,
    pub ingested: u64,
    pub unchanged: u64,
    pub removed: u64,
    pub failed: u64,
    pub skipped_unsupported: u64,
    pub duration_ms: i64,
    pub errors: Vec<String>,
}

/// Result returned after writing an approved markdown/wiki artifact into a vault.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct VaultWriteMarkdownReport {
    pub vault_id: String,
    pub rel_path: String,
    pub bytes_written: u64,
    pub created: bool,
}
