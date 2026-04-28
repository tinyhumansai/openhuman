//! Content-file path generation.
//!
//! Each chunk body is stored as a `.md` file under `<content_root>/`. The path
//! structure depends on the source kind:
//!
//! ```text
//! Email:    <content_root>/email/<sender_slug>/<thread_slug>/<chunk_id>.md
//! Chat:     <content_root>/chat/<source_slug>/<chunk_id>.md
//! Document: <content_root>/document/<source_slug>/<chunk_id>.md
//! ```
//!
//! Email paths parse the `source_id` as `gmail:{sender}:{thread_id}` and
//! slugify each segment independently, giving each thread its own directory
//! and eliminating cross-thread path collisions.
//!
//! Paths are stored in SQLite as **relative** strings with forward slashes so
//! they remain valid regardless of where the workspace is mounted.

use std::path::{Path, PathBuf};

/// Build the relative content path for a chunk, using forward slashes.
///
/// Path layout depends on source_kind:
/// - Email:    `"email/<sender_slug>/<thread_slug>/<chunk_id>.md"`
///   Parses `source_id` as `gmail:{sender}:{thread_id}` (three colon-separated
///   parts) and slugifies sender and thread independently.
///   If the parse fails (legacy or malformed source_id), falls through to the
///   chat/document layout using `slugify_source_id(source_id)` as the single
///   group key.
/// - Chat:     `"chat/<source_slug>/<chunk_id>.md"`
/// - Document: `"document/<source_slug>/<chunk_id>.md"`
///
/// `chunk_id` — the deterministic content hash produced by `types::chunk_id`.
pub fn chunk_rel_path(source_kind: &str, source_id: &str, chunk_id: &str) -> String {
    match source_kind {
        "email" => {
            // Expected format: "gmail:{sender}:{thread_id}"
            // Split on ':' — exactly 3 parts required.
            let parts: Vec<&str> = source_id.splitn(3, ':').collect();
            if parts.len() == 3 && !parts[1].is_empty() {
                let sender_slug = slugify_source_id(parts[1]);
                let thread_slug = slugify_source_id(parts[2]);
                format!("email/{}/{}/{}.md", sender_slug, thread_slug, chunk_id)
            } else {
                // Malformed / legacy source_id — fall back to flat layout.
                log::debug!(
                    "[content_store::paths] email source_id has unexpected format, falling back to flat layout: {:?}",
                    source_id
                );
                let slug = slugify_source_id(source_id);
                format!("email/{}/{}.md", slug, chunk_id)
            }
        }
        _ => {
            // Chat, Document, and any future kinds use a 3-level layout.
            let slug = slugify_source_id(source_id);
            format!("{}/{}/{}.md", source_kind, slug, chunk_id)
        }
    }
}

/// Build the absolute on-disk path for a chunk given the content root.
pub fn chunk_abs_path(
    content_root: &Path,
    source_kind: &str,
    source_id: &str,
    chunk_id: &str,
) -> PathBuf {
    let rel = chunk_rel_path(source_kind, source_id, chunk_id);
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
/// - `_` is preserved anywhere in the string (interior underscores are kept)
/// - truncate to 120 characters
pub fn slugify_source_id(source_id: &str) -> String {
    let lower = source_id.to_lowercase();
    let mut out = String::with_capacity(lower.len().min(120));
    let mut last_dash = true; // avoids leading dash; also suppresses leading underscore runs
    let mut pending_underscore = false; // deferred `_` to avoid leading underscore

    for ch in lower.chars() {
        if ch == '_' {
            // Defer underscores — emit only if we have already emitted a
            // non-separator character (so `_solo_` becomes `_solo_` once the
            // `s` is emitted, but a leading `_` is dropped).
            if !last_dash {
                // We have real content before this, so emit the underscore now.
                pending_underscore = true;
            }
            // If last_dash is true (nothing emitted yet), silently skip.
        } else if ch.is_ascii_alphanumeric() {
            if pending_underscore {
                out.push('_');
                pending_underscore = false;
            }
            out.push(ch);
            last_dash = false;
        } else {
            // Non-alphanumeric, non-underscore → convert to `-`.
            pending_underscore = false; // drop any pending underscore before a dash
            if !last_dash {
                out.push('-');
                last_dash = true;
            }
        }
    }
    // trailing underscore: drop it (trim trailing separators).
    // trim trailing dash
    let trimmed = out.trim_end_matches('-');
    // also trim any trailing underscore
    let trimmed = trimmed.trim_end_matches('_');
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

    // ─── slugify tests ────────────────────────────────────────────────────────

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
    fn slugify_preserves_interior_underscore() {
        // `_solo_` has a leading and trailing underscore; only the interior
        // `solo` + the part after should survive.  When used as a thread key
        // it arrives as the whole string `_solo_`.
        // Leading `_` is stripped (it's treated like a leading dash),
        // trailing `_` is stripped; interior `_` is preserved when sandwiched
        // between alphanumeric characters.
        let s = slugify_source_id("_solo_");
        // "solo" — both outer underscores trimmed, interior underscore has
        // nothing on the right so it's also trailing and trimmed.
        assert_eq!(s, "solo");
    }

    #[test]
    fn slugify_preserves_interior_underscore_between_chars() {
        // `foo_bar` — interior underscore stays.
        assert_eq!(slugify_source_id("foo_bar"), "foo_bar");
    }

    // ─── chunk_rel_path tests ─────────────────────────────────────────────────

    #[test]
    fn email_path_round_trips() {
        let p = chunk_rel_path("email", "gmail:alice@example.com:t1", "abc123");
        assert_eq!(p, "email/alice-example-com/t1/abc123.md");
    }

    #[test]
    fn email_solo_path() {
        // `_solo_` slugifies to `solo` (outer underscores stripped).
        let p = chunk_rel_path("email", "gmail:noreply@github.com:_solo_", "def456");
        assert_eq!(p, "email/noreply-github-com/solo/def456.md");
    }

    #[test]
    fn email_malformed_source_id_falls_back_to_flat_layout() {
        // Malformed: only 2 colon-separated parts (no thread segment).
        let p = chunk_rel_path("email", "legacyid", "xyz");
        // Falls back to email/<slug>/<chunk_id>.md
        assert!(p.starts_with("email/"), "must remain under email/");
        assert!(p.ends_with("/xyz.md"), "chunk_id must be the filename");
        // Must not panic.
    }

    #[test]
    fn chat_path() {
        let p = chunk_rel_path("chat", "slack:#eng", "xyz789");
        assert_eq!(p, "chat/slack-eng/xyz789.md");
    }

    #[test]
    fn document_path() {
        let p = chunk_rel_path("document", "doc:notes.md", "uvw");
        assert_eq!(p, "document/doc-notes-md/uvw.md");
    }

    #[test]
    fn chunk_abs_path_uses_os_separator() {
        use std::path::Path;
        let root = Path::new("/workspace/content");
        let abs = chunk_abs_path(root, "email", "gmail:alice@x.com:t1", "abc");
        assert!(abs.starts_with(root));
        assert!(abs.ends_with("abc.md"));
    }
}
