//! Recently-sealed summaries section (#623).
//!
//! Reads `mem_tree_summaries` rows sealed since the last tick, grouped
//! by their parent tree's scope label, and emits a markdown bullet list.

use std::fmt::Write;

use crate::openhuman::config::Config;

/// Hard ceiling on rows fetched. The tick LLM only needs a bounded
/// pre-cooked recap — anything beyond ~8 entries is noise.
const MAX_SUMMARIES: usize = 8;

/// Per-summary content cap — keep prompts compact.
const SUMMARY_CONTENT_PREVIEW: usize = 320;

pub async fn build_section(config: &Config, last_tick_at: f64) -> String {
    log::debug!(
        "[subconscious::situation_report::summaries] building section last_tick_at={last_tick_at}"
    );

    // Cold start gates everything in by widening the cutoff to 0.
    let cutoff_ms: i64 = if last_tick_at <= 0.0 {
        0
    } else {
        (last_tick_at * 1000.0) as i64
    };

    let rows = match read_recent_summaries(config, cutoff_ms) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("[subconscious::situation_report::summaries] read failed: {e}");
            return "## Recent summaries\n\nSummaries unavailable.\n".to_string();
        }
    };

    if rows.is_empty() {
        return "## Recent summaries\n\nNo new sealed summaries since last tick.\n".to_string();
    }

    let mut section = String::from("## Recent summaries\n\n");
    let _ = writeln!(
        section,
        "{} summaries sealed since last tick (most recent first):",
        rows.len()
    );
    section.push('\n');
    for row in &rows {
        let preview = truncate(&row.content, SUMMARY_CONTENT_PREVIEW);
        let _ = writeln!(
            section,
            "- **[{}]** L{} {} — {}",
            row.tree_scope, row.level, row.summary_id, preview
        );
    }
    section
}

#[derive(Debug)]
struct SummaryRow {
    summary_id: String,
    tree_scope: String,
    level: u32,
    content: String,
}

fn read_recent_summaries(config: &Config, cutoff_ms: i64) -> anyhow::Result<Vec<SummaryRow>> {
    crate::openhuman::memory::tree::store::with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT s.id, s.level, s.content, t.scope
             FROM mem_tree_summaries s
             JOIN mem_tree_trees t ON t.id = s.tree_id
             WHERE s.sealed_at_ms > ?1 AND s.deleted = 0
             ORDER BY s.sealed_at_ms DESC
             LIMIT ?2",
        )?;
        let rows = stmt
            .query_map(rusqlite::params![cutoff_ms, MAX_SUMMARIES as i64], |row| {
                Ok(SummaryRow {
                    summary_id: row.get(0)?,
                    level: row.get::<_, i64>(1)? as u32,
                    content: row.get(2)?,
                    tree_scope: row.get(3)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

fn truncate(text: &str, max_chars: usize) -> String {
    let trimmed = text.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.replace('\n', " ");
    }
    let mut out: String = trimmed.chars().take(max_chars).collect();
    out.push('…');
    out.replace('\n', " ")
}
