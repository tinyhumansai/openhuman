//! Session transcript persistence for KV cache stability.
//!
//! Stores the **exact** `Vec<ChatMessage>` sent to the LLM provider as
//! a human-readable `.md` file. On session resume the transcript is read
//! back to produce byte-identical messages, ensuring the inference
//! backend's KV cache prefix is reused rather than re-prefilled.
//!
//! ## File format
//!
//! ```text
//! <!-- session_transcript
//! agent: code_executor
//! dispatcher: native
//! cache_boundary: 1847
//! created: 2026-04-11T14:30:00Z
//! updated: 2026-04-11T14:35:22Z
//! turn_count: 3
//! -->
//!
//! <!--MSG role="system"-->
//! <exact system prompt bytes>
//! <!--/MSG-->
//!
//! <!--MSG role="user"-->
//! <exact user message bytes>
//! <!--/MSG-->
//! ```
//!
//! Content between `<!--MSG ...-->` and `<!--/MSG-->` is the **exact**
//! `ChatMessage.content`. The single escape: any literal `<!--/MSG-->`
//! inside content is written as `<!--\/MSG-->` and reversed on read.
//!
//! ## Storage layout
//!
//! ```text
//! {workspace}/sessions/DDMMYYYY/{agent}_{index}.md
//! ```

use crate::openhuman::providers::ChatMessage;
use anyhow::{Context, Result};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::path::{Path, PathBuf};

const MSG_OPEN_PREFIX: &str = "<!--MSG role=\"";
const MSG_OPEN_SUFFIX: &str = "\"-->";
const MSG_CLOSE: &str = "<!--/MSG-->";
const MSG_CLOSE_ESCAPED: &str = "<!--\\/MSG-->";

/// Metadata header for a session transcript file.
#[derive(Debug, Clone)]
pub struct TranscriptMeta {
    pub agent_name: String,
    pub dispatcher: String,
    pub cache_boundary: Option<usize>,
    pub created: String,
    pub updated: String,
    pub turn_count: usize,
    /// Cumulative input tokens across all provider calls this session.
    pub input_tokens: u64,
    /// Cumulative output tokens across all provider calls this session.
    pub output_tokens: u64,
    /// Cumulative input tokens served from the KV cache.
    pub cached_input_tokens: u64,
    /// Cumulative amount charged in USD.
    pub charged_amount_usd: f64,
}

/// A parsed session transcript: metadata + exact message array.
#[derive(Debug, Clone)]
pub struct SessionTranscript {
    pub meta: TranscriptMeta,
    pub messages: Vec<ChatMessage>,
}

// ── Write ────────────────────────────────────────────────────────────

/// Write a session transcript to `path`. Full rewrite (not append)
/// because context reduction may have removed earlier messages.
pub fn write_transcript(path: &Path, messages: &[ChatMessage], meta: &TranscriptMeta) -> Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .with_context(|| format!("create transcript dir {}", parent.display()))?;
    }

    let mut buf = String::new();

    // Header
    buf.push_str("<!-- session_transcript\n");
    let _ = writeln!(buf, "agent: {}", meta.agent_name);
    let _ = writeln!(buf, "dispatcher: {}", meta.dispatcher);
    if let Some(boundary) = meta.cache_boundary {
        let _ = writeln!(buf, "cache_boundary: {}", boundary);
    }
    let _ = writeln!(buf, "created: {}", meta.created);
    let _ = writeln!(buf, "updated: {}", meta.updated);
    let _ = writeln!(buf, "turn_count: {}", meta.turn_count);
    if meta.input_tokens > 0 || meta.output_tokens > 0 {
        let _ = writeln!(buf, "input_tokens: {}", meta.input_tokens);
        let _ = writeln!(buf, "output_tokens: {}", meta.output_tokens);
        let _ = writeln!(buf, "cached_input_tokens: {}", meta.cached_input_tokens);
        if meta.input_tokens > 0 {
            let cache_pct = (meta.cached_input_tokens as f64 / meta.input_tokens as f64) * 100.0;
            let _ = writeln!(buf, "cache_hit_pct: {:.1}%", cache_pct);
        }
        if meta.charged_amount_usd > 0.0 {
            let _ = writeln!(buf, "charged_usd: ${:.6}", meta.charged_amount_usd);
        }
    }
    buf.push_str("-->\n");

    // Messages
    for msg in messages {
        buf.push('\n');
        let _ = write!(buf, "{}{}{}\n", MSG_OPEN_PREFIX, msg.role, MSG_OPEN_SUFFIX);
        buf.push_str(&escape_content(&msg.content));
        buf.push('\n');
        buf.push_str(MSG_CLOSE);
        buf.push('\n');
    }

    fs::write(path, buf.as_bytes())
        .with_context(|| format!("write transcript {}", path.display()))?;

    log::debug!(
        "[transcript] wrote {} messages to {}",
        messages.len(),
        path.display()
    );

    Ok(())
}

