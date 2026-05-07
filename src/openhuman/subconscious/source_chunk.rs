//! Resolved source-chunk records for proactive reflections (#623).
//!
//! At tick time, the LLM emits each reflection with a `source_refs` list of
//! opaque ids like `entity:phoenix` or `summary:abc123` — pointers into the
//! same memory-tree data that built the situation report it just read. The
//! engine resolves each id into a [`SourceChunk`] (the underlying content
//! preview) before persisting the reflection, so:
//!
//! 1. The Intelligence-tab card can show a "Sources" disclosure with the
//!    chunks that informed the observation (transparency).
//! 2. The orchestrator's `SystemPromptBuilder` can inject those chunks into
//!    the system prompt for any chat turn in a thread spawned from the
//!    reflection (memory context, so follow-ups stay grounded — see the
//!    "Memory context" branch in `context::prompt::SystemPromptBuilder`).
//!
//! Snapshots are deliberate — chunks freeze at tick time so a thread
//! spawned from a week-old reflection still shows the LLM's original
//! context even if the underlying entity has since been merged or the
//! summary re-sealed.

use serde::{Deserialize, Serialize};

/// One resolved chunk of memory-tree content the reflection LLM cited via
/// `source_refs`. Snapshot-shaped: `content_preview` is the resolved text
/// at tick time, not a live join.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SourceChunk {
    /// The original opaque id from the LLM, e.g. `"entity:phoenix"` or
    /// `"summary:abc123"`. Preserved verbatim so dedup keys, debug logs,
    /// and downstream consumers can correlate against the raw LLM output.
    pub ref_id: String,

    /// Parsed kind portion of `ref_id` (the part before the first `:`).
    /// `"entity"`, `"summary"`, `"digest"`, `"recap"`, etc. `"unknown"`
    /// when the ref didn't contain a `:` separator.
    pub kind: String,

    /// Resolved chunk preview — the content the LLM was looking at, capped
    /// to ~`PREVIEW_MAX_CHARS` so the per-reflection row stays bounded.
    /// Empty when no resolver matched the kind (graceful degrade).
    pub content: String,

    /// Optional per-kind metadata, free-form JSON. For entities this might
    /// hold `{display_name, hotness}`; for summaries `{tree_id, sealed_at}`.
    /// Renderers MAY use these for richer chip displays; pure consumers can
    /// ignore.
    #[serde(default)]
    pub metadata: serde_json::Value,
}

/// Hard cap on resolved chunk content length so reflection rows don't bloat.
/// Picked empirically: 400 chars is enough for a useful preview while
/// keeping a 5-chunk reflection under 2 KB of stored JSON.
pub const PREVIEW_MAX_CHARS: usize = 400;

/// Parse a `kind:id` ref into its two components. Returns
/// `("unknown", &full_ref)` if there's no `:` separator so callers can
/// still record the original id without crashing on malformed LLM output.
pub fn parse_ref(raw: &str) -> (&str, &str) {
    match raw.split_once(':') {
        Some((kind, id)) => (kind, id),
        None => ("unknown", raw),
    }
}

/// Cap a resolved content string to [`PREVIEW_MAX_CHARS`] characters,
/// appending `…` when truncated. Operates on chars (not bytes) so multi-
/// byte UTF-8 input doesn't get cut mid-codepoint.
pub fn truncate_preview(text: &str) -> String {
    if text.chars().count() <= PREVIEW_MAX_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(PREVIEW_MAX_CHARS).collect();
    out.push('…');
    out
}

/// Resolve a list of raw `source_refs` into [`SourceChunk`]s.
///
/// MVP behaviour:
/// - `entity:<id>` and `summary:<id>` get content lookups (the two kinds
///   the LLM cites most often, per `prompt::build_evaluation_prompt`).
/// - All other kinds — `digest:`, `recap:`, anything novel — record an
///   empty-content chunk with `kind` set so the system-prompt injector
///   and the UI disclosure can still surface the ref id, just without
///   resolved text. Add resolvers per kind here as the LLM starts citing
///   them in real data.
///
/// Errors during resolution are swallowed per-ref: one bad id should not
/// stop a tick from persisting its other reflections. Failed resolutions
/// degrade to empty `content` with a `metadata.error` field set so the
/// system-prompt injector can still annotate "source unavailable".
pub fn resolve_chunks(
    config: &crate::openhuman::config::Config,
    source_refs: &[String],
) -> Vec<SourceChunk> {
    source_refs
        .iter()
        .map(|raw| resolve_one(config, raw))
        .collect()
}

