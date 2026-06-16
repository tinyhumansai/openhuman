//! Summary `.md` file composition and tag rewriting.

use chrono::{DateTime, Utc};

use crate::openhuman::memory_store::content::compose::chunk::rewrite_tags;
use crate::openhuman::memory_store::content::compose::yaml::{
    source_tag, split_front_matter, yaml_scalar,
};
use crate::openhuman::memory_store::content::compose::{
    MEMORY_ARTIFACT_FORMAT, OPENHUMAN_CORE_VERSION,
};
use crate::openhuman::memory_store::content::paths::{summary_filename, SummaryTreeKind};

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
                None => summary_filename(id),
            };
            let wikilink = format!("[[{}]]", basename);
            fm.push_str(&format!("  - {}\n", yaml_scalar(&wikilink)));
        }
    }
    fm.push_str(&format!("child_count: {}\n", r.child_count));
    fm.push_str(&format!("time_range_start: {trs}\n"));
    fm.push_str(&format!("time_range_end: {tre}\n"));
    fm.push_str(&format!("sealed_at: {sealed}\n"));
    fm.push_str(&format!(
        "openhuman_core_version: {}\n",
        yaml_scalar(OPENHUMAN_CORE_VERSION)
    ));
    fm.push_str(&format!(
        "memory_artifact_format: {}\n",
        MEMORY_ARTIFACT_FORMAT
    ));

    // aliases: human-readable title
    let alias = build_summary_alias(r);
    fm.push_str("aliases:\n");
    fm.push_str(&format!("  - {}\n", yaml_scalar(&alias)));

    // Source-tree summaries get a `source/<slug>` seed tag for graph
    // filtering. Global / topic trees aggregate across sources, so the
    // `source/...` tag has no single value there — leave them untagged
    // at compose time (LLM extraction adds entity tags later).
    if matches!(r.tree_kind, SummaryTreeKind::Source) {
        fm.push_str("tags:\n");
        fm.push_str(&format!("  - {}\n", yaml_scalar(&source_tag(r.tree_scope))));
    } else {
        fm.push_str("tags: []\n");
    }
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
pub fn scope_short_label(scope: &str) -> String {
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
    let rewritten = rewrite_tags(file_bytes, new_tags)?;
    let content =
        std::str::from_utf8(&rewritten).map_err(|e| format!("file is not valid UTF-8: {e}"))?;
    let (front_matter, body) = split_front_matter(content)
        .ok_or_else(|| "cannot find front-matter delimiters".to_string())?;
    let front_matter = upsert_summary_provenance(front_matter);

    let mut out = Vec::with_capacity(front_matter.len() + body.len());
    out.extend_from_slice(front_matter.as_bytes());
    out.extend_from_slice(body.as_bytes());
    Ok(out)
}

fn upsert_summary_provenance(front_matter: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    let mut inserted = false;

    for raw in front_matter.lines() {
        if raw.starts_with("openhuman_core_version: ")
            || raw.starts_with("memory_artifact_format: ")
        {
            continue;
        }
        if !inserted && raw == "aliases:" {
            lines.push(format!(
                "openhuman_core_version: {}",
                yaml_scalar(OPENHUMAN_CORE_VERSION)
            ));
            lines.push(format!(
                "memory_artifact_format: {}",
                MEMORY_ARTIFACT_FORMAT
            ));
            inserted = true;
        }
        lines.push(raw.to_string());
    }

    if !inserted {
        let insert_at = lines
            .iter()
            .rposition(|line| line == "---")
            .unwrap_or(lines.len());
        lines.insert(
            insert_at,
            format!(
                "openhuman_core_version: {}",
                yaml_scalar(OPENHUMAN_CORE_VERSION)
            ),
        );
        lines.insert(
            insert_at + 1,
            format!("memory_artifact_format: {}", MEMORY_ARTIFACT_FORMAT),
        );
    }

    let mut result = lines.join("\n");
    result.push('\n');
    result
}
