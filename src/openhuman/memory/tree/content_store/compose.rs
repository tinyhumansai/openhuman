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
//! For email source_kind, additional fields are emitted:
//! ```text
//! participants:
//!   - alice@example.com
//!   - bob@example.com
//! aliases:
//!   - "alice@example.com <-> bob@example.com: chunk 0"
//! ```
//! These are parsed from the `source_id` field (format `gmail:{participants}`
//! where `participants` is `addr1|addr2|...` pipe-separated) at compose time.
//! `sender` and `thread_id` are no longer emitted — they are not meaningful
//! with participant-based bucketing.
//!
//! **SHA-256 is computed over the body bytes only** (everything after `---\n`
//! on the second delimiter line). This allows tags to be rewritten atomically
//! without invalidating the content hash.

use chrono::{DateTime, Utc};

use crate::openhuman::memory::tree::content_store::paths::{sanitize_filename, SummaryTreeKind};
use crate::openhuman::memory::tree::types::{Chunk, SourceKind};

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

    // Email-specific fields: participants list + Obsidian alias.
    // Parsed from source_id which is `gmail:{participants}` for Gmail-ingested
    // chunks, where participants is `addr1|addr2|...` (sorted, deduped).
    // If the format doesn't match, these fields are omitted.
    if meta.source_kind == SourceKind::Email {
        if let Some(addrs) = parse_gmail_participants_source_id(&meta.source_id) {
            // participants: YAML list
            fm.push_str("participants:\n");
            for addr in &addrs {
                fm.push_str(&format!("  - {}\n", yaml_scalar(addr)));
            }
            // aliases: human-readable conversation label for Obsidian
            let alias = build_participants_alias(&addrs, chunk.seq_in_source);
            fm.push_str("aliases:\n");
            fm.push_str(&format!("  - {}\n", yaml_scalar(&alias)));
        }
    }

    fm.push_str("---\n");
    fm.into_bytes()
}

/// Parse a `gmail:{participants}` source_id into the list of participant addresses.
///
/// `participants` is `addr1|addr2|...` (sorted, deduped, pipe-separated).
/// Returns `Some(Vec<String>)` when the source_id has exactly two
/// colon-separated segments (`gmail` prefix + non-empty participants). Returns
/// `None` for legacy or malformed source_ids.
fn parse_gmail_participants_source_id(source_id: &str) -> Option<Vec<String>> {
    let (prefix, participants) = source_id.split_once(':')?;
    if prefix != "gmail" || participants.is_empty() {
        return None;
    }
    let addrs: Vec<String> = participants
        .split('|')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if addrs.is_empty() {
        None
    } else {
        Some(addrs)
    }
}

