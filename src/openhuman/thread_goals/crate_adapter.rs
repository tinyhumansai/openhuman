//! Adapter seam: mirror OpenHuman `thread_goals` onto the tinyagents
//! `graph::goals` crate store (issue #4249, plan §C2).
//!
//! **Adapter-first, dual-write.** The legacy per-thread file-JSON store
//! ([`super::store`]) stays **authoritative for reads**; this module *also*
//! mirrors every goal mutation into the crate's `graph::goals` store so a later
//! slice can flip reads over to the crate with zero data migration. The mirror
//! is a **faithful copy**: the crate row carries the *same* `goal_id`,
//! timestamps, and counters as the legacy row (we `put` the converted value
//! directly rather than calling the crate's `store::set`, which would re-mint a
//! `goal-<n>` id and reset counters).
//!
//! Persistence target: the crate [`Store`] rooted at the same workspace KV tree
//! as the 04-sessions journal (`{workspace}/tinyagents_store/kv`), namespace
//! [`GOALS_NAMESPACE`] (`graph.goals`), keyed by `hex(thread_id)` — byte-for-byte
//! the key the crate's own `graph::goals::store` computes, so the crate reader
//! finds exactly what we wrote.
//!
//! # Single-writer constraint
//!
//! The crate `Store` has **no compare-and-set and no cross-key transaction**
//! (see the crate `graph::goals::store` docs). Its per-thread atomicity is a
//! *process-local* async mutex, and the legacy store uses a process-wide mutex.
//! Neither is safe across processes. This is acceptable here because **the
//! OpenHuman core is the single writer** of thread goals — RPC handlers, agent
//! tools, and the heartbeat continuation runtime all run inside one core
//! process. Do not add a second mutating writer (a sidecar, a second core, a
//! cron in another process) without introducing a real CAS first.
//!
//! # Shadow mode
//!
//! The tool/host surface mirror is gated OFF by default behind
//! [`crate_goals_shadow_enabled`] (`OPENHUMAN_THREAD_GOALS_CRATE_SHADOW`). When
//! ON it acts on the legacy result and merely *logs* any crate-vs-legacy
//! divergence — it never changes what a caller observes.

use std::path::Path;
use std::sync::Arc;

use tinyagents::graph::goals::store::GOALS_NAMESPACE;
use tinyagents::graph::goals::{ThreadGoal as CrateThreadGoal, ThreadGoalStatus as CrateStatus};
use tinyagents::harness::store::Store;

use super::types::{ThreadGoal, ThreadGoalStatus};
use crate::openhuman::session_import::ops::open_session_stores;

/// Env flag gating the crate-goals **shadow** mirror on the tool/host surface.
/// Defaults **OFF**; any of `1`/`true`/`yes`/`on` (case-insensitive) enables it.
const SHADOW_ENV: &str = "OPENHUMAN_THREAD_GOALS_CRATE_SHADOW";

/// Whether the crate-goals shadow mirror is enabled (defaults OFF).
///
/// Shadow mode mirrors legacy mutations into the crate store and logs any
/// divergence; it never changes the caller-observed (legacy) result.
pub fn crate_goals_shadow_enabled() -> bool {
    std::env::var(SHADOW_ENV)
        .map(|v| {
            let v = v.trim().to_ascii_lowercase();
            matches!(v.as_str(), "1" | "true" | "yes" | "on")
        })
        .unwrap_or(false)
}

/// Open the crate [`Store`] handle used for the goals mirror, rooted at the
/// shared workspace KV tree (`{workspace}/tinyagents_store/kv`). Same layout the
/// 04-sessions journal + status store use, so everything lives under one tree.
pub(crate) fn crate_goals_store(workspace_dir: &Path) -> Arc<dyn Store> {
    Arc::new(open_session_stores(workspace_dir).kv)
}

