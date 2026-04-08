//! Markdown file-based persistence for the summary tree.
//!
//! Each tree node is stored as a markdown file with YAML frontmatter in the
//! memory namespaces directory:
//!   `{workspace}/memory/namespaces/{namespace}/tree/`
//!
//! The folder hierarchy mirrors the time hierarchy:
//!   root.md, 2024/summary.md, 2024/03/summary.md, 2024/03/15/summary.md, 2024/03/15/14.md

use anyhow::{Context, Result};
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;
use std::path::{Path, PathBuf};

use crate::openhuman::config::Config;
use crate::openhuman::tree_summarizer::types::{
    derive_parent_id, estimate_tokens, level_from_node_id, node_id_to_path, NodeLevel, TreeNode,
    TreeStatus,
};

// ── Path helpers ───────────────────────────────────────────────────────

/// Base tree directory for a namespace.
pub fn tree_dir(config: &Config, namespace: &str) -> PathBuf {
    config
        .workspace_dir
        .join("memory")
        .join("namespaces")
        .join(sanitize(namespace))
        .join("tree")
}

/// Buffer directory where raw ingested content is staged before summarization.
pub fn buffer_dir(config: &Config, namespace: &str) -> PathBuf {
    tree_dir(config, namespace).join("buffer")
}

/// Absolute file path for a given node.
pub fn node_file_path(config: &Config, namespace: &str, node_id: &str) -> PathBuf {
    tree_dir(config, namespace).join(node_id_to_path(node_id))
}

/// Sanitize a namespace string for use as a directory name.
/// Rejects namespaces containing path-traversal or reserved characters.
fn sanitize(namespace: &str) -> String {
    let trimmed = namespace.trim();
    // Replace characters that are unsafe for directory names
    trimmed
        .replace(['/', '\\', ':', '*', '?', '"', '<', '>', '|', '.'], "_")
        .replace("__", "_")
}

/// Validate a namespace string, returning an error for empty or dangerous input.
pub fn validate_namespace(namespace: &str) -> Result<(), String> {
    let trimmed = namespace.trim();
    if trimmed.is_empty() {
        return Err("namespace must not be empty".to_string());
    }
    if trimmed.contains("..") {
        return Err("namespace must not contain '..'".to_string());
    }
    if trimmed.starts_with('/') || trimmed.starts_with('\\') {
        return Err("namespace must not start with a path separator".to_string());
    }
    Ok(())
}

/// Validate a node_id against the allowed canonical formats.
/// Accepts: "root", "YYYY", "YYYY/MM", "YYYY/MM/DD", "YYYY/MM/DD/HH".
/// Rejects path traversal, empty segments, and non-numeric components.
pub fn validate_node_id(node_id: &str) -> Result<(), String> {
    if node_id == "root" {
        return Ok(());
    }

    // Reject path traversal and dangerous characters
    if node_id.contains("..") || node_id.starts_with('/') || node_id.ends_with('/') {
        return Err(format!("invalid node_id '{node_id}': contains path traversal or leading/trailing slashes"));
    }

    let parts: Vec<&str> = node_id.split('/').collect();
    if parts.is_empty() || parts.len() > 4 {
        return Err(format!(
            "invalid node_id '{node_id}': expected 1-4 segments (YYYY[/MM[/DD[/HH]]])"
        ));
    }

    // All parts must be non-empty numeric strings
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            return Err(format!("invalid node_id '{node_id}': empty segment at position {i}"));
        }
        if !part.chars().all(|c| c.is_ascii_digit()) {
            return Err(format!(
                "invalid node_id '{node_id}': non-numeric segment '{part}' at position {i}"
            ));
        }
    }

    // Basic range validation
    if parts.len() >= 2 {
        let month: u32 = parts[1].parse().unwrap_or(0);
        if !(1..=12).contains(&month) {
            return Err(format!("invalid node_id '{node_id}': month {month} out of range 1-12"));
        }
    }
    if parts.len() >= 3 {
        let day: u32 = parts[2].parse().unwrap_or(0);
        if !(1..=31).contains(&day) {
            return Err(format!("invalid node_id '{node_id}': day {day} out of range 1-31"));
        }
    }
    if parts.len() >= 4 {
        let hour: u32 = parts[3].parse().unwrap_or(99);
        if hour > 23 {
            return Err(format!("invalid node_id '{node_id}': hour {hour} out of range 0-23"));
        }
    }

    Ok(())
}