/// Build a human-readable alias for an email chunk suitable for Obsidian's
/// `aliases:` field.
///
/// For two participants: `"alice@x.com <-> bob@y.com: chunk 0"`
/// For more than two:   `"alice@x.com <-> 2 others: chunk 0"`
///   (where `alice@x.com` is the first in sorted order)
///
/// The alias is kept under ~80 characters to avoid YAML rendering issues.
fn build_participants_alias(addrs: &[String], seq: u32) -> String {
    let label = match addrs {
        [] => "unknown".to_string(),
        [only] => only.clone(),
        [first, second] => format!("{} <-> {}", first, second),
        [first, rest @ ..] => format!("{} <-> {} others", first, rest.len()),
    };
    format!("{}: chunk {}", label, seq)
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

// ── Summary composition ──────────────────────────────────────────────────────

/// Input data required to compose a summary `.md` file.
pub struct SummaryComposeInput<'a> {
    /// Stable id of the summary node (also used to derive the filename).
    pub summary_id: &'a str,
    /// Which tree (source / global / topic) this summary belongs to.
    pub tree_kind: SummaryTreeKind,
    /// Owning tree id (FK into `mem_tree_trees`).
    pub tree_id: &'a str,
    /// Raw tree scope string, e.g. `"gmail:alice@x.com|bob@y.com"` or `"global"`.
    pub tree_scope: &'a str,
    /// Level in the tree (L0 = leaves, L1+ = summaries).
    pub level: u32,
    /// Child ids (chunk_ids at L0 → L1, summary_ids for cascades).
    pub child_ids: &'a [String],
    /// Optional per-child wikilink basename overrides, aligned with
    /// `child_ids` by index. When `Some(basename)` is provided for a
    /// child, the front-matter `children: [[…]]` wikilink uses that
    /// basename instead of `sanitize_filename(child_id)`.
    ///
    /// Used to point chunk-level children at their **raw archive**
    /// files when the chunk store no longer stages on-disk `.md`
    /// files (today: email, since email chunks live as byte ranges
    /// inside `raw/<source>/<ts_ms>_<msg>.md` instead of
    /// `email/<scope>/<chunk_id>.md`). Without this, Obsidian
    /// wikilinks resolve to a non-existent `[[<chunk_hash>]]`
    /// target and the graph view stops drawing edges from L1
    /// summaries down to leaves.
    ///
    /// `None` (or `Some` entries that are themselves `None`) falls
    /// back to the default `sanitize_filename(child_id)` behaviour,
    /// which is correct for L≥2 (children are summary ids that map
    /// to actual `summaries/...md` files) and for legacy chunks
    /// still staged on-disk.
    pub child_basenames: Option<&'a [Option<String>]>,
    /// Total child count (== child_ids.len() unless truncated).
    pub child_count: usize,
    /// Start of the time range covered by this summary's children.
    pub time_range_start: DateTime<Utc>,
    /// End of the time range covered by this summary's children.
    pub time_range_end: DateTime<Utc>,
    /// When the buffer was sealed into this summary node.
    pub sealed_at: DateTime<Utc>,
    /// Raw summariser output text — the body written to disk.
    pub body: &'a str,
}

/// The composed front-matter, body, and full file content for a summary.
///
/// `body` is what the SHA-256 integrity hash is computed over.
pub struct ComposedSummary {
    /// The YAML front-matter block (including `---` delimiters), UTF-8 string.
    pub front_matter: String,
    /// The body (summariser output), UTF-8 string.
    pub body: String,
    /// `front_matter + body` — what gets written to disk.
    pub full: String,
}

/// Compose the full `.md` content for a summary node.
///
/// Returns a [`ComposedSummary`] whose `full` field is written to disk.
/// SHA-256 is computed over `body` bytes only, not `full`.
pub fn compose_summary_md(record: &SummaryComposeInput<'_>) -> ComposedSummary {
    let fm = build_summary_front_matter(record);
    let body = record.body.to_string();
    let full = format!("{}{}", fm, body);
    ComposedSummary {
        front_matter: fm,
        body,
        full,
    }
}