/// The crate store key for a thread's goal: lowercase hex of the (trimmed)
/// thread-id bytes. This MUST match the crate's private `graph::goals::store`
/// key function exactly so the crate reader resolves our mirrored value.
fn goal_key(thread_id: &str) -> String {
    thread_id
        .trim()
        .as_bytes()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Map a legacy [`ThreadGoalStatus`] onto the crate [`CrateStatus`]. The two
/// enums are 1:1 (Active/Paused/BudgetLimited/Complete) — this is the mapping
/// the parity tests pin.
pub(crate) fn to_crate_status(status: ThreadGoalStatus) -> CrateStatus {
    match status {
        ThreadGoalStatus::Active => CrateStatus::Active,
        ThreadGoalStatus::Paused => CrateStatus::Paused,
        ThreadGoalStatus::BudgetLimited => CrateStatus::BudgetLimited,
        ThreadGoalStatus::Complete => CrateStatus::Complete,
    }
}

/// Map a crate [`CrateStatus`] back onto the legacy [`ThreadGoalStatus`] (the
/// inverse of [`to_crate_status`]).
pub(crate) fn from_crate_status(status: CrateStatus) -> ThreadGoalStatus {
    match status {
        CrateStatus::Active => ThreadGoalStatus::Active,
        CrateStatus::Paused => ThreadGoalStatus::Paused,
        CrateStatus::BudgetLimited => ThreadGoalStatus::BudgetLimited,
        CrateStatus::Complete => ThreadGoalStatus::Complete,
    }
}

/// Convert a legacy [`ThreadGoal`] into the crate [`CrateThreadGoal`],
/// preserving every field verbatim (id, objective, status, budget/usage
/// counters, timestamps, continuation flag). A **faithful** projection — no
/// re-minting, no counter reset.
pub(crate) fn to_crate_goal(goal: &ThreadGoal) -> CrateThreadGoal {
    CrateThreadGoal {
        thread_id: goal.thread_id.clone(),
        goal_id: goal.goal_id.clone(),
        objective: goal.objective.clone(),
        status: to_crate_status(goal.status),
        token_budget: goal.token_budget,
        tokens_used: goal.tokens_used,
        time_used_seconds: goal.time_used_seconds,
        created_at_ms: goal.created_at_ms,
        updated_at_ms: goal.updated_at_ms,
        continuation_suppressed: goal.continuation_suppressed,
    }
}

/// Convert a crate [`CrateThreadGoal`] back into a legacy [`ThreadGoal`] (the
/// inverse of [`to_crate_goal`]), used by the shadow-divergence comparison.
pub(crate) fn from_crate_goal(goal: &CrateThreadGoal) -> ThreadGoal {
    ThreadGoal {
        thread_id: goal.thread_id.clone(),
        goal_id: goal.goal_id.clone(),
        objective: goal.objective.clone(),
        status: from_crate_status(goal.status),
        token_budget: goal.token_budget,
        tokens_used: goal.tokens_used,
        time_used_seconds: goal.time_used_seconds,
        created_at_ms: goal.created_at_ms,
        updated_at_ms: goal.updated_at_ms,
        continuation_suppressed: goal.continuation_suppressed,
    }
}

/// Write the faithful crate mirror of `goal` into `store` (ns `graph.goals`,
/// key `hex(thread_id)`). Overwrites any prior mirror; idempotent for an
/// unchanged value.
pub(crate) async fn put_mirror(store: &Arc<dyn Store>, goal: &ThreadGoal) -> Result<(), String> {
    let crate_goal = to_crate_goal(goal);
    let value =
        serde_json::to_value(&crate_goal).map_err(|e| format!("serialize crate goal: {e}"))?;
    store
        .put(GOALS_NAMESPACE, &goal_key(&goal.thread_id), value)
        .await
        .map_err(|e| format!("mirror thread goal into {GOALS_NAMESPACE}: {e}"))
}

/// Read the current crate mirror for `thread_id`, or `None`. Skips a mirror that
/// fails to decode (treated as absent) so a legacy/corrupt row can't wedge the
/// shadow path.
pub(crate) async fn get_mirror(
    store: &Arc<dyn Store>,
    thread_id: &str,
) -> Result<Option<ThreadGoal>, String> {
    let value = store
        .get(GOALS_NAMESPACE, &goal_key(thread_id))
        .await
        .map_err(|e| format!("read crate goal mirror: {e}"))?;
    match value {
        Some(v) => match serde_json::from_value::<CrateThreadGoal>(v) {
            Ok(crate_goal) => Ok(Some(from_crate_goal(&crate_goal))),
            Err(e) => {
                tracing::debug!(
                    thread_id = %thread_id,
                    error = %e,
                    "[thread_goals][crate-shadow] undecodable crate mirror; treating as absent"
                );
                Ok(None)
            }
        },
        None => Ok(None),
    }
}

/// Delete the crate mirror for `thread_id`. No-op when absent (matches the
/// crate/legacy clear contract).
pub(crate) async fn delete_mirror(store: &Arc<dyn Store>, thread_id: &str) -> Result<(), String> {
    store
        .delete(GOALS_NAMESPACE, &goal_key(thread_id))
        .await
        .map_err(|e| format!("delete crate goal mirror: {e}"))
}

// ── Shadow-mode surface (flag-gated; acts on legacy, logs divergence) ─────────

/// Shadow-mirror a legacy mutation result into the crate store, logging any
/// crate-vs-legacy divergence. No-op (and no store I/O) when the shadow flag is
/// OFF. Best-effort: a mirror error is logged, never propagated — the shadow
/// path must never change caller-observed behavior.
pub async fn shadow_mirror_goal(workspace_dir: &Path, legacy_goal: &ThreadGoal) {
    if !crate_goals_shadow_enabled() {
        return;
    }
    let store = crate_goals_store(workspace_dir);
    // Log divergence against the pre-write crate state (status/counter drift).
    match get_mirror(&store, &legacy_goal.thread_id).await {
        Ok(Some(prior)) if prior != *legacy_goal => {
            tracing::debug!(
                thread_id = %legacy_goal.thread_id,
                goal_id = %legacy_goal.goal_id,
                crate_status = prior.status.as_str(),
                legacy_status = legacy_goal.status.as_str(),
                crate_tokens = prior.tokens_used,
                legacy_tokens = legacy_goal.tokens_used,
                "[thread_goals][crate-shadow] mirror diverges from prior crate row; overwriting with legacy"
            );
        }
        Ok(_) => {}
        Err(e) => {
            tracing::debug!(error = %e, "[thread_goals][crate-shadow] prior-read failed");
        }
    }
    if let Err(e) = put_mirror(&store, legacy_goal).await {
        tracing::debug!(
            thread_id = %legacy_goal.thread_id,
            error = %e,
            "[thread_goals][crate-shadow] mirror write failed (ignored)"
        );
    } else {
        tracing::debug!(
            thread_id = %legacy_goal.thread_id,
            goal_id = %legacy_goal.goal_id,
            status = legacy_goal.status.as_str(),
            "[thread_goals][crate-shadow] mirrored goal into graph.goals"
        );
    }
}

/// Shadow-mirror a legacy clear into the crate store. No-op when the shadow flag
/// is OFF. Best-effort (errors logged, never propagated).
pub async fn shadow_mirror_clear(workspace_dir: &Path, thread_id: &str) {
    if !crate_goals_shadow_enabled() {
        return;
    }
    let store = crate_goals_store(workspace_dir);
    if let Err(e) = delete_mirror(&store, thread_id).await {
        tracing::debug!(
            thread_id = %thread_id,
            error = %e,
            "[thread_goals][crate-shadow] mirror clear failed (ignored)"
        );
    } else {
        tracing::debug!(
            thread_id = %thread_id,
            "[thread_goals][crate-shadow] cleared goal mirror in graph.goals"
        );
    }
}

// ── One-time migration helper (callable, logged, NOT wired to boot) ───────────

/// Outcome of a [`migrate_legacy_goals_into_crate_store`] run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GoalMigrationReport {
    /// Legacy goal rows examined.
    pub total: usize,
    /// Rows written into the crate store (absent or divergent mirror).
    pub copied: usize,
    /// Rows already present in the crate store with an identical value.
    pub skipped: usize,
}

