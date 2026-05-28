//! Domain types for the tree summarizer.

use chrono::{DateTime, Datelike, Timelike, Utc};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

// ── Node level ─────────────────────────────────────────────────────────

/// Hierarchical level of a tree node.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeLevel {
    Root,
    Year,
    Month,
    Day,
    Hour,
}

impl NodeLevel {
    /// Maximum number of tokens allowed at this level.
    pub fn max_tokens(&self) -> u32 {
        match self {
            Self::Hour => 1_000,
            Self::Day => 2_000,
            Self::Month => 4_000,
            Self::Year => 8_000,
            Self::Root => 20_000,
        }
    }

    /// The level above this one in the hierarchy (`None` for root).
    pub fn parent_level(&self) -> Option<NodeLevel> {
        match self {
            Self::Hour => Some(Self::Day),
            Self::Day => Some(Self::Month),
            Self::Month => Some(Self::Year),
            Self::Year => Some(Self::Root),
            Self::Root => None,
        }
    }

    /// True only for the leaf level (hour).
    pub fn is_leaf(&self) -> bool {
        matches!(self, Self::Hour)
    }

    /// Parse a level string from YAML frontmatter.
    pub fn from_str_label(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "root" => Some(Self::Root),
            "year" => Some(Self::Year),
            "month" => Some(Self::Month),
            "day" => Some(Self::Day),
            "hour" => Some(Self::Hour),
            _ => None,
        }
    }

    /// Label for display / frontmatter.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Root => "root",
            Self::Year => "year",
            Self::Month => "month",
            Self::Day => "day",
            Self::Hour => "hour",
        }
    }
}

// ── Tree node ──────────────────────────────────────────────────────────

/// A single node in the summary tree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeNode {
    pub node_id: String,
    pub namespace: String,
    pub level: NodeLevel,
    pub parent_id: Option<String>,
    pub summary: String,
    pub token_count: u32,
    pub child_count: u32,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<String>,
}

// ── Status ─────────────────────────────────────────────────────────────

/// Metadata about an entire tree within a namespace.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TreeStatus {
    pub namespace: String,
    pub total_nodes: u64,
    pub depth: u32,
    pub oldest_entry: Option<DateTime<Utc>>,
    pub newest_entry: Option<DateTime<Utc>>,
    pub last_run_at: Option<DateTime<Utc>>,
}

// ── Ingest request ─────────────────────────────────────────────────────

/// Input for appending raw content to the ingestion buffer.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IngestRequest {
    pub namespace: String,
    pub content: String,
    #[serde(default)]
    pub timestamp: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<serde_json::Value>,
}

// ── Query result ───────────────────────────────────────────────────────

/// Result of a tree query at a specific node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueryResult {
    pub node: TreeNode,
    pub children: Vec<TreeNode>,
}

// ── Helpers ────────────────────────────────────────────────────────────

/// Rough token estimate: ~4 characters per token.
pub fn estimate_tokens(text: &str) -> u32 {
    (text.len() as u32).div_ceil(4)
}

/// Derive the parent node ID from a node ID.
///
/// - `"2024/03/15/14"` → `Some("2024/03/15")`
/// - `"2024/03/15"`    → `Some("2024/03")`
/// - `"2024/03"`       → `Some("2024")`
/// - `"2024"`          → `Some("root")`
/// - `"root"`          → `None`
pub fn derive_parent_id(node_id: &str) -> Option<String> {
    if node_id == "root" {
        return None;
    }
    match node_id.rfind('/') {
        Some(pos) => Some(node_id[..pos].to_string()),
        None => Some("root".to_string()),
    }
}

/// Determine the `NodeLevel` from a node ID string.
pub fn level_from_node_id(node_id: &str) -> NodeLevel {
    if node_id == "root" {
        return NodeLevel::Root;
    }
    match node_id.matches('/').count() {
        0 => NodeLevel::Year,  // "2024"
        1 => NodeLevel::Month, // "2024/03"
        2 => NodeLevel::Day,   // "2024/03/15"
        _ => NodeLevel::Hour,  // "2024/03/15/14"
    }
}

/// Derive all ancestor node IDs from a timestamp (hour through root).
///
/// Returns `(hour_id, day_id, month_id, year_id, root_id)`.
pub fn derive_node_ids(ts: &DateTime<Utc>) -> (String, String, String, String, String) {
    let year = format!("{}", ts.year());
    let month = format!("{}/{:02}", ts.year(), ts.month());
    let day = format!("{}/{:02}/{:02}", ts.year(), ts.month(), ts.day());
    let hour = format!(
        "{}/{:02}/{:02}/{:02}",
        ts.year(),
        ts.month(),
        ts.day(),
        ts.hour()
    );
    (hour, day, month, year, "root".to_string())
}