fn resolve_one(config: &crate::openhuman::config::Config, raw: &str) -> SourceChunk {
    let (kind, _id_after_colon) = parse_ref(raw);
    // Important: the DB primary keys for summaries and entities INCLUDE the
    // kind prefix as part of the id — `mem_tree_summaries.id` looks like
    // `summary:L0:<uuid>` and `mem_tree_entity_index.entity_id` looks like
    // `artifact:"<surface>"` etc. So we route to a resolver by the kind
    // *prefix* but the resolver queries against the **full raw ref**, not
    // the part after the first colon. The earlier (broken) version
    // stripped the prefix and found nothing in either table.
    match kind {
        "summary" => resolve_summary(config, raw),
        // Reject only the obvious non-lookups: refs the parser gave up
        // on (`unknown` / empty kind) get an empty stub; everything
        // else is treated as a candidate entity_index lookup. The LLM
        // emits `artifact:`, `person:`, `place:`, `tool:`, `topic:`,
        // and occasionally novel kinds the schema later picks up — an
        // allowlist would silently drop those, taking their evidence
        // out of the reflection snapshot. Letting the SQL miss decide
        // costs at most one extra `query_row` for ids that happen not
        // to exist (e.g. per-tick `due_item:<uuid>` placeholders).
        "unknown" | "" => SourceChunk {
            ref_id: raw.to_string(),
            kind: kind.to_string(),
            content: String::new(),
            metadata: serde_json::Value::Null,
        },
        _ => resolve_entity(config, raw),
    }
}

/// Look up a sealed summary by id. Mirrors the read pattern in
/// [`crate::openhuman::subconscious::situation_report::summaries`] but
/// fetches a single row instead of the recent-summaries window. The
/// resolved `content` is truncated to [`PREVIEW_MAX_CHARS`] so the
/// reflection row stays bounded; full content remains queryable from
/// `mem_tree_summaries` if a future feature needs it.
///
/// Best-effort — DB errors, missing rows, or deleted summaries all
/// degrade to an empty-content chunk with a `resolver_status` metadata
/// field set so consumers can distinguish "not yet resolved" from
/// "looked up and got nothing."
fn resolve_summary(config: &crate::openhuman::config::Config, raw: &str) -> SourceChunk {
    // The DB primary key for `mem_tree_summaries.id` IS the full prefixed
    // string the LLM cites — e.g. `summary:L0:<uuid>` — because the
    // situation report's summaries section renders `s.id` verbatim and
    // that's what the LLM echoes back. Query against the raw ref
    // directly; an earlier version stripped `summary:` and the
    // `L<digits>:` token, which left no row matching anything in the
    // table.
    let lookup: anyhow::Result<Option<(String, i64, String)>> =
        crate::openhuman::memory::tree::store::with_connection(config, |conn| {
            let mut stmt = conn.prepare(
                "SELECT s.content, s.level, t.scope
                 FROM mem_tree_summaries s
                 JOIN mem_tree_trees t ON t.id = s.tree_id
                 WHERE s.id = ?1 AND s.deleted = 0",
            )?;
            let row = stmt
                .query_row(rusqlite::params![raw], |row| {
                    let content: String = row.get(0)?;
                    let level: i64 = row.get(1)?;
                    let scope: String = row.get(2)?;
                    Ok((content, level, scope))
                })
                .ok();
            Ok(row)
        });

    match lookup {
        Ok(Some((content, level, scope))) => SourceChunk {
            ref_id: raw.to_string(),
            kind: "summary".to_string(),
            content: truncate_preview(content.trim()),
            metadata: serde_json::json!({
                "tree_scope": scope,
                "level": level,
            }),
        },
        Ok(None) => SourceChunk {
            ref_id: raw.to_string(),
            kind: "summary".to_string(),
            content: String::new(),
            metadata: serde_json::json!({
                "resolver_status": "not_found",
            }),
        },
        Err(e) => {
            log::debug!("[subconscious::source_chunk] resolve_summary db error for {raw}: {e}");
            SourceChunk {
                ref_id: raw.to_string(),
                kind: "summary".to_string(),
                content: String::new(),
                metadata: serde_json::json!({
                    "resolver_status": "db_error",
                }),
            }
        }
    }
}

