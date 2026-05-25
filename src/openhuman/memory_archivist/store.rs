//! Disk-backed archivist store.
//!
//! Layout:
//! ```text
//! <content_root>/episodic/<session_id>/<seq:06>.md
//! ```
//!
//! Writes use the same atomic tempfile+rename contract as
//! `memory_store::content::atomic::write_if_new`, with one important
//! difference: we want to *append* turns to a session, so the seq is
//! computed from the existing directory contents on each call.

use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use crate::openhuman::config::Config;
use crate::openhuman::memory_archivist::types::ArchivedTurn;
use crate::openhuman::memory_store::content::atomic::write_if_new;

const EPISODIC_DIR: &str = "episodic";

fn session_dir(config: &Config, session_id: &str) -> PathBuf {
    config
        .memory_tree_content_root()
        .join(EPISODIC_DIR)
        .join(sanitize_session(session_id))
}

fn sanitize_session(s: &str) -> String {
    s.chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

fn next_seq(dir: &Path) -> u32 {
    let mut max = -1i64;
    if let Ok(rd) = fs::read_dir(dir) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let s = name.to_string_lossy();
            if let Some(stem) = s.strip_suffix(".md") {
                if let Ok(n) = stem.parse::<i64>() {
                    if n > max {
                        max = n;
                    }
                }
            }
        }
    }
    (max + 1) as u32
}

fn compose_turn(turn: &ArchivedTurn) -> String {
    let mut yaml = String::from("---\n");
    yaml.push_str(&format!("session_id: {}\n", turn.session_id));
    yaml.push_str(&format!("seq: {}\n", turn.seq));
    yaml.push_str(&format!("timestamp_ms: {}\n", turn.timestamp_ms));
    yaml.push_str(&format!("role: {}\n", turn.role));
    yaml.push_str(&format!("cost_microdollars: {}\n", turn.cost_microdollars));
    if let Some(lesson) = turn.lesson.as_ref() {
        yaml.push_str(&format!("lesson: {}\n", yaml_escape(lesson)));
    }
    if let Some(tc) = turn.tool_calls_json.as_ref() {
        yaml.push_str(&format!("tool_calls_json: {}\n", yaml_escape(tc)));
    }
    yaml.push_str("---\n\n");
    yaml.push_str(&turn.content);
    if !turn.content.ends_with('\n') {
        yaml.push('\n');
    }
    yaml
}