/// Convert a node ID to a relative file path within the tree directory.
///
/// - `"root"`          → `root.md`
/// - `"2024"`          → `2024/summary.md`
/// - `"2024/03"`       → `2024/03/summary.md`
/// - `"2024/03/15"`    → `2024/03/15/summary.md`
/// - `"2024/03/15/14"` → `2024/03/15/14.md`  (hour leaf — file, not folder)
pub fn node_id_to_path(node_id: &str) -> PathBuf {
    if node_id == "root" {
        return PathBuf::from("root.md");
    }
    let level = level_from_node_id(node_id);
    if level.is_leaf() {
        // Hour leaf: "2024/03/15/14" → "2024/03/15/14.md"
        PathBuf::from(format!("{node_id}.md"))
    } else {
        // Non-leaf: "2024/03" → "2024/03/summary.md"
        PathBuf::from(node_id).join("summary.md")
    }
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn node_level_max_tokens() {
        assert_eq!(NodeLevel::Hour.max_tokens(), 1_000);
        assert_eq!(NodeLevel::Day.max_tokens(), 2_000);
        assert_eq!(NodeLevel::Month.max_tokens(), 4_000);
        assert_eq!(NodeLevel::Year.max_tokens(), 8_000);
        assert_eq!(NodeLevel::Root.max_tokens(), 20_000);
    }

    #[test]
    fn node_level_parent_chain() {
        assert_eq!(NodeLevel::Hour.parent_level(), Some(NodeLevel::Day));
        assert_eq!(NodeLevel::Day.parent_level(), Some(NodeLevel::Month));
        assert_eq!(NodeLevel::Month.parent_level(), Some(NodeLevel::Year));
        assert_eq!(NodeLevel::Year.parent_level(), Some(NodeLevel::Root));
        assert_eq!(NodeLevel::Root.parent_level(), None);
    }

    #[test]
    fn derive_parent_id_chain() {
        assert_eq!(derive_parent_id("2024/03/15/14"), Some("2024/03/15".into()));
        assert_eq!(derive_parent_id("2024/03/15"), Some("2024/03".into()));
        assert_eq!(derive_parent_id("2024/03"), Some("2024".into()));
        assert_eq!(derive_parent_id("2024"), Some("root".into()));
        assert_eq!(derive_parent_id("root"), None);
    }

    #[test]
    fn level_from_node_id_all_levels() {
        assert_eq!(level_from_node_id("root"), NodeLevel::Root);
        assert_eq!(level_from_node_id("2024"), NodeLevel::Year);
        assert_eq!(level_from_node_id("2024/03"), NodeLevel::Month);
        assert_eq!(level_from_node_id("2024/03/15"), NodeLevel::Day);
        assert_eq!(level_from_node_id("2024/03/15/14"), NodeLevel::Hour);
    }

    #[test]
    fn derive_node_ids_from_timestamp() {
        let ts = Utc.with_ymd_and_hms(2024, 3, 15, 14, 30, 0).unwrap();
        let (hour, day, month, year, root) = derive_node_ids(&ts);
        assert_eq!(hour, "2024/03/15/14");
        assert_eq!(day, "2024/03/15");
        assert_eq!(month, "2024/03");
        assert_eq!(year, "2024");
        assert_eq!(root, "root");
    }

    #[test]
    fn node_id_to_path_mapping() {
        assert_eq!(node_id_to_path("root"), PathBuf::from("root.md"));
        assert_eq!(node_id_to_path("2024"), PathBuf::from("2024/summary.md"));
        assert_eq!(
            node_id_to_path("2024/03"),
            PathBuf::from("2024/03/summary.md")
        );
        assert_eq!(
            node_id_to_path("2024/03/15"),
            PathBuf::from("2024/03/15/summary.md")
        );
        assert_eq!(
            node_id_to_path("2024/03/15/14"),
            PathBuf::from("2024/03/15/14.md")
        );
    }

    #[test]
    fn estimate_tokens_rough() {
        assert_eq!(estimate_tokens(""), 0);
        assert_eq!(estimate_tokens("abcd"), 1);
        assert_eq!(estimate_tokens("abcdefgh"), 2);
        // Roughly 4 chars per token
        let text = "a".repeat(4000);
        assert_eq!(estimate_tokens(&text), 1000);
    }

    #[test]
    fn node_level_roundtrip() {
        for level in [
            NodeLevel::Root,
            NodeLevel::Year,
            NodeLevel::Month,
            NodeLevel::Day,
            NodeLevel::Hour,
        ] {
            assert_eq!(NodeLevel::from_str_label(level.as_str()), Some(level));
        }
    }
}
