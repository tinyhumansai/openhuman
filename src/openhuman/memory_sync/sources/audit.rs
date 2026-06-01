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
    /// Input tokens fed to the summariser. Provider-reported when the
    /// backend returned usage for every batch; otherwise estimated as the
    /// sum of item bodies / 4. See [`SyncAuditEntry::actual_charged_usd`]
    /// for the signal of which path produced the cost figure.
    pub input_tokens: u64,
    /// Output tokens produced by the summariser. Provider-reported when
    /// available, else estimated.
    pub output_tokens: u64,
    /// Estimated cost in USD (input + output at hardcoded model pricing).
    /// Always populated as a fallback so old audit entries — and runs
    /// where the backend reported no charge — still render a cost. Prefer
    /// [`SyncAuditEntry::actual_charged_usd`] when it is `Some`.
    pub estimated_cost_usd: f64,
    /// Real amount billed by the backend in USD (sum of
    /// `openhuman.billing.charged_amount_usd` across batches), when the
    /// provider reported it for the run. `None` for runs that fell back to
    /// the local estimate and for audit entries written before issue
    /// #3110 (the `#[serde(default)]` keeps those deserialising). When
    /// `Some`, this is the authoritative cost; renderers should show it in
    /// preference to `estimated_cost_usd`.
    #[serde(default)]
    pub actual_charged_usd: Option<f64>,
    /// Duration of the sync in milliseconds.
    pub duration_ms: u64,
    /// Whether the sync completed successfully.
    pub success: bool,
    /// Error message if the sync failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

impl SyncAuditEntry {
    /// The cost figure to display for this run: the real backend charge
    /// when the provider reported one ([`Self::actual_charged_usd`]),
    /// otherwise the hardcoded-pricing estimate ([`Self::estimated_cost_usd`]).
    ///
    /// Old audit entries (written before issue #3110) have no
    /// `actual_charged_usd`, so they transparently fall back to the
    /// estimate and keep rendering as before.
    pub fn effective_cost_usd(&self) -> f64 {
        self.actual_charged_usd.unwrap_or(self.estimated_cost_usd)
    }

    /// True when the cost figure came from the backend's real billing
    /// signal rather than the hardcoded-pricing estimate.
    pub fn cost_is_actual(&self) -> bool {
        self.actual_charged_usd.is_some()
    }
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
            actual_charged_usd: None,
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

    fn entry_with_costs(estimated: f64, actual: Option<f64>) -> SyncAuditEntry {
        SyncAuditEntry {
            timestamp: Utc::now(),
            source_id: "src".to_string(),
            source_kind: "github_repo".to_string(),
            scope: "github:org/repo".to_string(),
            items_fetched: 1,
            batches: 1,
            input_tokens: 100,
            output_tokens: 10,
            estimated_cost_usd: estimated,
            actual_charged_usd: actual,
            duration_ms: 1,
            success: true,
            error: None,
        }
    }

    #[test]
    fn effective_cost_prefers_actual_charge_when_present() {
        // An entry built from a provider `UsageInfo` carries the real
        // backend charge — `effective_cost_usd` must return it, not the
        // hardcoded-pricing estimate.
        let entry = entry_with_costs(0.0049, Some(0.0123));
        assert!(entry.cost_is_actual());
        assert!((entry.effective_cost_usd() - 0.0123).abs() < f64::EPSILON);
    }

    #[test]
    fn effective_cost_falls_back_to_estimate_without_usage() {
        // No provider usage → fall back to the estimate.
        let entry = entry_with_costs(0.0049, None);
        assert!(!entry.cost_is_actual());
        assert!((entry.effective_cost_usd() - 0.0049).abs() < f64::EPSILON);
    }

    #[test]
    fn old_entry_without_actual_field_deserializes_and_renders_estimate() {
        // A pre-#3110 audit line has no `actual_charged_usd` key. The
        // `#[serde(default)]` must let it deserialize, and the entry must
        // render its estimate via `effective_cost_usd`.
        let legacy = r#"{
            "timestamp": "2024-01-01T00:00:00Z",
            "source_id": "src_old",
            "source_kind": "github_repo",
            "scope": "github:org/repo",
            "items_fetched": 10,
            "batches": 1,
            "input_tokens": 50000,
            "output_tokens": 5000,
            "estimated_cost_usd": 0.0049,
            "duration_ms": 12000,
            "success": true
        }"#;
        let entry: SyncAuditEntry = serde_json::from_str(legacy).unwrap();
        assert_eq!(entry.actual_charged_usd, None);
        assert!(!entry.cost_is_actual());
        assert!((entry.effective_cost_usd() - 0.0049).abs() < f64::EPSILON);
    }

    #[test]
    fn new_entry_roundtrips_actual_charge_through_jsonl() {
        let tmp = tempfile::TempDir::new().unwrap();
        let path = tmp.path().join("roundtrip_audit.jsonl");
        let entry = entry_with_costs(0.0049, Some(0.0200));
        append_jsonl(&path, &entry).unwrap();

        let content = std::fs::read_to_string(&path).unwrap();
        let line = content.lines().next().unwrap();
        let parsed: SyncAuditEntry = serde_json::from_str(line).unwrap();
        assert_eq!(parsed.actual_charged_usd, Some(0.0200));
        assert!((parsed.effective_cost_usd() - 0.0200).abs() < f64::EPSILON);
    }
}