// ── Write ──────────────────────────────────────────────────────────────

/// Write a tree node to disk as a markdown file with YAML frontmatter.
pub fn write_node(config: &Config, node: &TreeNode) -> Result<()> {
    let path = node_file_path(config, &node.namespace, &node.node_id);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .with_context(|| format!("create dirs for {}", parent.display()))?;
    }

    let metadata_line = match &node.metadata {
        Some(m) => format!("metadata: {m}\n"),
        None => String::new(),
    };

    let frontmatter = format!(
        "---\n\
         node_id: \"{}\"\n\
         namespace: \"{}\"\n\
         level: {}\n\
         parent_id: {}\n\
         token_count: {}\n\
         child_count: {}\n\
         created_at: {}\n\
         updated_at: {}\n\
         {}\
         ---\n\n",
        node.node_id,
        node.namespace,
        node.level.as_str(),
        match &node.parent_id {
            Some(pid) => format!("\"{pid}\""),
            None => "~".to_string(),
        },
        node.token_count,
        node.child_count,
        node.created_at.to_rfc3339(),
        node.updated_at.to_rfc3339(),
        metadata_line,
    );

    let content = format!("{frontmatter}{}\n", node.summary);
    std::fs::write(&path, content)
        .with_context(|| format!("write tree node {}", path.display()))?;

    tracing::debug!(
        "[tree_summarizer] wrote node {} (level={}, tokens={}) -> {}",
        node.node_id,
        node.level.as_str(),
        node.token_count,
        path.display()
    );
    Ok(())
}

// ── Read ───────────────────────────────────────────────────────────────

/// Read a single tree node from its markdown file. Returns `None` if the file
/// does not exist.
pub fn read_node(config: &Config, namespace: &str, node_id: &str) -> Result<Option<TreeNode>> {
    let path = node_file_path(config, namespace, node_id);
    if !path.exists() {
        return Ok(None);
    }
    let raw = std::fs::read_to_string(&path)
        .with_context(|| format!("read tree node {}", path.display()))?;
    parse_node_markdown(&raw, namespace, node_id).map(Some)
}

/// Read all direct children of a node.
pub fn read_children(config: &Config, namespace: &str, parent_id: &str) -> Result<Vec<TreeNode>> {
    let parent_level = level_from_node_id(parent_id);
    let base = tree_dir(config, namespace);

    match parent_level {
        NodeLevel::Root => read_subdirectory_summaries(&base, namespace, ""),
        NodeLevel::Year | NodeLevel::Month => {
            read_subdirectory_summaries(&base, namespace, parent_id)
        }
        NodeLevel::Day => read_hour_leaves(&base, namespace, parent_id),
        NodeLevel::Hour => Ok(vec![]), // leaves have no children
    }
}

/// Walk up from a node to the root, returning all ancestors (excluding the node itself).
pub fn read_ancestors(config: &Config, namespace: &str, node_id: &str) -> Result<Vec<TreeNode>> {
    let mut ancestors = Vec::new();
    let mut current = derive_parent_id(node_id);
    while let Some(pid) = current {
        if let Some(node) = read_node(config, namespace, &pid)? {
            ancestors.push(node);
        }
        current = derive_parent_id(&pid);
    }
    Ok(ancestors)
}

/// Recursively count all `.md` files in the tree directory.
pub fn count_nodes(config: &Config, namespace: &str) -> Result<u64> {
    let base = tree_dir(config, namespace);
    if !base.exists() {
        return Ok(0);
    }
    count_md_files(&base)
}