/// Look up an entity by id and return its top surface form +
/// `entity_kind` plus the latest hotness score (when present). Joins
/// the (possibly many-row) `mem_tree_entity_index` to pick the highest-
/// scoring representative surface, then enriches with the score from
/// `mem_tree_entity_hotness` when available. Same best-effort error
/// behaviour as [`resolve_summary`].
fn resolve_entity(config: &crate::openhuman::config::Config, raw: &str) -> SourceChunk {
    // Same key convention as summaries — `mem_tree_entity_index.entity_id`
    // is the full kind-prefixed string (`artifact:"foo"`, `person:bar`,
    // etc.). Match against the raw ref verbatim.
    //
    // The returned `SourceChunk.kind` carries the LLM's *original*
    // prefix (`artifact`, `person`, `tool`, …) instead of being flattened
    // to the literal `"entity"` — preserving the exact type the LLM
    // cited matters for the system-prompt renderer downstream and for
    // any UI that wants to chip the chunk by category.
    let original_kind = parse_ref(raw).0.to_string();
    type EntityLookup = anyhow::Result<Option<(String, String, f64, Option<f64>)>>;
    let lookup: EntityLookup =
        crate::openhuman::memory::tree::store::with_connection(config, |conn| {
            // Top-scoring surface form for this entity.
            let mut stmt = conn.prepare(
                "SELECT entity_kind, surface, score
                 FROM mem_tree_entity_index
                 WHERE entity_id = ?1
                 ORDER BY score DESC
                 LIMIT 1",
            )?;
            let row = stmt
                .query_row(rusqlite::params![raw], |row| {
                    let entity_kind: String = row.get(0)?;
                    let surface: String = row.get(1)?;
                    let score: f64 = row.get(2)?;
                    Ok((entity_kind, surface, score))
                })
                .ok();

            let Some((entity_kind, surface, score)) = row else {
                return Ok(None);
            };

            // Optional hotness enrichment — empty for entities the
            // hotness pass hasn't seen yet, fine to leave None.
            let mut hotness_stmt = conn.prepare(
                "SELECT last_hotness FROM mem_tree_entity_hotness
                 WHERE entity_id = ?1 AND last_hotness IS NOT NULL",
            )?;
            let hotness: Option<f64> = hotness_stmt
                .query_row(rusqlite::params![raw], |row| row.get(0))
                .ok();

            Ok(Some((entity_kind, surface, score, hotness)))
        });

    match lookup {
        Ok(Some((entity_kind, surface, score, hotness))) => {
            // Content is the human-readable representation the LLM can
            // cite back: "<kind>: <surface>". Score + hotness ride in
            // metadata so consumers (UI / future prompt sections) can
            // render them without parsing free text.
            let content = truncate_preview(&format!("{entity_kind}: {surface}"));
            let mut metadata = serde_json::json!({
                "entity_kind": entity_kind,
                "surface": surface,
                "index_score": score,
            });
            if let Some(h) = hotness {
                metadata["hotness"] = serde_json::json!(h);
            }
            SourceChunk {
                ref_id: raw.to_string(),
                kind: original_kind.clone(),
                content,
                metadata,
            }
        }
        Ok(None) => SourceChunk {
            ref_id: raw.to_string(),
            kind: "entity".to_string(),
            content: String::new(),
            metadata: serde_json::json!({
                "resolver_status": "not_found",
            }),
        },
        Err(e) => {
            log::debug!("[subconscious::source_chunk] resolve_entity db error for {raw}: {e}");
            SourceChunk {
                ref_id: raw.to_string(),
                kind: original_kind.clone(),
                content: String::new(),
                metadata: serde_json::json!({
                    "resolver_status": "db_error",
                }),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_ref_splits_on_first_colon() {
        assert_eq!(parse_ref("entity:phoenix"), ("entity", "phoenix"));
        assert_eq!(parse_ref("summary:abc:123"), ("summary", "abc:123"));
    }

    #[test]
    fn parse_ref_handles_missing_separator() {
        assert_eq!(parse_ref("loose-id"), ("unknown", "loose-id"));
    }

    #[test]
    fn truncate_preview_passes_through_short_text() {
        assert_eq!(truncate_preview("short"), "short");
    }

    #[test]
    fn truncate_preview_caps_long_text_with_ellipsis() {
        let long: String = "x".repeat(PREVIEW_MAX_CHARS + 50);
        let out = truncate_preview(&long);
        assert_eq!(out.chars().count(), PREVIEW_MAX_CHARS + 1);
        assert!(out.ends_with('…'));
    }
}