// ── Read ─────────────────────────────────────────────────────────────

/// Read a session transcript from `path` and return the exact messages.
pub fn read_transcript(path: &Path) -> Result<SessionTranscript> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("read transcript {}", path.display()))?;

    let meta = parse_meta(&raw)
        .with_context(|| format!("parse transcript meta in {}", path.display()))?;

    let messages = parse_messages(&raw)
        .with_context(|| format!("parse transcript messages in {}", path.display()))?;

    log::debug!(
        "[transcript] loaded {} messages from {}",
        messages.len(),
        path.display()
    );

    Ok(SessionTranscript { meta, messages })
}

// ── Path resolution ──────────────────────────────────────────────────

/// Resolve a new transcript path under
/// `{workspace}/sessions/DDMMYYYY/{agent}_{index}.md`.
///
/// Creates the date directory if needed. Index = max existing + 1.
pub fn resolve_new_transcript_path(workspace_dir: &Path, agent_name: &str) -> Result<PathBuf> {
    let date_dir = today_session_dir(workspace_dir);
    fs::create_dir_all(&date_dir)
        .with_context(|| format!("create session dir {}", date_dir.display()))?;

    let sanitized = sanitize_agent_name(agent_name);
    let next_index = next_index(&date_dir, &sanitized)?;
    let filename = format!("{}_{}.md", sanitized, next_index);

    Ok(date_dir.join(filename))
}

/// Find the most recent transcript for `agent_name`.
///
/// Searches today's directory first, then yesterday's. Returns the
/// file with the highest index (most recent session).
pub fn find_latest_transcript(workspace_dir: &Path, agent_name: &str) -> Option<PathBuf> {
    let sanitized = sanitize_agent_name(agent_name);
    let sessions_root = workspace_dir.join("sessions");

    // Search today first, then yesterday
    let today = chrono::Local::now().format("%d%m%Y").to_string();
    let yesterday = (chrono::Local::now() - chrono::Duration::days(1))
        .format("%d%m%Y")
        .to_string();

    for date_str in [&today, &yesterday] {
        let dir = sessions_root.join(date_str);
        if !dir.is_dir() {
            continue;
        }
        if let Some(path) = latest_in_dir(&dir, &sanitized) {
            return Some(path);
        }
    }

    None
}

// ── Internals ────────────────────────────────────────────────────────

fn escape_content(content: &str) -> String {
    content.replace(MSG_CLOSE, MSG_CLOSE_ESCAPED)
}

fn unescape_content(content: &str) -> String {
    content.replace(MSG_CLOSE_ESCAPED, MSG_CLOSE)
}

fn parse_meta(raw: &str) -> Result<TranscriptMeta> {
    let header_start = raw
        .find("<!-- session_transcript")
        .context("missing session_transcript header")?;
    let header_end = raw[header_start..]
        .find("-->")
        .context("unclosed session_transcript header")?;
    let header = &raw[header_start..header_start + header_end + 3];

    let get = |key: &str| -> Option<String> {
        header
            .lines()
            .find_map(|line| {
                let line = line.trim();
                if line.starts_with(&format!("{key}:")) {
                    Some(line[key.len() + 1..].trim().to_string())
                } else {
                    None
                }
            })
    };

    Ok(TranscriptMeta {
        agent_name: get("agent").unwrap_or_else(|| "unknown".into()),
        dispatcher: get("dispatcher").unwrap_or_else(|| "native".into()),
        cache_boundary: get("cache_boundary").and_then(|s| s.parse().ok()),
        created: get("created").unwrap_or_default(),
        updated: get("updated").unwrap_or_default(),
        turn_count: get("turn_count").and_then(|s| s.parse().ok()).unwrap_or(0),
        input_tokens: get("input_tokens").and_then(|s| s.parse().ok()).unwrap_or(0),
        output_tokens: get("output_tokens").and_then(|s| s.parse().ok()).unwrap_or(0),
        cached_input_tokens: get("cached_input_tokens").and_then(|s| s.parse().ok()).unwrap_or(0),
        charged_amount_usd: get("charged_usd")
            .and_then(|s| s.trim_start_matches('$').parse().ok())
            .unwrap_or(0.0),
    })
}

