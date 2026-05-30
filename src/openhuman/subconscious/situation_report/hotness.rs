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

    // 1. Read current hotness from the memory_tree DB. `is_user` joins
    //    against the entity index (#1365) so reflection generation can
    //    tell which movers are the user vs other people.
    let current = match read_current_hotness(config) {
        Ok(rows) => rows,
        Err(e) => {
            log::warn!("[subconscious::situation_report::hotness] read failed: {e}");
            return "## Hotness deltas\n\nHotness deltas unavailable.\n".to_string();
        }
    };

    if current.is_empty() {
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

    // 3. Compute deltas; carry is_user through.
    let mut deltas: Vec<HotnessDelta> = current
        .iter()
        .map(|row| {
            let prev = prev_map.get(&row.entity_id).copied().unwrap_or(0.0);
            HotnessDelta {
                entity_id: row.entity_id.clone(),
                score: row.score,
                delta: row.score - prev,
                is_user: row.is_user,
            }
        })
        .collect();
    // Highest |delta| first; ties broken by current score.
    deltas.sort_by(|a, b| {
        b.delta
            .abs()
            .partial_cmp(&a.delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| {
                b.score
                    .partial_cmp(&a.score)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
    });

    // 4. Format top-K.
    let top: Vec<&HotnessDelta> = deltas
        .iter()
        .filter(|d| d.delta.abs() > f64::EPSILON)
        .take(MAX_DELTAS)
        .collect();

    let mut section = String::from("## Hotness deltas\n\n");
    if top.is_empty() {
        section.push_str("No movement since last tick.\n");
    } else {
        let _ = writeln!(
            section,
            "Top {} entity movers (score = post-delta, Δ = change). \
             Items tagged `(you)` are the user's own identifiers — \
             reflect on these in second person; for everything else, \
             reflect on what *others* are doing or talking about.",
            top.len()
        );
        section.push('\n');
        for d in &top {
            let arrow = if d.delta > 0.0 { "▲" } else { "▼" };
            let self_marker = if d.is_user { " (you)" } else { "" };
            let _ = writeln!(
                section,
                "- {arrow} {eid}{self_marker} (score={score:.2}, Δ={delta:+.2})",
                eid = d.entity_id,
                score = d.score,
                delta = d.delta
            );
        }
    }

    // 5. Refresh snapshots.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0);
    let snapshot_pairs: Vec<(String, f64)> = current
        .iter()
        .map(|r| (r.entity_id.clone(), r.score))
        .collect();
    if let Err(e) = update_snapshots_with_now(workspace_dir, &snapshot_pairs, now) {
        log::warn!("[subconscious::situation_report::hotness] snapshot refresh failed: {e}");
    }

    section
}

/// One row from `read_current_hotness`. `is_user` is OR'd across all
/// indexed nodes for the entity — true if any mention of this entity in
/// the tree resolved against the Composio identity registry.
struct CurrentHotness {
    entity_id: String,
    score: f64,
    is_user: bool,
}

/// Internal: a delta row with the carry-through identity flag.
struct HotnessDelta {
    entity_id: String,
    score: f64,
    delta: f64,
    is_user: bool,
}

/// Read `(entity_id, last_hotness, is_user)` rows from the memory_tree
/// DB, filtering nulls. The `is_user` flag is computed via a correlated
/// subquery over `mem_tree_entity_index` (#1365): true iff any indexed
/// row for this entity has `is_user = 1`.
fn read_current_hotness(config: &Config) -> anyhow::Result<Vec<CurrentHotness>> {
    crate::openhuman::memory_store::chunks::store::with_connection(config, |conn| {
        let mut stmt = conn.prepare(
            "SELECT h.entity_id,
                    h.last_hotness,
                    EXISTS (
                        SELECT 1 FROM mem_tree_entity_index i
                         WHERE i.entity_id = h.entity_id
                           AND i.is_user = 1
                    ) AS is_user
               FROM mem_tree_entity_hotness h
              WHERE h.last_hotness IS NOT NULL
              ORDER BY h.last_hotness DESC",
        )?;
        let rows = stmt
            .query_map([], |row| {
                let id: String = row.get(0)?;
                let score: f64 = row.get(1)?;
                let is_user_int: i64 = row.get(2)?;
                Ok(CurrentHotness {
                    entity_id: id,
                    score,
                    is_user: is_user_int != 0,
                })
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
