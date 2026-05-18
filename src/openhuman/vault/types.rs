use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A user-registered local folder whose files are mirrored into memory.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Vault {
    pub id: String,
    pub name: String,
    pub root_path: String,
    pub namespace: String,
    pub include_globs: Vec<String>,
    pub exclude_globs: Vec<String>,
    pub created_at: DateTime<Utc>,
    pub last_synced_at: Option<DateTime<Utc>>,
    pub file_count: u64,
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