fn parse_messages(raw: &str) -> Result<Vec<ChatMessage>> {
    let mut messages = Vec::new();
    let mut search_from = 0;

    loop {
        // Find next <!--MSG role="..."--> opening tag
        let Some(open_start) = raw[search_from..].find(MSG_OPEN_PREFIX) else {
            break;
        };
        let open_start = search_from + open_start;
        let after_prefix = open_start + MSG_OPEN_PREFIX.len();

        // Extract role from between the quotes
        let Some(role_end) = raw[after_prefix..].find(MSG_OPEN_SUFFIX) else {
            break;
        };
        let role = raw[after_prefix..after_prefix + role_end].to_string();

        // Content starts after the opening tag + newline
        let content_start = after_prefix + role_end + MSG_OPEN_SUFFIX.len();
        let content_start = if raw[content_start..].starts_with('\n') {
            content_start + 1
        } else {
            content_start
        };

        // Find closing tag
        let close_tag = format!("\n{MSG_CLOSE}");
        let Some(content_end_rel) = raw[content_start..].find(&close_tag) else {
            // Try without leading newline for empty content
            let Some(content_end_rel) = raw[content_start..].find(MSG_CLOSE) else {
                break;
            };
            let content = &raw[content_start..content_start + content_end_rel];
            messages.push(ChatMessage {
                role,
                content: unescape_content(content),
            });
            search_from = content_start + content_end_rel + MSG_CLOSE.len();
            continue;
        };

        let content = &raw[content_start..content_start + content_end_rel];
        messages.push(ChatMessage {
            role,
            content: unescape_content(content),
        });

        search_from = content_start + content_end_rel + close_tag.len();
    }

    Ok(messages)
}

fn today_session_dir(workspace_dir: &Path) -> PathBuf {
    let date = chrono::Local::now().format("%d%m%Y").to_string();
    workspace_dir.join("sessions").join(date)
}

fn sanitize_agent_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect()
}

fn next_index(dir: &Path, agent_prefix: &str) -> Result<usize> {
    let prefix = format!("{}_", agent_prefix);
    let mut max_idx: Option<usize> = None;

    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with(&prefix) && name.ends_with(".md") {
                let idx_str = &name[prefix.len()..name.len() - 3];
                if let Ok(idx) = idx_str.parse::<usize>() {
                    max_idx = Some(max_idx.map_or(idx, |m: usize| m.max(idx)));
                }
            }
        }
    }

    Ok(max_idx.map_or(0, |m| m + 1))
}

