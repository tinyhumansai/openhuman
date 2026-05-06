//! Hotness deltas section — top-K entities whose `mem_tree_entity_hotness`
//! score moved meaningfully since the last tick (#623).
//!
//! Joins the live hotness table against the `subconscious_hotness_snapshots`
//! table populated at the end of each tick. Returns the top 10 movers by
//! absolute delta. After formatting, refreshes the snapshots so the next
//! tick has a fresh baseline.
//!
//! Failure is non-fatal — any DB error returns a "Hotness deltas
//! unavailable" stub so the rest of the situation report still renders.

use std::fmt::Write;
use std::path::Path;

use crate::openhuman::config::Config;
use crate::openhuman::subconscious::reflection_store;
use crate::openhuman::subconscious::store as subconscious_store;

/// Maximum entries to render in the section.
const MAX_DELTAS: usize = 10;

pub async fn build_section(config: &Config, workspace_dir: &Path, _last_tick_at: f64) -> String {
    log::debug!("[subconscious::situation_report::hotness] building section");

    // 1. Read current hotness from the memory_tree DB.
    let current = match read_current_hotness(config) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("[subconscious::situation_report::hotness] read failed: {e}");
            return "## Hotness deltas\n\nHotness deltas unavailable.\n".to_string();
        }
    };

    if current.is_empty() {
        // Refresh snapshots to empty so future deltas are honest.
        let _ = update_snapshots(workspace_dir, &[]);
        return "## Hotness deltas\n\nNo entity hotness data yet.\n".to_string();
    }

    // 2. Read previous snapshot.
    let previous = subconscious_store::with_connection(workspace_dir, |conn| {
        reflection_store::load_hotness_snapshots(conn)
    })
    .unwrap_or_else(|e| {
        log::warn!("[subconscious::situation_report::hotness] snapshot load failed: {e}");
        Vec::new()
    });
    let prev_map: std::collections::HashMap<String, f64> = previous.into_iter().collect();

    // 3. Compute deltas.
    let mut deltas: Vec<(String, f64, f64)> = current
        .iter()
        .map(|(eid, score)| {
            let prev = prev_map.get(eid).copied().unwrap_or(0.0);
            (eid.clone(), *score, score - prev)
        })
        .collect();
    // Highest |delta| first; ties broken by current score.
    deltas.sort_by(|a, b| {
        b.2.abs()
            .partial_cmp(&a.2.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal))
    });

    // 4. Format top-K.
    let top: Vec<&(String, f64, f64)> = deltas
        .iter()
        .filter(|(_, _, delta)| delta.abs() > f64::EPSILON)
        .take(MAX_DELTAS)
        .collect();

    let mut section = String::from("## Hotness deltas\n\n");
    if top.is_empty() {
        section.push_str("No movement since last tick.\n");
    } else {
        let _ = writeln!(
            section,
            "Top {} entity movers (score = post-delta, Δ = change):",
            top.len()
        );
        section.push('\n');
        for (eid, score, delta) in &top {
            let arrow = if *delta > 0.0 { "▲" } else { "▼" };
            let _ = writeln!(section, "- {arrow} {eid} (score={score:.2}, Δ={delta:+.2})");
        }
    }

    // 5. Refresh snapshots.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    if let Err(e) = update_snapshots_with_now(workspace_dir, &current, now) {
        log::warn!("[subconscious::situation_report::hotness] snapshot refresh failed: {e}");
    }

    section
}

/// Read `(entity_id, last_hotness)` rows from the memory_tree DB,
/// filtering nulls. Returns rows ordered by score desc.
fn read_current_hotness(config: &Config) -> anyhow::Result<Vec<(String, f64)>> {
    crate::openhuman::memory::tree::store::with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT entity_id, last_hotness FROM mem_tree_entity_hotness
             WHERE last_hotness IS NOT NULL
             ORDER BY last_hotness DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let score: f64 = row.get(1)?;
                Ok((id, score))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    })
}

/// Refresh the snapshot table. Wrapper that captures `now` once.
fn update_snapshots(workspace_dir: &Path, snapshots: &[(String, f64)]) -> anyhow::Result<()> {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    update_snapshots_with_now(workspace_dir, snapshots, now)
}

fn update_snapshots_with_now(
    workspace_dir: &Path,
    snapshots: &[(String, f64)],
    now: f64,
) -> anyhow::Result<()> {
    // The closure-based `with_connection` API does not expose a `&mut Connection`
    // — we need one for the transaction in `replace_hotness_snapshots`.
    // Open a direct handle just for this write. Schema is a no-op since
    // the table already exists; we just need the migration to be applied
    // (callers always go through `with_connection` first, so the migration
    // ran by the time we get here).
    let db_path = workspace_dir.join("subconscious").join("subconscious.db");
    let mut conn = rusqlite::Connection::open(&db_path)?;
    reflection_store::replace_hotness_snapshots(&mut conn, snapshots, now)?;
    Ok(())
}