/// Build the YAML front-matter block for a summary node.
fn build_summary_front_matter(r: &SummaryComposeInput<'_>) -> String {
    let tree_kind_str = match r.tree_kind {
        SummaryTreeKind::Source => "source",
        SummaryTreeKind::Global => "global",
        SummaryTreeKind::Topic => "topic",
    };

    let trs = r.time_range_start.to_rfc3339();
    let tre = r.time_range_end.to_rfc3339();
    let sealed = r.sealed_at.to_rfc3339();

    let mut fm = String::new();
    fm.push_str("---\n");
    fm.push_str(&format!("id: {}\n", yaml_scalar(r.summary_id)));
    fm.push_str("kind: summary\n");
    fm.push_str(&format!("tree_kind: {tree_kind_str}\n"));
    fm.push_str(&format!("tree_id: {}\n", yaml_scalar(r.tree_id)));
    fm.push_str(&format!("tree_scope: {}\n", yaml_scalar(r.tree_scope)));
    fm.push_str(&format!("level: {}\n", r.level));

    // children: YAML list of Obsidian wikilinks (`[[<basename>]]`) so the
    // graph view draws summary→child edges. The wikilink target must match
    // the actual file basename — for chunks that's the raw chunk_id (a SHA
    // hash with no illegal chars), but for child summaries the structured id
    // `summary:L<n>:UUID` is sanitised to `summary-L<n>-UUID` by
    // `summary_rel_path` (colons are illegal on Windows NTFS). We apply the
    // same sanitisation here so the link resolves. `yaml_scalar` auto-quotes
    // because of the leading `[`, emitting `"[[<basename>]]"`.
    if r.child_ids.is_empty() {
        fm.push_str("children: []\n");
    } else {
        fm.push_str("children:\n");
        for (i, id) in r.child_ids.iter().enumerate() {
            // Prefer a caller-supplied basename override (used for L1
            // chunk children that live in the raw archive instead of
            // the chunk-store path); fall back to the sanitised
            // chunk/summary id.
            let basename: String = match r
                .child_basenames
                .and_then(|overrides| overrides.get(i))
                .and_then(|slot| slot.as_ref())
            {
                Some(b) => b.clone(),
                None => sanitize_filename(id),
            };
            let wikilink = format!("[[{}]]", basename);
            fm.push_str(&format!("  - {}\n", yaml_scalar(&wikilink)));
        }
    }
    fm.push_str(&format!("child_count: {}\n", r.child_count));
    fm.push_str(&format!("time_range_start: {trs}\n"));
    fm.push_str(&format!("time_range_end: {tre}\n"));
    fm.push_str(&format!("sealed_at: {sealed}\n"));

    // aliases: human-readable title
    let alias = build_summary_alias(r);
    fm.push_str("aliases:\n");
    fm.push_str(&format!("  - {}\n", yaml_scalar(&alias)));

    fm.push_str("tags: []\n");
    fm.push_str("---\n");
    fm
}

/// Build a human-readable alias for the summary's `aliases:` front-matter field.
fn build_summary_alias(r: &SummaryComposeInput<'_>) -> String {
    let date_range = format_date_range(r.time_range_start, r.time_range_end);
    match r.tree_kind {
        SummaryTreeKind::Source => {
            let scope_short = scope_short_label(r.tree_scope);
            format!(
                "L{} \u{00b7} {} \u{00b7} {} children \u{00b7} {}",
                r.level, scope_short, r.child_count, date_range
            )
        }
        SummaryTreeKind::Global => {
            format!(
                "L{} \u{00b7} global digest \u{00b7} {}",
                r.level, date_range
            )
        }
        SummaryTreeKind::Topic => {
            // Strip protocol prefix like "topic:" from scope for readability.
            let entity = r
                .tree_scope
                .split_once(':')
                .map(|(_, v)| v)
                .unwrap_or(r.tree_scope);
            format!(
                "L{} \u{00b7} topic {} \u{00b7} {} children",
                r.level, entity, r.child_count
            )
        }
    }
}

/// Format the date range as `"yyyy-mm-dd"` (if start == end date) or
/// `"yyyy-mm-dd–yyyy-mm-dd"`.
fn format_date_range(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let s = start.format("%Y-%m-%d").to_string();
    let e = end.format("%Y-%m-%d").to_string();
    if s == e {
        s
    } else {
        format!("{s}\u{2013}{e}") // en dash
    }
}

/// Build a short human-readable label for the tree scope used in aliases.
///
/// For Gmail source scopes like `"gmail:alice@x.com|bob@y.com"`:
/// - 2 participants → `"alice@x.com ↔ bob@y.com"`
/// - N > 2 → `"alice@x.com + N-1 others"`
/// - Otherwise → the raw scope (e.g. `"slack:#eng"`)
fn scope_short_label(scope: &str) -> String {
    if let Some((prefix, participants)) = scope.split_once(':') {
        if prefix == "gmail" && !participants.is_empty() {
            let addrs: Vec<&str> = participants.split('|').collect();
            return match addrs.as_slice() {
                [] => scope.to_string(),
                [only] => only.to_string(),
                [first, second] => format!("{} \u{2194} {}", first, second), // ↔
                [first, rest @ ..] => format!("{} + {} others", first, rest.len()),
            };
        }
    }
    scope.to_string()
}