fn yaml_escape(s: &str) -> String {
    // Quote any string that contains characters with YAML semantic meaning.
    // Simple double-quote escaping is good enough for these single-line
    // front-matter fields.
    let needs_quote = s
        .chars()
        .any(|c| matches!(c, ':' | '#' | '\n' | '"' | '\'' | '[' | ']' | '{' | '}'));
    if needs_quote {
        format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// Append a turn to its session's archive. Returns the assigned `seq`.
///
/// `turn.seq` is ignored on input — the on-disk directory is the source of
/// truth and the returned `ArchivedTurn` carries the actually-assigned seq.
pub fn record_turn(config: &Config, mut turn: ArchivedTurn) -> Result<ArchivedTurn> {
    let dir = session_dir(config, &turn.session_id);
    fs::create_dir_all(&dir).with_context(|| format!("failed to mkdir -p {}", dir.display()))?;
    turn.seq = next_seq(&dir);
    let path = dir.join(format!("{:06}.md", turn.seq));
    let bytes = compose_turn(&turn).into_bytes();
    write_if_new(&path, &bytes)
        .with_context(|| format!("failed to write episodic turn {}", path.display()))?;
    log::debug!(
        "[memory_archivist] recorded session={} seq={} role={} bytes={}",
        turn.session_id,
        turn.seq,
        turn.role,
        bytes.len()
    );
    Ok(turn)
}

/// Read every turn for `session_id`, sorted by seq ascending.
pub fn session_entries(config: &Config, session_id: &str) -> Result<Vec<ArchivedTurn>> {
    let dir = session_dir(config, session_id);
    if !dir.exists() {
        return Ok(Vec::new());
    }
    let mut files: Vec<(u32, PathBuf)> = fs::read_dir(&dir)
        .with_context(|| format!("failed to read_dir {}", dir.display()))?
        .filter_map(|e| e.ok())
        .filter_map(|e| {
            let name = e.file_name();
            let s = name.to_string_lossy();
            let stem = s.strip_suffix(".md")?;
            let seq = stem.parse::<u32>().ok()?;
            Some((seq, e.path()))
        })
        .collect();
    files.sort_by_key(|(seq, _)| *seq);
    let mut out = Vec::with_capacity(files.len());
    for (_, path) in files {
        let bytes =
            fs::read(&path).with_context(|| format!("failed to read {}", path.display()))?;
        let text = String::from_utf8_lossy(&bytes);
        if let Some(turn) = parse_turn(&text) {
            out.push(turn);
        }
    }
    Ok(out)
}

fn parse_turn(text: &str) -> Option<ArchivedTurn> {
    let body_start = text.strip_prefix("---\n")?;
    let end = body_start.find("\n---\n")?;
    let (yaml, rest) = body_start.split_at(end);
    let body = rest.strip_prefix("\n---\n").unwrap_or(rest).to_string();
    let mut turn = ArchivedTurn::default();
    for line in yaml.lines() {
        let Some((k, v)) = line.split_once(':') else {
            continue;
        };
        let k = k.trim();
        let v = v.trim();
        let v_unquoted = v
            .strip_prefix('"')
            .and_then(|s| s.strip_suffix('"'))
            .map(|s| s.replace("\\\"", "\"").replace("\\\\", "\\"))
            .unwrap_or_else(|| v.to_string());
        match k {
            "session_id" => turn.session_id = v_unquoted,
            "seq" => turn.seq = v_unquoted.parse().unwrap_or(0),
            "timestamp_ms" => turn.timestamp_ms = v_unquoted.parse().unwrap_or(0),
            "role" => turn.role = v_unquoted,
            "cost_microdollars" => turn.cost_microdollars = v_unquoted.parse().unwrap_or(0),
            "lesson" => turn.lesson = Some(v_unquoted),
            "tool_calls_json" => turn.tool_calls_json = Some(v_unquoted),
            _ => {}
        }
    }
    // Strip the single blank line compose() writes between the closing
    // `---\n` and the body, then trim trailing newline. Internal blank
    // lines in the body are preserved.
    turn.content = body
        .strip_prefix('\n')
        .unwrap_or(body.as_str())
        .trim_end()
        .to_string();
    Some(turn)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    fn turn(session: &str, role: &str, content: &str) -> ArchivedTurn {
        ArchivedTurn {
            session_id: session.into(),
            seq: 0,
            timestamp_ms: 1_700_000_000_000,
            role: role.into(),
            content: content.into(),
            lesson: None,
            tool_calls_json: None,
            cost_microdollars: 0,
        }
    }

    #[test]
    fn round_trip_single_turn() {
        let (_tmp, cfg) = test_config();
        let stored = record_turn(&cfg, turn("s1", "user", "hello world")).unwrap();
        assert_eq!(stored.seq, 0);
        let read = session_entries(&cfg, "s1").unwrap();
        assert_eq!(read.len(), 1);
        assert_eq!(read[0].content, "hello world");
        assert_eq!(read[0].role, "user");
        assert_eq!(read[0].session_id, "s1");
        assert_eq!(read[0].seq, 0);
    }

    #[test]
    fn append_increments_seq() {
        let (_tmp, cfg) = test_config();
        let a = record_turn(&cfg, turn("s1", "user", "one")).unwrap();
        let b = record_turn(&cfg, turn("s1", "assistant", "two")).unwrap();
        let c = record_turn(&cfg, turn("s1", "user", "three")).unwrap();
        assert_eq!((a.seq, b.seq, c.seq), (0, 1, 2));
        let read = session_entries(&cfg, "s1").unwrap();
        assert_eq!(
            read.iter().map(|t| t.seq).collect::<Vec<_>>(),
            vec![0, 1, 2]
        );
        assert_eq!(read[1].role, "assistant");
        assert_eq!(read[2].content, "three");
    }

    #[test]
    fn missing_session_returns_empty() {
        let (_tmp, cfg) = test_config();
        assert!(session_entries(&cfg, "never").unwrap().is_empty());
    }

    #[test]
    fn preserves_lesson_and_tool_calls() {
        let (_tmp, cfg) = test_config();
        let mut t = turn("s1", "assistant", "did the thing");
        t.lesson = Some("be careful with X: it bites".into());
        t.tool_calls_json = Some(r#"[{"name":"bash","args":{"cmd":"ls"}}]"#.into());
        t.cost_microdollars = 1234;
        record_turn(&cfg, t.clone()).unwrap();
        let read = session_entries(&cfg, "s1").unwrap();
        assert_eq!(
            read[0].lesson.as_deref(),
            Some("be careful with X: it bites")
        );
        assert_eq!(
            read[0].tool_calls_json.as_deref(),
            Some(r#"[{"name":"bash","args":{"cmd":"ls"}}]"#)
        );
        assert_eq!(read[0].cost_microdollars, 1234);
    }

    #[test]
    fn distinct_sessions_dont_mix() {
        let (_tmp, cfg) = test_config();
        record_turn(&cfg, turn("a", "user", "hi a")).unwrap();
        record_turn(&cfg, turn("b", "user", "hi b")).unwrap();
        record_turn(&cfg, turn("a", "user", "more a")).unwrap();
        let a = session_entries(&cfg, "a").unwrap();
        let b = session_entries(&cfg, "b").unwrap();
        assert_eq!(a.len(), 2);
        assert_eq!(b.len(), 1);
        assert_eq!(b[0].content, "hi b");
    }
}
