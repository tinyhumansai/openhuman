//! YAML front-matter + body composition for chunk `.md` files.
//!
//! Each file written to disk has the form:
//! ```text
//! ---
//! source_kind: chat
//! source_id: slack:#eng
//! seq: 0
//! owner: alice@example.com
//! timestamp: 2026-04-28T10:00:00Z
//! time_range_start: 2026-04-28T10:00:00Z
//! time_range_end: 2026-04-28T10:05:00Z
//! source_ref: slack://permalink/…
//! tags:
//!   - person/Alice-Smith
//!   - project/Phoenix
//! ---
//! ## 2026-04-28T10:00:00Z — alice
//! Message body here.
//! ```
//!
//! **SHA-256 is computed over the body bytes only** (everything after `---\n`
//! on the second delimiter line). This allows tags to be rewritten atomically
//! without invalidating the content hash.

use crate::openhuman::memory::tree::types::Chunk;

/// Compose the full file content (front-matter + body) for `chunk`.
///
/// Returns `(full_file_bytes, body_bytes)`. The caller writes `full_file_bytes`
/// to disk; `body_bytes` is what the SHA-256 is computed over.
pub fn compose_chunk_file(chunk: &Chunk) -> (Vec<u8>, Vec<u8>) {
    let front_matter = build_front_matter(chunk);
    let body = chunk.content.as_bytes().to_vec();

    let mut full = Vec::with_capacity(front_matter.len() + body.len());
    full.extend_from_slice(&front_matter);
    full.extend_from_slice(&body);

    (full, body)
}

/// Build the YAML front-matter block (including delimiters) as UTF-8 bytes.
fn build_front_matter(chunk: &Chunk) -> Vec<u8> {
    let meta = &chunk.metadata;
    let ts = meta.timestamp.to_rfc3339();
    let ts_start = meta.time_range.0.to_rfc3339();
    let ts_end = meta.time_range.1.to_rfc3339();

    let mut fm = String::new();
    fm.push_str("---\n");
    fm.push_str(&format!("source_kind: {}\n", meta.source_kind.as_str()));
    // Escape backslashes and quotes in source_id for safety.
    fm.push_str(&format!("source_id: {}\n", yaml_scalar(&meta.source_id)));
    fm.push_str(&format!("seq: {}\n", chunk.seq_in_source));
    fm.push_str(&format!("owner: {}\n", yaml_scalar(&meta.owner)));
    fm.push_str(&format!("timestamp: {ts}\n"));
    fm.push_str(&format!("time_range_start: {ts_start}\n"));
    fm.push_str(&format!("time_range_end: {ts_end}\n"));

    if let Some(ref sr) = meta.source_ref {
        fm.push_str(&format!("source_ref: {}\n", yaml_scalar(&sr.value)));
    }

    if meta.tags.is_empty() {
        fm.push_str("tags: []\n");
    } else {
        fm.push_str("tags:\n");
        for tag in &meta.tags {
            fm.push_str(&format!("  - {}\n", yaml_scalar(tag)));
        }
    }

    fm.push_str("---\n");
    fm.into_bytes()
}

/// Rewrite the `tags:` block in an existing file's front-matter, replacing it
/// with the new tag list while leaving the body unchanged.
///
/// Returns the new full file bytes. Errors if the front-matter delimiters
/// cannot be found.
pub fn rewrite_tags(file_bytes: &[u8], new_tags: &[String]) -> Result<Vec<u8>, String> {
    let content =
        std::str::from_utf8(file_bytes).map_err(|e| format!("file is not valid UTF-8: {e}"))?;

    let (front_matter, body) = split_front_matter(content)
        .ok_or_else(|| "cannot find front-matter delimiters".to_string())?;

    // Rewrite tags: block in the front-matter string.
    let new_fm = replace_tags_in_front_matter(front_matter, new_tags)?;

    let mut out = Vec::with_capacity(new_fm.len() + body.len() + 4);
    out.extend_from_slice(new_fm.as_bytes());
    out.extend_from_slice(body.as_bytes());
    Ok(out)
}

/// Replace the `tags:` stanza in a front-matter string. Returns the new
/// front-matter string (delimiters preserved).
fn replace_tags_in_front_matter(fm: &str, new_tags: &[String]) -> Result<String, String> {
    // Build the replacement block.
    let replacement = if new_tags.is_empty() {
        "tags: []".to_string()
    } else {
        let mut s = "tags:".to_string();
        for tag in new_tags {
            s.push('\n');
            s.push_str(&format!("  - {}", yaml_scalar(tag)));
        }
        s
    };

    // Locate the `tags:` key and consume through the block.
    let lines: Vec<&str> = fm.lines().collect();
    let mut out_lines: Vec<&str> = Vec::new();
    let mut i = 0;
    let mut found = false;

    while i < lines.len() {
        let line = lines[i];
        if line == "tags: []" || line == "tags:" {
            found = true;
            // Skip all subsequent lines that are tag list items (start with `  - `).
            // The replacement will be inserted wholesale.
            i += 1;
            if line == "tags:" {
                while i < lines.len() && lines[i].starts_with("  - ") {
                    i += 1;
                }
            }
            // We've consumed the old block; we'll append replacement after the loop.
            continue;
        }
        out_lines.push(line);
        i += 1;
    }

    if !found {
        return Err("tags: key not found in front-matter".to_string());
    }

    // Rebuild: all non-tag lines + replacement + closing `---`.
    // Front-matter was: `---\n...\ntags: ...\n---\n`
    // After loop, out_lines has everything except the tags block.
    // Insert replacement before the closing `---`.
    let closing = out_lines
        .iter()
        .rposition(|l| *l == "---")
        .unwrap_or(out_lines.len());

    let mut result_lines: Vec<String> =
        out_lines[..closing].iter().map(|l| l.to_string()).collect();
    result_lines.push(replacement);
    result_lines.push("---".to_string());

    let mut result = result_lines.join("\n");
    result.push('\n');
    Ok(result)
}

