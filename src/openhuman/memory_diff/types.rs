//! Domain types for snapshot-based memory source change tracking.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum SnapshotTrigger {
    Auto,
    Manual,
}

impl SnapshotTrigger {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Snapshot {
    pub id: String,
    pub source_id: String,
    pub source_kind: String,
    pub label: String,
    pub trigger: SnapshotTrigger,
    pub item_count: u32,
    pub taken_at_ms: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ChangeKind {
    Added,
    Removed,
    Modified,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ItemChange {
    pub item_id: String,
    pub title: String,
    pub kind: ChangeKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub old_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub new_content_hash: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub text_diff: Option<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema)]
pub struct DiffSummary {
    pub added: u32,
    pub removed: u32,
    pub modified: u32,
    pub unchanged: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DiffResult {
    pub source_id: String,
    pub source_kind: String,
    pub source_label: String,
    pub from_snapshot_id: Option<String>,
    pub to_snapshot_id: String,
    pub summary: DiffSummary,
    pub changes: Vec<ItemChange>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct Checkpoint {
    pub id: String,
    pub label: String,
    pub created_at_ms: i64,
    pub snapshot_ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct CrossSourceDiff {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checkpoint_id: Option<String>,
    pub computed_at_ms: i64,
    pub summary: DiffSummary,
    pub per_source: Vec<DiffResult>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_trigger_round_trips() {
        for trigger in [SnapshotTrigger::Auto, SnapshotTrigger::Manual] {
            let json = serde_json::to_string(&trigger).unwrap();
            let decoded: SnapshotTrigger = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, trigger);
        }
    }

    #[test]
    fn change_kind_round_trips() {
        for kind in [ChangeKind::Added, ChangeKind::Removed, ChangeKind::Modified] {
            let json = serde_json::to_string(&kind).unwrap();
            let decoded: ChangeKind = serde_json::from_str(&json).unwrap();
            assert_eq!(decoded, kind);
        }
    }

    #[test]
    fn diff_summary_defaults_to_zero() {
        let s = DiffSummary::default();
        assert_eq!(s.added, 0);
        assert_eq!(s.removed, 0);
        assert_eq!(s.modified, 0);
        assert_eq!(s.unchanged, 0);
    }
}