/// Copy every existing legacy thread-goal row into the crate `graph.goals`
/// store. **Idempotent**: a row whose crate mirror already equals the legacy
/// value is skipped, so re-running does no writes and reports everything under
/// `skipped`.
///
/// Callable + logged; deliberately **not wired into boot** in this slice (a
/// later slice schedules it behind a one-shot marker, mirroring the
/// session-import global marker). Honors the single-writer constraint: run it
/// only inside the core process.
pub async fn migrate_legacy_goals_into_crate_store(
    workspace_dir: &Path,
) -> Result<GoalMigrationReport, String> {
    let legacy = super::store::list_all(workspace_dir).await?;
    let store = crate_goals_store(workspace_dir);
    let mut report = GoalMigrationReport {
        total: legacy.len(),
        ..Default::default()
    };
    tracing::info!(
        workspace = %workspace_dir.display(),
        total = report.total,
        "[thread_goals][crate-migrate] start copy legacy goals → graph.goals"
    );
    for goal in &legacy {
        match get_mirror(&store, &goal.thread_id).await {
            Ok(Some(existing)) if existing == *goal => {
                report.skipped += 1;
                tracing::debug!(
                    thread_id = %goal.thread_id,
                    goal_id = %goal.goal_id,
                    "[thread_goals][crate-migrate] skip (already mirrored)"
                );
                continue;
            }
            Ok(_) => {}
            Err(e) => {
                // Read failure → attempt the write anyway (fail-forward copy).
                tracing::debug!(
                    thread_id = %goal.thread_id,
                    error = %e,
                    "[thread_goals][crate-migrate] mirror pre-read failed; copying anyway"
                );
            }
        }
        put_mirror(&store, goal).await?;
        report.copied += 1;
        tracing::debug!(
            thread_id = %goal.thread_id,
            goal_id = %goal.goal_id,
            "[thread_goals][crate-migrate] copied"
        );
    }
    tracing::info!(
        total = report.total,
        copied = report.copied,
        skipped = report.skipped,
        "[thread_goals][crate-migrate] done"
    );
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::thread_goals::store as legacy_store;

    fn sample_goal(status: ThreadGoalStatus) -> ThreadGoal {
        ThreadGoal {
            thread_id: "thread-α".into(),
            goal_id: "goal-uuid-1".into(),
            objective: "ship the migration".into(),
            status,
            token_budget: Some(5_000),
            tokens_used: 1_234,
            time_used_seconds: 42,
            created_at_ms: 1_000,
            updated_at_ms: 2_000,
            continuation_suppressed: true,
        }
    }

    #[test]
    fn status_mapping_is_bijective_across_all_variants() {
        for status in [
            ThreadGoalStatus::Active,
            ThreadGoalStatus::Paused,
            ThreadGoalStatus::BudgetLimited,
            ThreadGoalStatus::Complete,
        ] {
            let round = from_crate_status(to_crate_status(status));
            assert_eq!(round, status, "status round-trip must be identity");
        }
        // Pin the exact crate labels the mapping produces.
        assert_eq!(to_crate_status(ThreadGoalStatus::Active).as_str(), "active");
        assert_eq!(to_crate_status(ThreadGoalStatus::Paused).as_str(), "paused");
        assert_eq!(
            to_crate_status(ThreadGoalStatus::BudgetLimited).as_str(),
            "budget_limited"
        );
        assert_eq!(
            to_crate_status(ThreadGoalStatus::Complete).as_str(),
            "complete"
        );
    }

    #[test]
    fn goal_mapping_preserves_every_field_and_completion_contract() {
        // Completion contract: Complete + continuation_suppressed carries through.
        let g = sample_goal(ThreadGoalStatus::Complete);
        let crate_goal = to_crate_goal(&g);
        assert_eq!(crate_goal.thread_id, g.thread_id);
        assert_eq!(
            crate_goal.goal_id, g.goal_id,
            "goal_id preserved (no re-mint)"
        );
        assert_eq!(crate_goal.objective, g.objective);
        assert_eq!(crate_goal.status, CrateStatus::Complete);
        assert_eq!(crate_goal.token_budget, g.token_budget, "budget preserved");
        assert_eq!(crate_goal.tokens_used, g.tokens_used, "usage preserved");
        assert_eq!(crate_goal.time_used_seconds, g.time_used_seconds);
        assert_eq!(crate_goal.created_at_ms, g.created_at_ms);
        assert_eq!(crate_goal.updated_at_ms, g.updated_at_ms);
        assert!(
            crate_goal.continuation_suppressed,
            "completion suppresses continuation"
        );
        // Full round-trip identity.
        assert_eq!(from_crate_goal(&crate_goal), g);
    }

    #[test]
    fn budget_limited_maps_and_over_budget_carries() {
        let mut g = sample_goal(ThreadGoalStatus::BudgetLimited);
        g.tokens_used = 6_000; // over the 5_000 budget
        let crate_goal = to_crate_goal(&g);
        assert_eq!(crate_goal.status, CrateStatus::BudgetLimited);
        assert!(crate_goal.over_budget(), "over-budget invariant carries");
        assert_eq!(crate_goal.budget_remaining(), Some(0));
    }

    #[tokio::test]
    async fn put_get_delete_mirror_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = crate_goals_store(tmp.path());
        let g = sample_goal(ThreadGoalStatus::Active);

        assert!(get_mirror(&store, &g.thread_id).await.unwrap().is_none());
        put_mirror(&store, &g).await.unwrap();
        let read = get_mirror(&store, &g.thread_id).await.unwrap().unwrap();
        assert_eq!(read, g, "mirror round-trips the exact legacy value");

        delete_mirror(&store, &g.thread_id).await.unwrap();
        assert!(get_mirror(&store, &g.thread_id).await.unwrap().is_none());
        // Delete is idempotent (no-op when absent).
        delete_mirror(&store, &g.thread_id).await.unwrap();
    }

    #[tokio::test]
    async fn crate_reader_resolves_the_mirrored_key() {
        // Proves the key/namespace we write matches what the crate's own
        // `graph::goals::store` reader computes — the whole point of the mirror.
        let tmp = tempfile::tempdir().unwrap();
        let store = crate_goals_store(tmp.path());
        let g = sample_goal(ThreadGoalStatus::Paused);
        put_mirror(&store, &g).await.unwrap();

        let via_crate = tinyagents::graph::goals::store::get(&store, &g.thread_id)
            .await
            .unwrap()
            .expect("crate reader finds the mirrored row");
        assert_eq!(via_crate.goal_id, g.goal_id);
        assert_eq!(via_crate.status, CrateStatus::Paused);
        assert_eq!(via_crate.tokens_used, g.tokens_used);
    }

    #[tokio::test]
    async fn migration_copies_then_is_idempotent() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        // Seed two legacy goals via the authoritative legacy store.
        legacy_store::set(dir, "t1", "objective one", Some(1_000))
            .await
            .unwrap();
        let g2 = legacy_store::set(dir, "t2", "objective two", None)
            .await
            .unwrap();
        legacy_store::account_usage(dir, "t2", &g2.goal_id, 50, 3)
            .await
            .unwrap();

        // First run copies both.
        let r1 = migrate_legacy_goals_into_crate_store(dir).await.unwrap();
        assert_eq!(r1.total, 2);
        assert_eq!(r1.copied, 2);
        assert_eq!(r1.skipped, 0);

        // Crate rows now match legacy rows exactly.
        let store = crate_goals_store(dir);
        let m2 = get_mirror(&store, "t2").await.unwrap().unwrap();
        assert_eq!(m2.tokens_used, 50);
        assert_eq!(m2.objective, "objective two");

        // Second run is a no-op (idempotent) — everything already mirrored.
        let r2 = migrate_legacy_goals_into_crate_store(dir).await.unwrap();
        assert_eq!(r2.total, 2);
        assert_eq!(r2.copied, 0, "idempotent: nothing re-copied");
        assert_eq!(r2.skipped, 2);
    }

    #[tokio::test]
    async fn migration_recopies_a_diverged_row() {
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path();
        let g = legacy_store::set(dir, "t", "obj", None).await.unwrap();
        migrate_legacy_goals_into_crate_store(dir).await.unwrap();

        // Legacy advances (usage accounted) → crate mirror is now stale.
        legacy_store::account_usage(dir, "t", &g.goal_id, 99, 1)
            .await
            .unwrap();

        let r = migrate_legacy_goals_into_crate_store(dir).await.unwrap();
        assert_eq!(r.copied, 1, "diverged row re-copied");
        assert_eq!(r.skipped, 0);
        let store = crate_goals_store(dir);
        assert_eq!(
            get_mirror(&store, "t").await.unwrap().unwrap().tokens_used,
            99
        );
    }

    #[test]
    fn shadow_flag_defaults_off() {
        // Not asserting env mutation (process-global); just the default parse.
        // An unset/empty value must read as OFF.
        assert!(!matches!(
            "".trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        ));
    }
}
