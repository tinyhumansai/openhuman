//! Subconscious scratchpad — persistent working memory across ticks.
//!
//! The scratchpad holds up to `MAX_ENTRIES` (default 100) short thoughts
//! that carry over between subconscious ticks, giving the agent a
//! consistent stream-of-consciousness.
//!
//! Stored as `{workspace_dir}/subconscious/SUBCONSCIOUS_SCRATCHPAD.md`
//! so it's trivially inspectable with any text editor.
//!
//! ## File format
//!
//! # Subconscious Scratchpad
//!
//! ```json
//! [
//!   {
//!     "id": "abc123",
//!     "body": "The thought body goes here.",
//!     "priority": 5,
//!     "created_at": 1700000000.123456,
//!     "updated_at": 1700000000.123456
//!   }
//! ]
//! ```
//!
//! The agent manages its own scratchpad via three tools:
//! `scratchpad_add`, `scratchpad_edit`, `scratchpad_remove`.

pub mod tools;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const DEFAULT_MAX_ENTRIES: usize = 100;

const FILENAME: &str = "SUBCONSCIOUS_SCRATCHPAD.md";

/// One scratchpad entry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScratchpadEntry {
    pub id: String,
    pub body: String,
    pub priority: u32,
    pub created_at: f64,
    pub updated_at: f64,
}

fn scratchpad_path(workspace_dir: &Path) -> PathBuf {
    workspace_dir.join("subconscious").join(FILENAME)
}

pub fn load(workspace_dir: &Path) -> Result<Vec<ScratchpadEntry>> {
    let path = scratchpad_path(workspace_dir);
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e).context("read scratchpad"),
    };
    Ok(parse_entries(&content))
}

pub fn add(workspace_dir: &Path, body: &str, priority: u32, max_entries: usize) -> Result<String> {
    let mut entries = load(workspace_dir)?;
    let id = short_id();
    let now = now_secs();
    entries.push(ScratchpadEntry {
        id: id.clone(),
        body: body.to_string(),
        priority,
        created_at: now,
        updated_at: now,
    });
    evict(&mut entries, max_entries);
    save(workspace_dir, &entries)?;
    Ok(id)
}

pub fn edit(workspace_dir: &Path, id: &str, body: &str, priority: Option<u32>) -> Result<bool> {
    let mut entries = load(workspace_dir)?;
    let Some(entry) = entries.iter_mut().find(|e| e.id == id) else {
        return Ok(false);
    };
    entry.body = body.to_string();
    if let Some(p) = priority {
        entry.priority = p;
    }
    entry.updated_at = now_secs();
    save(workspace_dir, &entries)?;
    Ok(true)
}

pub fn remove(workspace_dir: &Path, id: &str) -> Result<bool> {
    let mut entries = load(workspace_dir)?;
    let before = entries.len();
    entries.retain(|e| e.id != id);
    if entries.len() == before {
        return Ok(false);
    }
    save(workspace_dir, &entries)?;
    Ok(true)
}

pub fn render_for_prompt(entries: &[ScratchpadEntry]) -> String {
    if entries.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Scratchpad (your persistent working memory)\n\n");
    out.push_str(
        "These are your own notes from previous ticks. Update, remove, or \
         add entries as your understanding evolves.\n\n",
    );
    for entry in entries {
        let ts = chrono::DateTime::from_timestamp(entry.updated_at as i64, 0)
            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
            .unwrap_or_else(|| "unknown".to_string());
        out.push_str(&format!(
            "- **[{}]** (p{}) {}\n  _updated: {}_\n",
            entry.id, entry.priority, entry.body, ts
        ));
    }
    out
}

// ── File I/O ────────────────────────────────────────────────────────────────

fn save(workspace_dir: &Path, entries: &[ScratchpadEntry]) -> Result<()> {
    let path = scratchpad_path(workspace_dir);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).context("create scratchpad dir")?;
    }
    let content = render_file(entries);
    std::fs::write(&path, content).context("write scratchpad")?;
    Ok(())
}