/// Rewrite the `tags:` block in a summary file's front-matter, replacing it
/// with the new tag list while leaving the body unchanged.
///
/// Reuses the generic [`rewrite_tags`] function — the front-matter structure
/// is identical for both chunk and summary `.md` files.
pub fn rewrite_summary_tags(file_bytes: &[u8], new_tags: &[String]) -> Result<Vec<u8>, String> {
    rewrite_tags(file_bytes, new_tags)
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
        || s.contains([':', '#', '[', ']', '{', '}', '"', '\'']);

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
    use crate::openhuman::memory::tree::content_store::paths::SummaryTreeKind;
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

    fn sample_email_chunk() -> Chunk {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        Chunk {
            id: "emailchunk1".into(),
            content: "---\nFrom: alice@example.com\nSubject: Hello\n\nHello there.".into(),
            metadata: Metadata {
                source_kind: SourceKind::Email,
                source_id: "gmail:alice@example.com|bob@example.com".into(),
                owner: "owner@example.com".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec!["gmail".into()],
                source_ref: None,
            },
            token_count: 15,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        }
    }

    #[test]
    fn email_chunk_has_participants_list_and_alias() {
        let chunk = sample_email_chunk();
        let (full, _body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        // participants block must be a YAML list
        assert!(
            full_str.contains("participants:"),
            "email chunk must have participants field; got:\n{full_str}"
        );
        assert!(
            full_str.contains("  - alice@example.com"),
            "alice must appear as list item; got:\n{full_str}"
        );
        assert!(
            full_str.contains("  - bob@example.com"),
            "bob must appear as list item; got:\n{full_str}"
        );
        // aliases block must be present
        assert!(
            full_str.contains("aliases:"),
            "email chunk must have aliases field; got:\n{full_str}"
        );
        assert!(
            full_str.contains("alice@example.com <-> bob@example.com: chunk 0"),
            "alias must encode participants; got:\n{full_str}"
        );
        // sender and thread_id must NOT appear
        assert!(
            !full_str.contains("sender:"),
            "email chunk must NOT have sender field; got:\n{full_str}"
        );
        assert!(
            !full_str.contains("thread_id:"),
            "email chunk must NOT have thread_id field; got:\n{full_str}"
        );
    }

    #[test]
    fn email_chunk_many_participants_alias_summarises() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: "em2".into(),
            content: "body".into(),
            metadata: Metadata {
                source_kind: SourceKind::Email,
                source_id: "gmail:alice@x.com|bob@y.com|carol@z.com".into(),
                owner: "owner".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: None,
            },
            token_count: 1,
            seq_in_source: 3,
            created_at: ts,
            partial_message: false,
        };
        let (full, _) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        assert!(
            full_str.contains("participants:"),
            "three-party chunk needs participants list; got:\n{full_str}"
        );
        // With 3 participants: first + "2 others"
        assert!(
            full_str.contains("alice@x.com <-> 2 others: chunk 3"),
            "alias with 3 participants must summarise; got:\n{full_str}"
        );
    }

    #[test]
    fn email_chunk_body_bytes_unchanged_by_extra_fields() {
        // Adding participants/aliases to front-matter must not affect body_bytes
        // (SHA-256 invariant: the hash is over body only, not front-matter).
        let chunk = sample_email_chunk();
        let (full, body) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        // Body must still appear at the end unmodified.
        assert!(
            full_str.ends_with(std::str::from_utf8(&body).unwrap()),
            "body bytes must appear unmodified after front-matter"
        );
        // body must equal chunk.content bytes
        assert_eq!(body, chunk.content.as_bytes());
    }

    #[test]
    fn chat_chunk_has_no_email_specific_fields() {
        let chunk = sample_chunk(); // source_kind = Chat
        let (full, _) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        assert!(
            !full_str.contains("aliases:"),
            "chat chunk must not have aliases field"
        );
        assert!(
            !full_str.contains("participants:"),
            "chat chunk must not have participants field"
        );
        assert!(
            !full_str.contains("sender:"),
            "chat chunk must not have sender field"
        );
        assert!(
            !full_str.contains("thread_id:"),
            "chat chunk must not have thread_id field"
        );
    }

    #[test]
    fn email_chunk_with_malformed_source_id_omits_extra_fields() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let chunk = Chunk {
            id: "xyz".into(),
            content: "body".into(),
            metadata: Metadata {
                source_kind: SourceKind::Email,
                source_id: "legacysourceid".into(), // no `gmail:` prefix → parse fails
                owner: "owner".into(),
                timestamp: ts,
                time_range: (ts, ts),
                tags: vec![],
                source_ref: None,
            },
            token_count: 1,
            seq_in_source: 0,
            created_at: ts,
            partial_message: false,
        };
        let (full, _) = compose_chunk_file(&chunk);
        let full_str = std::str::from_utf8(&full).unwrap();
        // Malformed source_id → no email extras, no panic.
        assert!(!full_str.contains("aliases:"));
        assert!(!full_str.contains("participants:"));
        assert!(!full_str.contains("sender:"));
    }

    // ─── summary compose tests ────────────────────────────────────────────────

    fn sample_summary_input(
        tree_kind: SummaryTreeKind,
        scope: &str,
        level: u32,
    ) -> SummaryComposeInput<'static> {
        let ts_start = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let ts_end = chrono::Utc.timestamp_millis_opt(1_700_086_400_000).unwrap();
        let sealed = chrono::Utc.timestamp_millis_opt(1_700_090_000_000).unwrap();
        // Leak the strings so they have 'static lifetime for this test helper.
        // Only used in tests, not production code.
        let scope: &'static str = Box::leak(scope.to_string().into_boxed_str());
        SummaryComposeInput {
            summary_id: "summary:L1:abc",
            tree_kind,
            tree_id: "tree-id-001",
            tree_scope: scope,
            level,
            child_ids: Box::leak(
                vec!["child-1".to_string(), "child-2".to_string()].into_boxed_slice(),
            ),
            child_basenames: None,
            child_count: 2,
            time_range_start: ts_start,
            time_range_end: ts_end,
            sealed_at: sealed,
            body: "This is the summariser output.\n",
        }
    }

    #[test]
    fn compose_source_summary_has_required_front_matter() {
        let input = sample_summary_input(SummaryTreeKind::Source, "gmail:alice@x.com|bob@y.com", 1);
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        assert!(fm.starts_with("---\n"), "front-matter must start with ---");
        assert!(fm.ends_with("---\n"), "front-matter must end with ---\\n");
        assert!(fm.contains("kind: summary"), "must have kind: summary");
        assert!(
            fm.contains("tree_kind: source"),
            "must have tree_kind: source"
        );
        assert!(fm.contains("level: 1"), "must have level");
        assert!(fm.contains("child_count: 2"), "must have child_count");
        assert!(
            fm.contains("  - \"[[child-1]]\""),
            "must list child ids as Obsidian wikilinks; got:\n{fm}"
        );
        assert!(
            fm.contains("  - \"[[child-2]]\""),
            "must list child ids as Obsidian wikilinks; got:\n{fm}"
        );
        assert!(fm.contains("tags: []"), "must start with empty tags");
        // aliases must mention the scope
        assert!(fm.contains("aliases:"), "must have aliases");
        assert!(
            composed.body == "This is the summariser output.\n",
            "body must be the summariser text"
        );
        assert!(composed.full.ends_with("This is the summariser output.\n"));
    }

    #[test]
    fn children_are_emitted_as_obsidian_wikilinks() {
        // Contract: every entry in `children:` must be wrapped in `[[…]]` so
        // Obsidian's graph view draws a summary→child edge. The YAML scalar is
        // quoted because of the leading `[` — both forms below are required.
        let input = sample_summary_input(SummaryTreeKind::Source, "gmail:alice@x.com", 1);
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        for id in ["child-1", "child-2"] {
            let expected = format!("  - \"[[{id}]]\"");
            assert!(
                fm.contains(&expected),
                "child id {id} must be emitted as a quoted wikilink ({expected}); got:\n{fm}"
            );
            // Belt-and-braces: the bare id must NOT appear as a plain scalar
            // (i.e. unwrapped). The wikilink form contains the id, so we
            // search for the bare list-item form.
            let plain = format!("  - {id}\n");
            assert!(
                !fm.contains(&plain),
                "child id {id} must not be emitted as a plain scalar; got:\n{fm}"
            );
        }
    }

    #[test]
    fn child_basename_overrides_replace_chunk_id_in_wikilink() {
        // L1 seals: each child's wikilink should point at the
        // raw archive file basename, not the chunk_id hash. Without
        // this override the link would be `[[<32-char hex>]]` and
        // Obsidian wouldn't find a matching file (the chunk-store
        // copy under `email/<scope>/...` is gone after the
        // raw_refs migration).
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let child_ids = vec!["abc123hash".to_string(), "def456hash".to_string()];
        let overrides: Vec<Option<String>> = vec![
            Some("1700000000000_msg-id-1".into()),
            None, // second child has no override → falls back to sanitize_filename
        ];
        let input = SummaryComposeInput {
            summary_id: "summary:L1:test",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 1,
            child_ids: &child_ids,
            child_basenames: Some(&overrides),
            child_count: 2,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "L1 body",
        };
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        // First child uses the override (raw archive basename).
        assert!(
            fm.contains(r#"  - "[[1700000000000_msg-id-1]]""#),
            "first child must use override basename; got:\n{fm}"
        );
        // Second child has None override — fall back to chunk_id.
        assert!(
            fm.contains(r#"  - "[[def456hash]]""#),
            "None override must fall back to sanitize_filename; got:\n{fm}"
        );
    }

    #[test]
    fn structured_child_summary_id_is_sanitised_in_wikilink() {
        // Real-world case: an L2 summary lists child L1 summaries by their
        // structured id (e.g. `summary:L1:UUID`). Colons are illegal in
        // Windows NTFS filenames, so `summary_rel_path` writes the file as
        // `summary-L1-UUID.md`. The wikilink target must match that basename
        // — i.e. colons must be converted to dashes — otherwise Obsidian
        // cannot resolve the link and the graph stays disconnected.
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let child_id = "summary:L1:b9fa5f08-bf79-41a7-a5c8-2d87883d5c01";
        let expected_basename = "summary-L1-b9fa5f08-bf79-41a7-a5c8-2d87883d5c01";
        let input = SummaryComposeInput {
            summary_id: "summary:L2:cc9a1224",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 2,
            child_ids: &[child_id.to_string()],
            child_basenames: None,
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "L2 body",
        };
        let composed = compose_summary_md(&input);
        let fm = &composed.front_matter;
        let expected = format!("  - \"[[{expected_basename}]]\"");
        assert!(
            fm.contains(&expected),
            "structured child id must be sanitised to filename basename in wikilink; \
             expected line: {expected}; got:\n{fm}"
        );
        // Raw colon-bearing id must NOT appear inside `[[…]]` — that wikilink
        // would not resolve in Obsidian.
        assert!(
            !fm.contains(&format!("[[{child_id}]]")),
            "raw structured id with colons must not appear inside wikilink; got:\n{fm}"
        );
    }

    #[test]
    fn compose_global_summary_alias_format() {
        let input = sample_summary_input(SummaryTreeKind::Global, "global", 0);
        let composed = compose_summary_md(&input);
        assert!(
            composed.front_matter.contains("tree_kind: global"),
            "must have tree_kind: global"
        );
        assert!(
            composed.front_matter.contains("global digest"),
            "alias must mention 'global digest'"
        );
    }

    #[test]
    fn compose_topic_summary_alias_format() {
        let input = sample_summary_input(SummaryTreeKind::Topic, "person:alex-johnson", 1);
        let composed = compose_summary_md(&input);
        assert!(
            composed.front_matter.contains("tree_kind: topic"),
            "must have tree_kind: topic"
        );
        assert!(
            composed.front_matter.contains("topic"),
            "alias must mention topic entity"
        );
    }

    #[test]
    fn compose_summary_with_zero_children() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "summary:L0:empty",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 0,
            child_ids: &[],
            child_basenames: None,
            child_count: 0,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "empty",
        };
        let composed = compose_summary_md(&input);
        assert!(composed.front_matter.contains("children: []"));
        assert!(composed.front_matter.contains("child_count: 0"));
    }

    #[test]
    fn compose_summary_same_start_end_date_single_date_alias() {
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "summary:L1:sameday",
            tree_kind: SummaryTreeKind::Global,
            tree_id: "t1",
            tree_scope: "global",
            level: 1,
            child_ids: &["child-a".to_string()],
            child_basenames: None,
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts, // same as start
            sealed_at: ts,
            body: "day recap",
        };
        let composed = compose_summary_md(&input);
        // Alias must contain just one date, not "date–date"
        let alias_line = composed
            .front_matter
            .lines()
            .find(|l| l.contains("L1") && l.contains("global digest"))
            .expect("alias line must be present");
        // The date should appear exactly once (no en-dash range)
        let date_str = ts.format("%Y-%m-%d").to_string();
        assert!(
            alias_line.contains(&date_str),
            "alias must contain the date; got: {alias_line}"
        );
        // Must not contain an en-dash (range indicator)
        assert!(
            !alias_line.contains('\u{2013}'),
            "same-day alias must not have en-dash range; got: {alias_line}"
        );
    }

    #[test]
    fn scope_short_label_two_participants() {
        let label = scope_short_label("gmail:alice@x.com|bob@y.com");
        assert_eq!(label, "alice@x.com \u{2194} bob@y.com");
    }

    #[test]
    fn scope_short_label_many_participants() {
        let label = scope_short_label("gmail:alice@x.com|bob@y.com|carol@z.com");
        assert_eq!(label, "alice@x.com + 2 others");
    }

    #[test]
    fn scope_short_label_non_gmail_returns_raw() {
        let label = scope_short_label("slack:#general");
        assert_eq!(label, "slack:#general");
    }

    #[test]
    fn rewrite_summary_tags_delegates_to_rewrite_tags() {
        // compose a summary, then rewrite its tags — body must stay unchanged.
        let ts = chrono::Utc.timestamp_millis_opt(1_700_000_000_000).unwrap();
        let input = SummaryComposeInput {
            summary_id: "sum:L1:rwttest",
            tree_kind: SummaryTreeKind::Source,
            tree_id: "t1",
            tree_scope: "gmail:alice@x.com",
            level: 1,
            child_ids: &["c1".to_string()],
            child_basenames: None,
            child_count: 1,
            time_range_start: ts,
            time_range_end: ts,
            sealed_at: ts,
            body: "summary body text",
        };
        let composed = compose_summary_md(&input);
        let file_bytes = composed.full.as_bytes();
        let new_tags = vec!["person/Alice-Smith".to_string(), "topic/Memory".to_string()];
        let rewritten = rewrite_summary_tags(file_bytes, &new_tags).unwrap();
        let rewritten_str = std::str::from_utf8(&rewritten).unwrap();
        assert!(rewritten_str.contains("  - person/Alice-Smith"));
        assert!(rewritten_str.contains("  - topic/Memory"));
        assert!(!rewritten_str.contains("tags: []"));
        // Body must be unchanged
        assert!(rewritten_str.ends_with("summary body text"));
    }
}
