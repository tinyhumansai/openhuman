//! Content-file path generation.
//!
//! Each chunk body is stored as a `.md` file under `<content_root>/`. The path
//! structure is:
//!
//! ```text
//! <content_root>/<source_kind>/<slugified_source_id>/<seq_in_source>.md
//! ```
//!
//! Paths are stored in SQLite as **relative** strings with forward slashes so
//! they remain valid regardless of where the workspace is mounted.

use std::path::{Path, PathBuf};

/// Build the relative content path for a chunk, using forward slashes.
///
/// `source_kind`  — e.g. `"chat"`, `"email"`, `"document"`
/// `source_id`    — raw source identifier (may contain `:`, `/`, spaces, …)
/// `seq`          — the chunk's `seq_in_source`
///
/// The returned string is suitable for storing in `mem_tree_chunks.content_path`.
pub fn chunk_rel_path(source_kind: &str, source_id: &str, seq: u32) -> String {
    let slug = slugify_source_id(source_id);
    format!("{}/{}/{}.md", source_kind, slug, seq)
}

/// Build the absolute on-disk path for a chunk given the content root.
pub fn chunk_abs_path(
    content_root: &Path,
    source_kind: &str,
    source_id: &str,
    seq: u32,
) -> PathBuf {
    let rel = chunk_rel_path(source_kind, source_id, seq);
    // Convert forward-slash relative path to OS-native path.
    let mut abs = content_root.to_path_buf();
    for component in rel.split('/') {
        abs.push(component);
    }
    abs
}

/// Convert a raw `source_id` (e.g. `"slack:#general"`, `"gmail:thread/abc"`)
/// into a filesystem-safe slug using only `[a-z0-9_-]` characters.
///
/// Rules:
/// - lowercase the whole string
/// - replace any character outside `[a-z0-9_-]` with `-`
/// - collapse consecutive `-` to one
/// - trim leading/trailing `-`
/// - truncate to 120 characters
pub fn slugify_source_id(source_id: &str) -> String {
    let lower = source_id.to_lowercase();
    let mut out = String::with_capacity(lower.len().min(120));
    let mut last_dash = true; // avoids leading dash
    for ch in lower.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            out.push(ch);
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    // trim trailing dash
    let trimmed = out.trim_end_matches('-');
    let truncated = truncate_at_char(trimmed, 120);
    if truncated.is_empty() {
        "unknown".to_string()
    } else {
        truncated.to_string()
    }
}

/// Truncate `s` to at most `max_chars` Unicode code points.
fn truncate_at_char(s: &str, max_chars: usize) -> &str {
    match s.char_indices().nth(max_chars) {
        Some((idx, _)) => &s[..idx],
        None => s,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slugify_slack_channel() {
        assert_eq!(slugify_source_id("slack:#general"), "slack-general");
    }

    #[test]
    fn slugify_gmail_thread() {
        assert_eq!(
            slugify_source_id("gmail:thread/abc-123"),
            "gmail-thread-abc-123"
        );
    }

    #[test]
    fn slugify_collapses_consecutive_separators() {
        assert_eq!(slugify_source_id("foo::bar"), "foo-bar");
    }

    #[test]
    fn slugify_uppercase_lowercased() {
        assert_eq!(slugify_source_id("Slack:ABC"), "slack-abc");
    }

    #[test]
    fn slugify_empty_falls_back_to_unknown() {
        assert_eq!(slugify_source_id(""), "unknown");
        assert_eq!(slugify_source_id(":::"), "unknown");
    }

    #[test]
    fn slugify_truncates_at_120_chars() {
        let long = "a".repeat(200);
        let slug = slugify_source_id(&long);
        assert_eq!(slug.len(), 120);
    }

    #[test]
    fn chunk_rel_path_format() {
        let p = chunk_rel_path("chat", "slack:#eng", 3);
        assert_eq!(p, "chat/slack-eng/3.md");
    }

    #[test]
    fn chunk_abs_path_uses_os_separator() {
        use std::path::Path;
        let root = Path::new("/workspace/content");
        let abs = chunk_abs_path(root, "email", "gmail:t1", 0);
        assert!(abs.starts_with(root));
        assert!(abs.ends_with("0.md"));
    }
}