fn render_file(entries: &[ScratchpadEntry]) -> String {
    let mut out = String::from("# Subconscious Scratchpad\n\n");
    if entries.is_empty() {
        return out;
    }
    out.push_str("```json\n");
    match serde_json::to_string_pretty(entries) {
        Ok(json) => out.push_str(&json),
        Err(e) => {
            log::warn!("[subconscious] failed to render scratchpad JSON: {e}");
            out.push_str("[]");
        }
    }
    out.push_str("\n```\n");
    out
}

fn parse_entries(content: &str) -> Vec<ScratchpadEntry> {
    if let Some(entries) = parse_json_entries(content) {
        return entries;
    }

    let mut entries = Vec::new();

    for block in content.split("\n---\n") {
        let block = block.trim();
        if block.is_empty() || block.starts_with("# ") {
            if let Some(rest) = block.strip_prefix("# Subconscious Scratchpad") {
                let rest = rest.trim();
                if rest.is_empty() {
                    continue;
                }
                if let Some(entry) = parse_single_block(rest) {
                    entries.push(entry);
                }
                continue;
            }
            continue;
        }
        if let Some(entry) = parse_single_block(block) {
            entries.push(entry);
        }
    }

    entries
}

fn parse_json_entries(content: &str) -> Option<Vec<ScratchpadEntry>> {
    let marker = "```json";
    let start = content.find(marker)? + marker.len();
    let rest = &content[start..];
    let end = rest.rfind("```")?;
    let json = rest[..end].trim();
    if json.is_empty() {
        return Some(Vec::new());
    }
    match serde_json::from_str(json) {
        Ok(entries) => Some(entries),
        Err(e) => {
            log::warn!("[subconscious] failed to parse scratchpad JSON: {e}");
            None
        }
    }
}

fn parse_single_block(block: &str) -> Option<ScratchpadEntry> {
    let meta_start = block.find("<!-- entry:")?;
    let meta_end = block[meta_start..].find("-->")? + meta_start + 3;
    let meta_line = &block[meta_start..meta_end];

    let id = extract_meta(meta_line, "entry:")?;
    let priority = extract_meta(meta_line, "p:")
        .and_then(|s| s.parse::<u32>().ok())
        .unwrap_or(0);
    let created_at = extract_meta(meta_line, "created:")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(0.0);
    let updated_at = extract_meta(meta_line, "updated:")
        .and_then(|s| s.parse::<f64>().ok())
        .unwrap_or(created_at);

    let body = block[meta_end..].trim().to_string();
    if body.is_empty() {
        return None;
    }

    Some(ScratchpadEntry {
        id,
        body,
        priority,
        created_at,
        updated_at,
    })
}

fn extract_meta(line: &str, key: &str) -> Option<String> {
    let start = line.find(key)? + key.len();
    let rest = &line[start..];
    let end = rest
        .find(|c: char| c.is_whitespace() || c == '-')
        .unwrap_or(rest.len());
    let val = rest[..end].trim().to_string();
    if val.is_empty() {
        None
    } else {
        Some(val)
    }
}

fn evict(entries: &mut Vec<ScratchpadEntry>, max: usize) {
    if entries.len() <= max {
        return;
    }
    entries.sort_by(|a, b| {
        b.priority.cmp(&a.priority).then(
            b.updated_at
                .partial_cmp(&a.updated_at)
                .unwrap_or(std::cmp::Ordering::Equal),
        )
    });
    entries.truncate(max);
}

