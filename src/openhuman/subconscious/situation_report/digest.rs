//! Latest global L0 digest section (#623).
//!
//! The global tree's L0 nodes are daily digests. We fetch the most recent
//! one for the situation report. The body is truncated to keep prompt
//! footprint tight.
//!
//! Cutoff semantics: only the digest sealed *after* `last_tick_at` is
//! emitted. Without this gate the same digest gets re-rendered in every
//! tick's report verbatim, the LLM keeps citing its id, and
//! `persist_and_surface_reflections` (no insert-time dedupe) accumulates
//! near-duplicate reflections about the same digest forever — which is
//! exactly what was happening before this section was gated.

use crate::openhuman::config::Config;
use crate::openhuman::memory::tree::tree_source::types::TreeKind;

/// Truncate point for the digest body in the situation report.
const DIGEST_BODY_PREVIEW: usize = 1200;

pub async fn build_section(config: &Config, last_tick_at: f64) -> String {
    let cutoff_ms: i64 = if last_tick_at <= 0.0 {
        // Cold start — accept any digest. The summaries / query_window
        // sections do the same thing on cold start.
        0
    } else {
        (last_tick_at * 1000.0) as i64
    };
    log::debug!(
        "[subconscious::situation_report::digest] building section last_tick_at={last_tick_at} cutoff_ms={cutoff_ms}"
    );

    let row = match read_latest_global_l0(config, cutoff_ms) {
        Ok(Some(row)) => row,
        Ok(None) => {
            // Distinguish "no digest exists at all" from "digest exists
            // but hasn't advanced since last tick" — both render the
            // same to the LLM (no fresh content), but the log is
            // useful for diagnosing why it stopped citing the digest.
            log::debug!(
                "[subconscious::situation_report::digest] no digest sealed since cutoff_ms={cutoff_ms}"
            );
            return "## Latest daily digest\n\nNo new global digest sealed since last tick.\n"
                .to_string();
        }
        Err(e) => {
            log::warn!("[subconscious::situation_report::digest] read failed: {e}");
            return "## Latest daily digest\n\nDigest unavailable.\n".to_string();
        }
    };

    let preview = truncate(&row.content, DIGEST_BODY_PREVIEW);
    format!(
        "## Latest daily digest\n\nSealed at unix-ms {} (id={}):\n\n{}\n",
        row.sealed_at_ms, row.id, preview
    )
}

#[derive(Debug)]
struct DigestRow {
    id: String,
    content: String,
    sealed_at_ms: i64,
}

fn read_latest_global_l0(config: &Config, cutoff_ms: i64) -> anyhow::Result<Option<DigestRow>> {
    crate::openhuman::memory::tree::store::with_connection(config, |conn| {
        let row = conn
            .query_row(
                "SELECT s.id, s.content, s.sealed_at_ms
                 FROM mem_tree_summaries s
                 JOIN mem_tree_trees t ON t.id = s.tree_id
                 WHERE t.kind = ?1 AND s.level = 0 AND s.deleted = 0
                   AND s.sealed_at_ms > ?2
                 ORDER BY s.sealed_at_ms DESC LIMIT 1",
                rusqlite::params![tree_kind_global_str(), cutoff_ms],
                |row| {
                    Ok(DigestRow {
                        id: row.get(0)?,
                        content: row.get(1)?,
                        sealed_at_ms: row.get(2)?,
                    })
                },
            )
            .ok();
        Ok(row)
    })
}

/// Stable wire string for `TreeKind::Global` as persisted by the
/// memory_tree's `tree_source` writer. Centralised here so a future
/// rename in the source-of-truth lands in one place.
fn tree_kind_global_str() -> &'static str {
    // `TreeKind` serialises via serde with rename_all = "snake_case",
    // so `Global` -> "global". Keep the constant explicit (rather than
    // round-tripping serde at runtime) so the prompt section is cheap.
    let _kind_check = TreeKind::Global;
    "global"
}

fn truncate(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push('…');
    out
}