/// Scan the tree to produce a status summary.
pub fn get_tree_status(config: &Config, namespace: &str) -> Result<TreeStatus> {
    let base = tree_dir(config, namespace);
    let total_nodes = if base.exists() {
        count_md_files(&base)?
    } else {
        0
    };

    // Determine depth by checking which levels exist.
    let mut depth = 0u32;
    let root_path = base.join("root.md");
    if root_path.exists() {
        depth = 1;
    }

    // Scan for years/months/days/hours to figure out actual depth and date range.
    let mut oldest: Option<DateTime<Utc>> = None;
    let mut newest: Option<DateTime<Utc>> = None;

    if base.exists() {
        for entry in std::fs::read_dir(&base).into_iter().flatten().flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) && name.len() == 4 {
                if depth < 2 {
                    depth = 2;
                }
                // Scan months, days, hours inside
                let year_dir = entry.path();
                for month_entry in std::fs::read_dir(&year_dir).into_iter().flatten().flatten() {
                    if month_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                        if depth < 3 {
                            depth = 3;
                        }
                        let month_dir = month_entry.path();
                        for day_entry in std::fs::read_dir(&month_dir)
                            .into_iter()
                            .flatten()
                            .flatten()
                        {
                            if day_entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
                                if depth < 4 {
                                    depth = 4;
                                }
                                // Check for hour .md files
                                let day_dir = day_entry.path();
                                for hour_entry in
                                    std::fs::read_dir(&day_dir).into_iter().flatten().flatten()
                                {
                                    let hname =
                                        hour_entry.file_name().to_string_lossy().to_string();
                                    if hname.ends_with(".md") && hname != "summary.md" {
                                        if depth < 5 {
                                            depth = 5;
                                        }
                                        // Try to parse timestamp from path
                                        if let Some(ts) = timestamp_from_hour_path(
                                            &name,
                                            &month_entry.file_name().to_string_lossy().to_string(),
                                            &day_entry.file_name().to_string_lossy().to_string(),
                                            &hname,
                                        ) {
                                            match &oldest {
                                                None => oldest = Some(ts),
                                                Some(o) if ts < *o => oldest = Some(ts),
                                                _ => {}
                                            }
                                            match &newest {
                                                None => newest = Some(ts),
                                                Some(n) if ts > *n => newest = Some(ts),
                                                _ => {}
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(TreeStatus {
        namespace: namespace.to_string(),
        total_nodes,
        depth,
        oldest_entry: oldest,
        newest_entry: newest,
        last_run_at: None, // filled by caller if needed
    })
}

/// Remove the entire tree directory for a namespace.
pub fn delete_tree(config: &Config, namespace: &str) -> Result<u64> {
    let base = tree_dir(config, namespace);
    if !base.exists() {
        return Ok(0);
    }
    let count = count_md_files(&base)?;
    std::fs::remove_dir_all(&base).with_context(|| format!("delete tree at {}", base.display()))?;
    tracing::debug!(
        "[tree_summarizer] deleted tree for namespace '{}' ({} nodes)",
        namespace,
        count
    );
    Ok(count)
}

// ── Buffer operations ──────────────────────────────────────────────────

/// Append raw content to the ingestion buffer as a timestamped file.
/// Optionally includes metadata as a JSON object stored alongside the content.
pub fn buffer_write(
    config: &Config,
    namespace: &str,
    content: &str,
    ts: &DateTime<Utc>,
    metadata: Option<&Value>,
) -> Result<PathBuf> {
    let dir = buffer_dir(config, namespace);
    std::fs::create_dir_all(&dir)
        .with_context(|| format!("create buffer dir {}", dir.display()))?;

    let filename = format!(
        "{}_{}.md",
        ts.timestamp_millis(),
        &uuid::Uuid::new_v4().to_string()[..8]
    );
    let path = dir.join(&filename);

    // If metadata is provided, write it as a YAML frontmatter block
    let file_content = if let Some(meta) = metadata {
        let meta_str = serde_json::to_string(meta).unwrap_or_default();
        format!("---\nmetadata: {meta_str}\n---\n\n{content}")
    } else {
        content.to_string()
    };

    std::fs::write(&path, file_content)
        .with_context(|| format!("write buffer entry {}", path.display()))?;

    tracing::debug!(
        "[tree_summarizer] buffered {} bytes for namespace '{}' -> {}",
        content.len(),
        namespace,
        filename
    );
    Ok(path)
}

/// Read and drain all buffered entries, returning `(filename, content)` pairs
/// sorted by filename (chronological). Files are deleted after reading.
///
/// Returns an error if any file deletion fails, to prevent duplicate processing.
pub fn buffer_drain(config: &Config, namespace: &str) -> Result<Vec<(String, String)>> {
    let dir = buffer_dir(config, namespace);
    if !dir.exists() {
        return Ok(vec![]);
    }

    let mut entries: Vec<(String, PathBuf)> = Vec::new();
    for entry in std::fs::read_dir(&dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().map(|e| e == "md").unwrap_or(false) {
            let name = entry.file_name().to_string_lossy().to_string();
            entries.push((name, path));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    let mut contents = Vec::with_capacity(entries.len());
    for (name, path) in &entries {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("read buffer entry {}", path.display()))?;
        // Strip metadata frontmatter if present, pass raw content
        let text = strip_buffer_frontmatter(&raw);
        contents.push((name.clone(), text));
    }

    // Delete after successful reads — propagate errors to prevent duplicates
    for (name, path) in &entries {
        std::fs::remove_file(path).with_context(|| {
            format!(
                "failed to remove buffer entry '{}' at {}",
                name,
                path.display()
            )
        })?;
    }

    tracing::debug!(
        "[tree_summarizer] drained {} buffer entries for namespace '{}'",
        contents.len(),
        namespace
    );
    Ok(contents)
}

/// Strip the optional metadata frontmatter from a buffer entry,
/// returning only the content body.
fn strip_buffer_frontmatter(raw: &str) -> String {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return raw.to_string();
    }
    let after_open = &trimmed[3..];
    if let Some(close_pos) = after_open.find("\n---") {
        let body_start = close_pos + 4;
        after_open[body_start..].trim_start_matches('\n').to_string()
    } else {
        raw.to_string()
    }
}

// ── Internal helpers ───────────────────────────────────────────────────

/// Read summary.md files from subdirectories of a given parent path.
fn read_subdirectory_summaries(
    base: &Path,
    namespace: &str,
    parent_id: &str,
) -> Result<Vec<TreeNode>> {
    let scan_dir = if parent_id.is_empty() {
        base.to_path_buf()
    } else {
        base.join(parent_id)
    };
    if !scan_dir.exists() {
        return Ok(vec![]);
    }

    let mut children = Vec::new();
    for entry in std::fs::read_dir(&scan_dir)? {
        let entry = entry?;
        if !entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
            continue;
        }
        let child_name = entry.file_name().to_string_lossy().to_string();
        // Skip non-numeric directories and the buffer directory
        if child_name == "buffer"
            || child_name == "buffer_backup"
            || child_name.chars().any(|c| !c.is_ascii_digit())
        {
            continue;
        }
        let child_id = if parent_id.is_empty() {
            child_name
        } else {
            format!("{parent_id}/{child_name}")
        };
        let summary_path = entry.path().join("summary.md");
        if summary_path.exists() {
            let raw = std::fs::read_to_string(&summary_path)?;
            if let Ok(node) = parse_node_markdown(&raw, namespace, &child_id) {
                children.push(node);
            }
        }
    }

    children.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    Ok(children)
}

/// Read hour leaf .md files (excluding summary.md) from a day directory.
fn read_hour_leaves(base: &Path, namespace: &str, day_id: &str) -> Result<Vec<TreeNode>> {
    let day_dir = base.join(day_id);
    if !day_dir.exists() {
        return Ok(vec![]);
    }

    let mut leaves = Vec::new();
    for entry in std::fs::read_dir(&day_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.ends_with(".md") || name == "summary.md" {
            continue;
        }
        let hour_part = name.trim_end_matches(".md");
        let node_id = format!("{day_id}/{hour_part}");
        let raw = std::fs::read_to_string(entry.path())?;
        if let Ok(node) = parse_node_markdown(&raw, namespace, &node_id) {
            leaves.push(node);
        }
    }

    leaves.sort_by(|a, b| a.node_id.cmp(&b.node_id));
    Ok(leaves)
}

/// Public entry point for parsing a markdown node (used by engine rebuild).
pub fn parse_node_markdown_pub(raw: &str, namespace: &str, node_id: &str) -> Result<TreeNode> {
    parse_node_markdown(raw, namespace, node_id)
}

/// Parse a markdown file with YAML frontmatter into a `TreeNode`.
fn parse_node_markdown(raw: &str, namespace: &str, node_id: &str) -> Result<TreeNode> {
    let (frontmatter, body) = split_frontmatter(raw);

    let level = frontmatter
        .get("level")
        .and_then(|v| NodeLevel::from_str_label(v))
        .unwrap_or_else(|| level_from_node_id(node_id));

    let parent_id = frontmatter
        .get("parent_id")
        .and_then(|v| {
            let trimmed = v.trim().trim_matches('"');
            if trimmed == "~" || trimmed.is_empty() {
                None
            } else {
                Some(trimmed.to_string())
            }
        })
        .or_else(|| derive_parent_id(node_id));

    let token_count = frontmatter
        .get("token_count")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or_else(|| estimate_tokens(&body));

    let child_count = frontmatter
        .get("child_count")
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(0);

    let created_at = frontmatter
        .get("created_at")
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let updated_at = frontmatter
        .get("updated_at")
        .and_then(|v| DateTime::parse_from_rfc3339(v).ok())
        .map(|dt| dt.with_timezone(&Utc))
        .unwrap_or_else(Utc::now);

    let metadata = frontmatter.get("metadata").map(|v| v.to_string());

    Ok(TreeNode {
        node_id: node_id.to_string(),
        namespace: namespace.to_string(),
        level,
        parent_id,
        summary: body,
        token_count,
        child_count,
        created_at,
        updated_at,
        metadata,
    })
}

/// Split markdown into (frontmatter key-value map, body text).
fn split_frontmatter(raw: &str) -> (std::collections::HashMap<String, String>, String) {
    let mut map = std::collections::HashMap::new();
    let trimmed = raw.trim_start();

    if !trimmed.starts_with("---") {
        return (map, raw.to_string());
    }

    // Find the closing ---
    let after_open = &trimmed[3..];
    if let Some(close_pos) = after_open.find("\n---") {
        let fm_block = &after_open[..close_pos];
        let body_start = close_pos + 4; // skip "\n---"
        let body = after_open[body_start..]
            .trim_start_matches('\n')
            .to_string();

        for line in fm_block.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if let Some(colon_pos) = line.find(':') {
                let key = line[..colon_pos].trim().to_string();
                let value = line[colon_pos + 1..].trim().trim_matches('"').to_string();
                map.insert(key, value);
            }
        }

        (map, body)
    } else {
        (map, raw.to_string())
    }
}

fn count_md_files(dir: &Path) -> Result<u64> {
    let mut count = 0u64;
    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let ft = entry.file_type()?;
        if ft.is_dir() {
            let name = entry.file_name().to_string_lossy().to_string();
            if name == "buffer" || name == "buffer_backup" {
                continue; // skip buffer directories
            }
            count += count_md_files(&entry.path())?;
        } else if ft.is_file() {
            if entry.path().extension().map(|e| e == "md").unwrap_or(false) {
                count += 1;
            }
        }
    }
    Ok(count)
}

fn timestamp_from_hour_path(
    year: &str,
    month: &str,
    day: &str,
    hour_file: &str,
) -> Option<DateTime<Utc>> {
    let hour = hour_file.trim_end_matches(".md");
    let y: i32 = year.parse().ok()?;
    let m: u32 = month.parse().ok()?;
    let d: u32 = day.parse().ok()?;
    let h: u32 = hour.parse().ok()?;
    chrono::Utc.with_ymd_and_hms(y, m, d, h, 0, 0).single()
}

// ── Tests ──────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config(tmp: &TempDir) -> Config {
        let config = Config {
            workspace_dir: tmp.path().join("workspace"),
            config_path: tmp.path().join("config.toml"),
            ..Config::default()
        };
        std::fs::create_dir_all(&config.workspace_dir).unwrap();
        config
    }

    fn make_node(namespace: &str, node_id: &str, summary: &str) -> TreeNode {
        let level = level_from_node_id(node_id);
        TreeNode {
            node_id: node_id.to_string(),
            namespace: namespace.to_string(),
            level,
            parent_id: derive_parent_id(node_id),
            summary: summary.to_string(),
            token_count: estimate_tokens(summary),
            child_count: 0,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            metadata: None,
        }
    }

    #[test]
    fn write_and_read_node_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";

        let node = make_node(ns, "root", "All-time summary of events.");
        write_node(&config, &node).unwrap();

        let read_back = read_node(&config, ns, "root").unwrap().unwrap();
        assert_eq!(read_back.node_id, "root");
        assert_eq!(read_back.level, NodeLevel::Root);
        assert_eq!(read_back.summary, "All-time summary of events.");
        assert!(read_back.parent_id.is_none());
    }

    #[test]
    fn write_and_read_hour_leaf() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";

        let node = make_node(ns, "2024/03/15/14", "Hour 14 summary.");
        write_node(&config, &node).unwrap();

        let read_back = read_node(&config, ns, "2024/03/15/14").unwrap().unwrap();
        assert_eq!(read_back.level, NodeLevel::Hour);
        assert_eq!(read_back.parent_id.as_deref(), Some("2024/03/15"));
        assert_eq!(read_back.summary, "Hour 14 summary.");
    }

    #[test]
    fn read_children_of_day() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";

        // Write some hour leaves
        for hour in [10, 11, 14] {
            let node = make_node(
                ns,
                &format!("2024/03/15/{hour:02}"),
                &format!("Hour {hour}."),
            );
            write_node(&config, &node).unwrap();
        }
        // Write the day summary (should not appear as a child)
        let day = make_node(ns, "2024/03/15", "Day summary.");
        write_node(&config, &day).unwrap();

        let children = read_children(&config, ns, "2024/03/15").unwrap();
        assert_eq!(children.len(), 3);
        assert_eq!(children[0].node_id, "2024/03/15/10");
        assert_eq!(children[1].node_id, "2024/03/15/11");
        assert_eq!(children[2].node_id, "2024/03/15/14");
    }

    #[test]
    fn read_children_of_root() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";

        for year in ["2023", "2024"] {
            let node = make_node(ns, year, &format!("Year {year} summary."));
            write_node(&config, &node).unwrap();
        }

        let children = read_children(&config, ns, "root").unwrap();
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].node_id, "2023");
        assert_eq!(children[1].node_id, "2024");
    }

    #[test]
    fn read_node_missing_returns_none() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        assert!(read_node(&config, "ns", "root").unwrap().is_none());
    }

    #[test]
    fn count_nodes_and_status() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";

        write_node(&config, &make_node(ns, "root", "root")).unwrap();
        write_node(&config, &make_node(ns, "2024", "year")).unwrap();
        write_node(&config, &make_node(ns, "2024/03", "month")).unwrap();
        write_node(&config, &make_node(ns, "2024/03/15", "day")).unwrap();
        write_node(&config, &make_node(ns, "2024/03/15/14", "hour")).unwrap();

        assert_eq!(count_nodes(&config, ns).unwrap(), 5);

        let status = get_tree_status(&config, ns).unwrap();
        assert_eq!(status.total_nodes, 5);
        assert_eq!(status.depth, 5);
    }

    #[test]
    fn delete_tree_removes_all() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";

        write_node(&config, &make_node(ns, "root", "root")).unwrap();
        write_node(&config, &make_node(ns, "2024/03/15/14", "hour")).unwrap();

        let deleted = delete_tree(&config, ns).unwrap();
        assert!(deleted >= 2);
        assert_eq!(count_nodes(&config, ns).unwrap(), 0);
    }

    #[test]
    fn buffer_write_and_drain() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";
        let now = Utc::now();

        buffer_write(&config, ns, "entry one", &now, None).unwrap();
        buffer_write(&config, ns, "entry two", &now, None).unwrap();

        let drained = buffer_drain(&config, ns).unwrap();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].1, "entry one");
        assert_eq!(drained[1].1, "entry two");

        // Buffer should be empty now
        let again = buffer_drain(&config, ns).unwrap();
        assert!(again.is_empty());
    }

    #[test]
    fn buffer_write_with_metadata() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";
        let now = Utc::now();

        let meta = serde_json::json!({"source": "test", "priority": 1});
        buffer_write(&config, ns, "entry with meta", &now, Some(&meta)).unwrap();

        let drained = buffer_drain(&config, ns).unwrap();
        assert_eq!(drained.len(), 1);
        // Content should be stripped of frontmatter
        assert_eq!(drained[0].1, "entry with meta");
    }

    #[test]
    fn ancestors_walk_to_root() {
        let tmp = TempDir::new().unwrap();
        let config = test_config(&tmp);
        let ns = "test-ns";

        write_node(&config, &make_node(ns, "root", "root")).unwrap();
        write_node(&config, &make_node(ns, "2024", "year")).unwrap();
        write_node(&config, &make_node(ns, "2024/03", "month")).unwrap();
        write_node(&config, &make_node(ns, "2024/03/15", "day")).unwrap();

        let ancestors = read_ancestors(&config, ns, "2024/03/15/14").unwrap();
        let ids: Vec<&str> = ancestors.iter().map(|n| n.node_id.as_str()).collect();
        assert_eq!(ids, vec!["2024/03/15", "2024/03", "2024", "root"]);
    }

    #[test]
    fn frontmatter_parsing() {
        let raw = "---\nnode_id: \"root\"\nlevel: root\ntoken_count: 42\n---\n\nHello world.";
        let (fm, body) = split_frontmatter(raw);
        assert_eq!(fm.get("level").unwrap(), "root");
        assert_eq!(fm.get("token_count").unwrap(), "42");
        assert_eq!(body, "Hello world.");
    }

    #[test]
    fn validate_node_id_accepts_valid() {
        assert!(validate_node_id("root").is_ok());
        assert!(validate_node_id("2024").is_ok());
        assert!(validate_node_id("2024/03").is_ok());
        assert!(validate_node_id("2024/03/15").is_ok());
        assert!(validate_node_id("2024/03/15/14").is_ok());
    }

    #[test]
    fn validate_node_id_rejects_traversal() {
        assert!(validate_node_id("..").is_err());
        assert!(validate_node_id("../etc").is_err());
        assert!(validate_node_id("2024/../etc").is_err());
        assert!(validate_node_id("/2024").is_err());
        assert!(validate_node_id("2024/").is_err());
    }

    #[test]
    fn validate_node_id_rejects_non_numeric() {
        assert!(validate_node_id("abc").is_err());
        assert!(validate_node_id("2024/abc").is_err());
        assert!(validate_node_id("2024/03/15/foo").is_err());
    }

    #[test]
    fn validate_node_id_rejects_out_of_range() {
        assert!(validate_node_id("2024/13").is_err()); // month 13
        assert!(validate_node_id("2024/03/32").is_err()); // day 32
        assert!(validate_node_id("2024/03/15/24").is_err()); // hour 24
    }

    #[test]
    fn validate_namespace_rejects_dangerous() {
        assert!(validate_namespace("").is_err());
        assert!(validate_namespace("  ").is_err());
        assert!(validate_namespace("../etc").is_err());
        assert!(validate_namespace("/absolute").is_err());
    }

    #[test]
    fn validate_namespace_accepts_valid() {
        assert!(validate_namespace("my-namespace").is_ok());
        assert!(validate_namespace("skill:gmail:user@example.com").is_ok());
    }
}
