//! Content-file path generation.
//!
//! Each chunk body is stored as a `.md` file under `<content_root>/`. The path
//! structure depends on the source kind:
//!
//! ```text
//! Email:    <content_root>/email/<participants_slug>/<chunk_id>.md
//! Chat:     <content_root>/chat/<source_slug>/<chunk_id>.md
//! Document: <content_root>/document/<source_slug>/<chunk_id>.md
//! ```
//!
//! Email paths parse `source_id` as `gmail:{participants}` where `participants`
//! is `addr1|addr2|...` (sorted, deduped, lowercased bare emails). The
//! participants string is slugified as a whole (pipe and `@` both become `-`)
//! to produce a single directory level, giving one folder per unique
//! conversation set.
//!
//! Paths are stored in SQLite as **relative** strings with forward slashes so
//! they remain valid regardless of where the workspace is mounted.

use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};

/// Which kind of summary tree a summary belongs to. Determines the top-level
/// directory under `<content_root>/summaries/`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SummaryTreeKind {
    /// Per-source-tree summary. Layout: `summaries/source/<scope_slug>/L<level>/<id>.md`
    Source,
    /// Global digest tree. Layout: `summaries/global/<yyyy-mm-dd>/L<level>/<id>.md`
    Global,
    /// Per-topic (entity) tree. Layout: `summaries/topic/<scope_slug>/L<level>/<id>.md`
    Topic,
}

/// Build the relative content path for a summary, using forward slashes.
///
/// Path layout depends on tree_kind:
/// - Source: `"summaries/source/<scope_slug>/L<level>/<summary_filename>.md"`
/// - Global: `"summaries/global/<yyyy-mm-dd>/L<level>/<summary_filename>.md"`
///   Panics (via `expect`) if `date_for_global` is `None` for `SummaryTreeKind::Global`.
/// - Topic:  `"summaries/topic/<scope_slug>/L<level>/<summary_filename>.md"`
///
/// `scope_slug` must already be slugified by the caller (use [`slugify_source_id`] or
/// a per-kind variant). A trailing `.md` on `summary_id` is stripped if present.
///
/// The `summary_id` is sanitized into a filesystem-safe filename by replacing
/// characters illegal on Windows (`:`, `\`, `*`, `?`, `"`, `<`, `>`, `|`) with `-`.
pub fn summary_rel_path(
    tree_kind: SummaryTreeKind,
    scope_slug: &str,
    level: u32,
    summary_id: &str,
    date_for_global: Option<DateTime<Utc>>,
) -> String {
    // Strip a trailing `.md` from summary_id if accidentally included.
    let id = summary_id.strip_suffix(".md").unwrap_or(summary_id);
    // Sanitize to a cross-platform filename (colons are illegal on Windows NTFS).
    let filename = sanitize_filename(id);

    match tree_kind {
        SummaryTreeKind::Source => {
            format!("summaries/source/{}/L{}/{}.md", scope_slug, level, filename)
        }
        SummaryTreeKind::Global => {
            let date = date_for_global
                .expect("date_for_global is required for SummaryTreeKind::Global")
                .format("%Y-%m-%d");
            format!("summaries/global/{}/L{}/{}.md", date, level, filename)
        }
        SummaryTreeKind::Topic => {
            format!("summaries/topic/{}/L{}/{}.md", scope_slug, level, filename)
        }
    }
}

/// Replace characters that are illegal in filenames on Windows NTFS with `-`.
///
/// Illegal characters: `\`, `/`, `:`, `*`, `?`, `"`, `<`, `>`, `|`.
/// (Forward slash is not replaced since `summary_id` should not contain path
/// separators, but we sanitize it anyway for safety.)
fn sanitize_filename(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|' => '-',
            c => c,
        })
        .collect()
}

/// Build the absolute on-disk path for a summary given the content root.
pub fn summary_abs_path(
    content_root: &Path,
    tree_kind: SummaryTreeKind,
    scope_slug: &str,
    level: u32,
    summary_id: &str,
    date_for_global: Option<DateTime<Utc>>,
) -> PathBuf {
    let rel = summary_rel_path(tree_kind, scope_slug, level, summary_id, date_for_global);
    let mut abs = content_root.to_path_buf();
    for component in rel.split('/') {
        abs.push(component);
    }
    abs
}