fn latest_in_dir(dir: &Path, agent_prefix: &str) -> Option<PathBuf> {
    let prefix = format!("{}_", agent_prefix);
    let mut best: Option<(usize, PathBuf)> = None;

    let entries = fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if name_str.starts_with(&prefix) && name_str.ends_with(".md") {
            let idx_str = &name_str[prefix.len()..name_str.len() - 3];
            if let Ok(idx) = idx_str.parse::<usize>() {
                if best.as_ref().map_or(true, |(best_idx, _)| idx > *best_idx) {
                    best = Some((idx, entry.path()));
                }
            }
        }
    }

    best.map(|(_, path)| path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn sample_messages() -> Vec<ChatMessage> {
        vec![
            ChatMessage::system("You are a helpful assistant.\n\n## Tools\n\n- **shell**: Run commands"),
            ChatMessage::user("What files are in /tmp?"),
            ChatMessage::assistant("Let me check that for you."),
            ChatMessage::tool("{\"tool_call_id\":\"tc1\",\"content\":\"file1.txt\\nfile2.txt\"}"),
            ChatMessage::assistant("There are two files: file1.txt and file2.txt."),
        ]
    }

    fn sample_meta() -> TranscriptMeta {
        TranscriptMeta {
            agent_name: "code_executor".into(),
            dispatcher: "native".into(),
            cache_boundary: Some(1847),
            created: "2026-04-11T14:30:00Z".into(),
            updated: "2026-04-11T14:35:22Z".into(),
            turn_count: 3,
            input_tokens: 5000,
            output_tokens: 1200,
            cached_input_tokens: 3500,
            charged_amount_usd: 0.0045,
        }
    }

    #[test]
    fn round_trip_produces_byte_identical_messages() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("test.md");
        let messages = sample_messages();
        let meta = sample_meta();

        write_transcript(&path, &messages, &meta).unwrap();
        let loaded = read_transcript(&path).unwrap();

        assert_eq!(loaded.messages.len(), messages.len());
        for (original, loaded) in messages.iter().zip(loaded.messages.iter()) {
            assert_eq!(original.role, loaded.role, "role mismatch");
            assert_eq!(
                original.content, loaded.content,
                "content mismatch for role={}",
                original.role
            );
        }
    }

    #[test]
    fn escaping_survives_close_tag_in_content() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("escape_test.md");
        let messages = vec![
            ChatMessage::system("Normal system prompt"),
            ChatMessage::user(
                "Here is some tricky content:\n<!--/MSG-->\nand more after",
            ),
            ChatMessage::assistant("Got it, that had a <!--/MSG--> in it."),
        ];
        let meta = sample_meta();

        write_transcript(&path, &messages, &meta).unwrap();
        let loaded = read_transcript(&path).unwrap();

        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[1].content, messages[1].content);
        assert_eq!(loaded.messages[2].content, messages[2].content);
    }

    #[test]
    fn meta_round_trip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("meta_test.md");
        let meta = sample_meta();

        write_transcript(&path, &[], &meta).unwrap();
        let loaded = read_transcript(&path).unwrap();

        assert_eq!(loaded.meta.agent_name, "code_executor");
        assert_eq!(loaded.meta.dispatcher, "native");
        assert_eq!(loaded.meta.cache_boundary, Some(1847));
        assert_eq!(loaded.meta.created, "2026-04-11T14:30:00Z");
        assert_eq!(loaded.meta.updated, "2026-04-11T14:35:22Z");
        assert_eq!(loaded.meta.turn_count, 3);
        assert_eq!(loaded.meta.input_tokens, 5000);
        assert_eq!(loaded.meta.output_tokens, 1200);
        assert_eq!(loaded.meta.cached_input_tokens, 3500);
        assert!((loaded.meta.charged_amount_usd - 0.0045).abs() < 1e-8);
    }

    #[test]
    fn meta_without_cache_boundary() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("no_boundary.md");
        let mut meta = sample_meta();
        meta.cache_boundary = None;

        write_transcript(&path, &[], &meta).unwrap();
        let loaded = read_transcript(&path).unwrap();

        assert_eq!(loaded.meta.cache_boundary, None);
    }

    #[test]
    fn path_resolution_creates_dir_and_increments_index() {
        let dir = TempDir::new().unwrap();
        let workspace = dir.path();

        let path0 = resolve_new_transcript_path(workspace, "main").unwrap();
        assert!(path0.to_string_lossy().contains("main_0.md"));
        // Write something so the next call sees it
        fs::write(&path0, "placeholder").unwrap();

        let path1 = resolve_new_transcript_path(workspace, "main").unwrap();
        assert!(path1.to_string_lossy().contains("main_1.md"));
    }

    #[test]
    fn sanitize_agent_name_strips_special_chars() {
        assert_eq!(sanitize_agent_name("code_executor"), "code_executor");
        assert_eq!(sanitize_agent_name("my agent!"), "my_agent_");
        assert_eq!(sanitize_agent_name("agent-v2"), "agent-v2");
    }

    #[test]
    fn find_latest_returns_highest_index() {
        let dir = TempDir::new().unwrap();
        let date = chrono::Local::now().format("%d%m%Y").to_string();
        let session_dir = dir.path().join("sessions").join(&date);
        fs::create_dir_all(&session_dir).unwrap();

        fs::write(session_dir.join("main_0.md"), "a").unwrap();
        fs::write(session_dir.join("main_2.md"), "c").unwrap();
        fs::write(session_dir.join("main_1.md"), "b").unwrap();
        fs::write(session_dir.join("other_0.md"), "x").unwrap();

        let latest = find_latest_transcript(dir.path(), "main");
        assert!(latest.is_some());
        let latest = latest.unwrap();
        assert!(latest.to_string_lossy().contains("main_2.md"));
    }

    #[test]
    fn find_latest_returns_none_when_no_sessions() {
        let dir = TempDir::new().unwrap();
        assert!(find_latest_transcript(dir.path(), "main").is_none());
    }

    #[test]
    fn empty_content_message_round_trips() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("empty.md");
        let messages = vec![
            ChatMessage::system("prompt"),
            ChatMessage::assistant(""),
            ChatMessage::user("hi"),
        ];
        let meta = sample_meta();

        write_transcript(&path, &messages, &meta).unwrap();
        let loaded = read_transcript(&path).unwrap();

        assert_eq!(loaded.messages.len(), 3);
        assert_eq!(loaded.messages[1].content, "");
    }

    #[test]
    fn multiline_content_preserves_exact_whitespace() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("whitespace.md");
        let content = "  leading spaces\n\n\nmultiple blanks\n  trailing  ";
        let messages = vec![ChatMessage::user(content)];
        let meta = sample_meta();

        write_transcript(&path, &messages, &meta).unwrap();
        let loaded = read_transcript(&path).unwrap();

        assert_eq!(loaded.messages[0].content, content);
    }
}