fn short_id() -> String {
    let uuid = uuid::Uuid::new_v4().to_string();
    uuid[..8].to_string()
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_workspace() -> TempDir {
        tempfile::tempdir().unwrap()
    }

    #[test]
    fn add_and_load_round_trip() {
        let ws = temp_workspace();
        let id = add(ws.path(), "first thought", 1, 100).unwrap();
        let entries = load(ws.path()).unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].id, id);
        assert_eq!(entries[0].body, "first thought");
        assert_eq!(entries[0].priority, 1);
    }

    #[test]
    fn edit_updates_body_and_priority() {
        let ws = temp_workspace();
        let id = add(ws.path(), "old", 0, 100).unwrap();
        let found = edit(ws.path(), &id, "new", Some(5)).unwrap();
        assert!(found);
        let entries = load(ws.path()).unwrap();
        assert_eq!(entries[0].body, "new");
        assert_eq!(entries[0].priority, 5);
    }

    #[test]
    fn remove_deletes_entry() {
        let ws = temp_workspace();
        let id = add(ws.path(), "gone", 0, 100).unwrap();
        assert!(remove(ws.path(), &id).unwrap());
        assert!(load(ws.path()).unwrap().is_empty());
    }

    #[test]
    fn remove_nonexistent_returns_false() {
        let ws = temp_workspace();
        add(ws.path(), "keep", 0, 100).unwrap();
        assert!(!remove(ws.path(), "nope").unwrap());
    }

    #[test]
    fn add_evicts_oldest_low_priority_beyond_cap() {
        let ws = temp_workspace();
        for i in 0..5 {
            add(ws.path(), &format!("note {i}"), 0, 100).unwrap();
        }
        add(ws.path(), "high priority", 10, 100).unwrap();
        add(ws.path(), "newest", 0, 3).unwrap();
        let entries = load(ws.path()).unwrap();
        assert!(entries.len() <= 3);
        assert!(entries.iter().any(|e| e.body == "high priority"));
    }

    #[test]
    fn render_for_prompt_formats_entries() {
        let entries = vec![ScratchpadEntry {
            id: "abc".to_string(),
            body: "test thought".to_string(),
            priority: 2,
            created_at: 1700000000.0,
            updated_at: 1700000000.0,
        }];
        let rendered = render_for_prompt(&entries);
        assert!(rendered.contains("## Scratchpad"));
        assert!(rendered.contains("[abc]"));
        assert!(rendered.contains("test thought"));
        assert!(rendered.contains("(p2)"));
    }

    #[test]
    fn render_for_prompt_empty_returns_empty() {
        assert!(render_for_prompt(&[]).is_empty());
    }

    #[test]
    fn load_missing_file_returns_empty() {
        let ws = temp_workspace();
        assert!(load(ws.path()).unwrap().is_empty());
    }

    #[test]
    fn file_is_readable_markdown() {
        let ws = temp_workspace();
        add(ws.path(), "thought one", 5, 100).unwrap();
        add(ws.path(), "thought two", 0, 100).unwrap();
        let content = std::fs::read_to_string(scratchpad_path(ws.path())).unwrap();
        assert!(content.starts_with("# Subconscious Scratchpad"));
        assert!(content.contains("```json"));
        assert!(content.contains("thought one"));
        assert!(content.contains("thought two"));
        assert!(content.contains("\"created_at\""));
    }

    #[test]
    fn parse_round_trip_preserves_data() {
        let original = vec![
            ScratchpadEntry {
                id: "aaa".to_string(),
                body: "first".to_string(),
                priority: 3,
                created_at: 1700000000.0,
                updated_at: 1700000100.0,
            },
            ScratchpadEntry {
                id: "bbb".to_string(),
                body: "second\nwith newlines".to_string(),
                priority: 0,
                created_at: 1700000050.0,
                updated_at: 1700000050.0,
            },
        ];
        let rendered = render_file(&original);
        let parsed = parse_entries(&rendered);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].id, "aaa");
        assert_eq!(parsed[0].body, "first");
        assert_eq!(parsed[0].priority, 3);
        assert_eq!(parsed[1].id, "bbb");
        assert_eq!(parsed[1].body, "second\nwith newlines");
    }

    #[test]
    fn parse_round_trip_preserves_markdown_separators_and_subsecond_timestamps() {
        let original = vec![ScratchpadEntry {
            id: "aaa".to_string(),
            body: "first\n---\nsecond".to_string(),
            priority: 3,
            created_at: 1700000000.123456,
            updated_at: 1700000100.654321,
        }];
        let rendered = render_file(&original);
        let parsed = parse_entries(&rendered);
        assert_eq!(parsed, original);
    }

    #[test]
    fn parse_legacy_markdown_entries() {
        let content = "# Subconscious Scratchpad\n\n<!-- entry:abc p:2 created:1700000000 updated:1700000001 -->\nlegacy thought";
        let parsed = parse_entries(content);
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].id, "abc");
        assert_eq!(parsed[0].body, "legacy thought");
        assert_eq!(parsed[0].priority, 2);
    }
}