/// Build the relative content path for a chunk, using forward slashes.
///
/// Path layout depends on source_kind:
/// - Email:    `"email/<participants_slug>/<chunk_id>.md"`
///   Parses `source_id` as `gmail:{participants}` (two colon-separated parts)
///   where `participants` is `addr1|addr2|...` (sorted, deduped, lowercased).
///   The entire participants string is slugified as a single unit to produce
///   one folder level per conversation set (no nested thread subfolder).
///   If the source_id lacks a `gmail:` prefix or has no participants segment,
///   falls through to the chat/document layout using `slugify_source_id(source_id)`.
/// - Chat:     `"chat/<source_slug>/<chunk_id>.md"`
/// - Document: `"document/<source_slug>/<chunk_id>.md"`
///
/// `chunk_id` — the deterministic content hash produced by `types::chunk_id`.
///
/// # Examples
///
/// ```text
/// chunk_rel_path("email", "gmail:alice@x.com|bob@y.com", "abc")
///     → "email/alice-x-com-bob-y-com/abc.md"
///
/// chunk_rel_path("email", "gmail:notifications@github.com|sanil@x.com", "def")
///     → "email/notifications-github-com-sanil-x-com/def.md"
///
/// chunk_rel_path("email", "legacyid", "xyz")
///     → "email/legacyid/xyz.md"   (malformed — flat fallback)
/// ```
pub fn chunk_rel_path(source_kind: &str, source_id: &str, chunk_id: &str) -> String {
    match source_kind {
        "email" => {
            // Expected format: "gmail:{participants}"
            // Split on ':' — exactly 2 parts required; part[0] == "gmail".
            let parts: Vec<&str> = source_id.splitn(2, ':').collect();
            if parts.len() == 2 && parts[0] == "gmail" && !parts[1].is_empty() {
                let participants_slug = slugify_source_id(parts[1]);
                format!("email/{}/{}.md", participants_slug, chunk_id)
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
    fn email_one_to_one_conversation_path() {
        // 1:1 conversation between alice and bob.
        let p = chunk_rel_path("email", "gmail:alice@x.com|bob@y.com", "abc");
        assert_eq!(p, "email/alice-x-com-bob-y-com/abc.md");
    }

    #[test]
    fn email_group_conversation_path() {
        // Group conversation with three participants.
        let p = chunk_rel_path("email", "gmail:notifications@github.com|sanil@x.com", "def");
        assert_eq!(p, "email/notifications-github-com-sanil-x-com/def.md");
    }

    #[test]
    fn email_solo_no_to_path() {
        // Solo sender (no To), participants = single address.
        let p = chunk_rel_path("email", "gmail:alice@x.com", "solo123");
        assert_eq!(p, "email/alice-x-com/solo123.md");
    }

    #[test]
    fn email_malformed_source_id_falls_back_to_flat_layout() {
        // Malformed: no `gmail:` prefix → flat fallback.
        let p = chunk_rel_path("email", "legacyid", "xyz");
        // Falls back to email/<slug>/<chunk_id>.md
        assert!(p.starts_with("email/"), "must remain under email/");
        assert!(p.ends_with("/xyz.md"), "chunk_id must be the filename");
        // Must not panic.
    }

    #[test]
    fn email_three_participant_path() {
        // Three participants: alice, bob, carol (pipe-separated, sorted).
        let p = chunk_rel_path("email", "gmail:alice@x.com|bob@y.com|carol@z.com", "g42");
        assert_eq!(p, "email/alice-x-com-bob-y-com-carol-z-com/g42.md");
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
        let abs = chunk_abs_path(root, "email", "gmail:alice@x.com|bob@y.com", "abc");
        assert!(abs.starts_with(root));
        assert!(abs.ends_with("abc.md"));
    }

    // ─── summary_rel_path tests ───────────────────────────────────────────────

    #[test]
    fn summary_rel_path_source() {
        let p = summary_rel_path(
            SummaryTreeKind::Source,
            "gmail-alice-x-com-bob-y-com",
            1,
            "summary:L1:abc",
            None,
        );
        // Colons in summary_id are replaced with '-' for cross-platform filenames.
        assert_eq!(
            p,
            "summaries/source/gmail-alice-x-com-bob-y-com/L1/summary-L1-abc.md"
        );
    }

    #[test]
    fn summary_rel_path_global() {
        use chrono::TimeZone;
        let date = chrono::Utc.with_ymd_and_hms(2026, 4, 28, 12, 0, 0).unwrap();
        let p = summary_rel_path(
            SummaryTreeKind::Global,
            "global",
            0,
            "summary:L0:daily",
            Some(date),
        );
        assert_eq!(p, "summaries/global/2026-04-28/L0/summary-L0-daily.md");
    }

    #[test]
    fn summary_rel_path_topic() {
        let p = summary_rel_path(
            SummaryTreeKind::Topic,
            "person-alex-johnson",
            1,
            "summary:L1:xyz",
            None,
        );
        assert_eq!(
            p,
            "summaries/topic/person-alex-johnson/L1/summary-L1-xyz.md"
        );
    }

    #[test]
    fn summary_rel_path_strips_trailing_md_extension() {
        // If the caller accidentally appends .md to the summary_id, strip it.
        let p = summary_rel_path(
            SummaryTreeKind::Topic,
            "entity-slug",
            2,
            "summary:L2:foo.md",
            None,
        );
        assert_eq!(p, "summaries/topic/entity-slug/L2/summary-L2-foo.md");
    }

    #[test]
    #[should_panic(expected = "date_for_global is required")]
    fn summary_rel_path_global_panics_without_date() {
        summary_rel_path(SummaryTreeKind::Global, "global", 0, "summary:L0:x", None);
    }

    #[test]
    fn summary_abs_path_rooted_under_content_root() {
        use chrono::TimeZone;
        use std::path::Path;
        let root = Path::new("/workspace/content");
        let date = chrono::Utc.with_ymd_and_hms(2026, 1, 15, 0, 0, 0).unwrap();
        let abs = summary_abs_path(
            root,
            SummaryTreeKind::Global,
            "global",
            0,
            "daily-123",
            Some(date),
        );
        assert!(abs.starts_with(root));
        assert!(abs.ends_with("daily-123.md"));
    }
}
