//! Sync audit log — append-only JSONL recording each sync run's token
//! usage and cost.
//!
//! Written to `<workspace>/memory_tree/sync_audit.jsonl`. Each line is a
//! self-contained JSON object describing one completed sync run.

use chrono::{DateTime, Utc};
use serde::Serialize;
use std::io::Write;
use std::path::Path;

use crate::openhuman::config::Config;

#[derive(Clone, Debug, Serialize, serde::Deserialize)]
pub struct SyncAuditEntry {
    pub timestamp: DateTime<Utc>,
    pub source_id: String,
    pub source_kind: String,
    pub scope: String,
    /// Total items fetched from the source (commits, issues, PRs, etc.).
    pub items_fetched: u32,
    /// Number of summarise batches produced.
    pub batches: u32,
    /// Estimated input tokens fed to the summariser (sum of item bodies / 4).
    pub input_tokens: u64,
    /// Output tokens produced by the summariser.
    pub output_tokens: u64,
    /// Estimated cost in USD (input + output at model pricing).
    pub estimated_cost_usd: f64,
    /// Number of Composio billable API actions executed during this sync
    /// (e.g. `GMAIL_FETCH_EMAILS`, `SLACK_LIST_CONVERSATIONS`). `0` for
    /// non-Composio source kinds. `#[serde(default)]` keeps audit lines
    /// written before #3111 parseable.
    #[serde(default)]
    pub composio_actions_called: u32,
    /// Actual USD charged for those Composio actions, summed from each
    /// response's backend-reported `cost_usd`. `0.0` for non-Composio kinds
    /// or when the backend reports no charge (e.g. direct mode).
    #[serde(default)]
    pub composio_cost_usd: f64,
    /// Duration of the sync in milliseconds.
    pub duration_ms: u64,
    /// Whether the sync completed successfully.
    pub success: bool,
    /// Error message if the sync failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl SyncAuditEntry {
    /// Total cost of the run: LLM summarisation cost plus the actual
    /// Composio API-action cost. This is the "combined cost" the Sync
    /// History UI surfaces so users see the full expense of a sync in one
    /// number rather than just the summarisation slice (#3111).
    pub fn combined_cost_usd(&self) -> f64 {
        self.estimated_cost_usd + self.composio_cost_usd
    }
}

const AUDIT_FILENAME: &str = "sync_audit.jsonl";

/// Append an audit entry to the sync audit log.
pub fn append_audit_entry(config: &Config, entry: &SyncAuditEntry) {
    let dir = config.workspace_dir.join("memory_tree");
    if let Err(e) = std::fs::create_dir_all(&dir) {
        tracing::warn!(
            error = %e,
            "[memory_sync:audit] failed to create audit dir"
        );
        return;
    }

    let path = dir.join(AUDIT_FILENAME);
    if let Err(e) = append_jsonl(&path, entry) {
        tracing::warn!(
            error = %e,
            "[memory_sync:audit] failed to write audit entry"
        );
    }
}

fn append_jsonl(path: &Path, entry: &SyncAuditEntry) -> std::io::Result<()> {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    let json = serde_json::to_string(entry)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    writeln!(file, "{json}")?;
    Ok(())
}

/// Read all audit entries, most recent first. Returns an empty vec if
/// the file doesn't exist yet.
pub fn read_audit_log(config: &Config) -> Vec<SyncAuditEntry> {
    let path = config
        .workspace_dir
        .join("memory_tree")
        .join(AUDIT_FILENAME);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };
    let mut entries: Vec<SyncAuditEntry> = content
        .lines()
        .filter(|l| !l.trim().is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    entries.reverse();
    entries
}

/// Estimate cost in USD for a given token count.
///
/// Uses DeepSeek v4 flash pricing (the summarization-v1 backing model):
/// $0.07/M input, $0.28/M output.
pub fn estimate_cost_usd(input_tokens: u64, output_tokens: u64) -> f64 {
    let input_cost = input_tokens as f64 * 0.07 / 1_000_000.0;
    let output_cost = output_tokens as f64 * 0.28 / 1_000_000.0;
    input_cost + output_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_reasonable() {
        // 50k input + 5k output at DeepSeek flash pricing
        let cost = estimate_cost_usd(50_000, 5_000);
        // $0.0035 input + $0.0014 output = $0.0049
        assert!((cost - 0.0049).abs() < 0.0001);
    }

    #[test]
    fn append_creates_file_and_writes_jsonl() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("test_audit.jsonl");

        let entry = SyncAuditEntry {
            timestamp: Utc::now(),
            source_id: "src_123".to_string(),
            source_kind: "github_repo".to_string(),
            scope: "github:org/repo".to_string(),
            items_fetched: 100,
            batches: 2,
            input_tokens: 50_000,
            output_tokens: 5_000,
            estimated_cost_usd: 0.225,
            composio_actions_called: 0,
            composio_cost_usd: 0.0,
            duration_ms: 12_000,
            success: true,
            error: None,
        };

        append_jsonl(&path, &entry).unwrap();
        append_jsonl(&path, &entry).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].contains("src_123"));
    }

    /// `combined_cost_usd` sums the LLM summarisation cost and the Composio
    /// API-action cost so the Sync History UI can show one total (#3111).
    #[test]
    fn combined_cost_sums_llm_and_composio() {
        let entry = SyncAuditEntry {
            timestamp: Utc::now(),
            source_id: "src_gmail".to_string(),
            source_kind: "composio".to_string(),
            scope: "gmail".to_string(),
            items_fetched: 40,
            batches: 1,
            input_tokens: 20_000,
            output_tokens: 2_000,
            estimated_cost_usd: 0.001_96,
            composio_actions_called: 8,
            composio_cost_usd: 0.04,
            duration_ms: 5_000,
            success: true,
            error: None,
        };
        // 0.00196 (LLM) + 0.04 (Composio) = 0.04196
        assert!((entry.combined_cost_usd() - 0.041_96).abs() < 1e-9);
    }

    /// Audit lines written before #3111 (no composio_* fields) must still
    /// deserialize — the new fields are `#[serde(default)]`. Pins the
    /// back-compat contract for the append-only JSONL log.
    #[test]
    fn legacy_audit_line_without_composio_fields_deserializes() {
        let legacy = r#"{"timestamp":"2026-05-01T00:00:00Z","source_id":"src_old","source_kind":"github_repo","scope":"github:org/repo","items_fetched":10,"batches":1,"input_tokens":1000,"output_tokens":100,"estimated_cost_usd":0.0001,"duration_ms":2000,"success":true}"#;
        let entry: SyncAuditEntry =
            serde_json::from_str(legacy).expect("legacy audit line must still parse");
        assert_eq!(entry.composio_actions_called, 0);
        assert_eq!(entry.composio_cost_usd, 0.0);
        assert_eq!(entry.source_id, "src_old");
    }
}