/// Split a file into `(front_matter, body)` at the second `---` delimiter.
///
/// Returns `None` if the file does not have the expected `---\n...\n---\n` form.
pub fn split_front_matter(content: &str) -> Option<(&str, &str)> {
    // The file must start with `---\n`.
    if !content.starts_with("---\n") {
        return None;
    }
    // Find the closing `---` line (must be `---` alone on a line after the first line).
    let rest = &content[4..]; // skip the opening `---\n`
    let close_idx = rest.find("\n---\n").or_else(|| {
        // Could be at the very end (no body).
        rest.strip_suffix("\n---").map(|r| r.len())
    })?;
    let fm_end = 4 + close_idx + 5; // include `\n---\n`
    Some((&content[..fm_end], &content[fm_end..]))
}

/// Format a string as an unquoted YAML scalar when safe, or as a
/// double-quoted string when it contains special characters.
///
/// We conservatively quote strings containing `:`, `#`, `[`, `]`, `{`, `}`,
/// `"`, `'`, `\`, leading/trailing whitespace, or that start with special
/// YAML indicator characters.
fn yaml_scalar(s: &str) -> String {
    let needs_quoting = s.is_empty()
        || s.trim() != s
        || s.starts_with(|c: char| {
            matches!(
                c,
                '&' | '*' | '?' | '|' | '-' | '<' | '>' | '=' | '!' | '%' | '@' | '`'
            )
        })
        || s.contains(|c: char| matches!(c, ':' | '#' | '[' | ']' | '{' | '}' | '"' | '\''));

    if needs_quoting {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        format!("\"{escaped}\"")
    } else {
        s.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::types::{Metadata, SourceKind, SourceRef};
    use chrono::TimeZone;

    fn sample_chunk() -> Chunk {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: "abc123".into(),
            content: "## 2026-01-01T00:00:00Z — alice\nhello world".into(),
            metadata: Metadata {
                source_kind: SourceKind::Chat,
                source_id: "slack:#eng".into(),
                owner: "alice@example.com".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["person/Alice".into(), "org/Acme".into()],
                source_ref: Some(SourceRef::new("slack://m1".to_string())),
            },
            token_count: 10,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn compose_produces_front_matter_and_body() {
        let chunk = sample_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        assert!(full_str.starts_with("---\n"), "must start with ---");
        assert!(full_str.contains("source_kind: chat"));
        assert!(full_str.contains("source_id: \"slack:#eng\""));
        assert!(full_str.contains("seq: 0"));
        assert!(full_str.contains("tags:"));
        assert!(full_str.contains("  - person/Alice"));
        assert!(full_str.ends_with("hello world"));
        assert_eq!(
            body,
            b"## 2026-01-01T00:00:00Z \xe2\x80\x94 alice\nhello world"
        );
    }

    #[test]
    fn split_front_matter_round_trips() {
        let chunk = sample_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        let (fm, b) = split_front_matter(full_str).expect("split must succeed");
        assert!(fm.starts_with("---\n"));
        assert!(fm.ends_with("---\n"));
        assert_eq!(b.as_bytes(), body.as_slice());
    }

    #[test]
    fn rewrite_tags_preserves_body() {
        let chunk = sample_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let new_tags = vec!["person/Bob".into(), "project/Phoenix".into()];
        let rewritten = rewrite_tags(&full, &new_tags).unwrap();
        let rewritten_str = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten_str.contains("  - person/Bob"));
        assert!(!rewritten_str.contains("  - person/Alice"));
        // Body must be unchanged.
        assert!(rewritten_str.ends_with(std::str::from_utf8(&body).unwrap()));
    }

    #[test]
    fn rewrite_tags_empty_list() {
        let chunk = sample_chunk();
        let (full, _) = compose_chunk_file(&chunk);
        let rewritten = rewrite_tags(&full, &[]).unwrap();
        let s = std::str::from_utf8(&rewritten).unwrap();
        assert!(s.contains("tags: []"));
        assert!(!s.contains("  - person/"));
    }

    #[test]
    fn yaml_scalar_quotes_special_characters() {
        assert_eq!(yaml_scalar("slack:#eng"), "\"slack:#eng\"");
        assert_eq!(yaml_scalar("hello world"), "hello world");
        assert_eq!(yaml_scalar(""), "\"\"");
    }
}
