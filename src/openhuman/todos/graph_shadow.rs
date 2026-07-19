//! Shadow adapter: mirror the OpenHuman task board into the vendored
//! `tinyagents::graph::todos` crate `TaskBoard` (crate `Store` namespace
//! `graph.todos`).
//!
//! ADAPTER-FIRST / SHADOW ONLY — nothing in this module changes product
//! behavior. The legacy [`TaskBoardStore`](crate::openhuman::agent::task_board)
//! + [`todos::ops`](crate::openhuman::todos::ops) remain the single source of truth.
//!
//! This module (C2b first slice) mirrors post-mutation card snapshots
//! into a crate `Store` and shadow-runs the crate `claim_card` CAS purely to
//! prove parity ahead of the C2 cutover, logging any divergence. All work is
//! best-effort and fire-and-forget: a mirror/claim failure is logged and
//! swallowed, never surfaced to a caller.
//!
//! # Status mapping
//! The OpenHuman and crate `TaskCardStatus` enums are 1:1
//! (`Todo`/`AwaitingApproval`/`Ready`/`InProgress`/`Blocked`/`Done`/`Rejected`),
//! so [`map_status_to_crate`] is total and lossless.
//!
//! # Known OpenHuman ↔ crate semantic divergences (logged, not reconciled)
//! - **Scratch boards.** OpenHuman has an in-memory, thread-less
//!   [`BoardLocation::Scratch`](crate::openhuman::todos::ops::BoardLocation)
//!   fallback (tool calls outside a chat thread). The crate task board is
//!   always `(Store, thread_id)`, so scratch mutations have no mirror target
//!   and are skipped (trace-logged).
//! - **Card id minting.** OpenHuman `normalise_board` mints missing ids as
//!   `task-<uuid>`; the crate mints `task-<seq>`. We pass ids through unchanged
//!   so an already-persisted board round-trips, but a brand-new blank id would
//!   diverge — logged if observed.
//! - **Timestamps.** OpenHuman stores `updated_at` as RFC3339; the crate stores
//!   unix-epoch millis. Cosmetic; the mirror does not attempt to reconcile.
//! - **Single writer.** The crate `Store` has no compare-and-set, so both the
//!   mirror and the shadow-claim assume the core process is the only writer of
//!   ns `graph.todos` (it is). Concurrent shadow tasks converge to the latest
//!   legacy state; only the log-line ordering is nondeterministic.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use tinyagents::graph::todos::store as crate_todos;
use tinyagents::graph::todos::{
    TaskApprovalMode as CrateApprovalMode, TaskBoardCard as CrateCard,
    TaskCardStatus as CrateStatus,
};
use tinyagents::harness::store::{FileStore, Store};

use crate::openhuman::agent::task_board::{
    TaskApprovalMode as OhApprovalMode, TaskBoardCard as OhCard, TaskCardStatus as OhStatus,
};
use crate::openhuman::todos::ops::BoardLocation;

/// Sub-directory of the workspace holding the crate `FileStore` that backs the
/// shadow `graph.todos` namespace. Kept separate from the authoritative
/// `agent_task_boards/` JSON so the shadow never collides with product state.
const SHADOW_STORE_DIR: &str = "tinyagents_graph_store";

/// Maps an OpenHuman [`OhStatus`] to the crate [`CrateStatus`]. Total (the two
/// enums share the same seven variants).
pub(crate) fn map_status_to_crate(status: &OhStatus) -> CrateStatus {
    match status {
        OhStatus::Todo => CrateStatus::Todo,
        OhStatus::AwaitingApproval => CrateStatus::AwaitingApproval,
        OhStatus::Ready => CrateStatus::Ready,
        OhStatus::InProgress => CrateStatus::InProgress,
        OhStatus::Blocked => CrateStatus::Blocked,
        OhStatus::Done => CrateStatus::Done,
        OhStatus::Rejected => CrateStatus::Rejected,
    }
}

/// Maps a crate [`CrateStatus`] back to an OpenHuman [`OhStatus`]. Total; used
/// only to compare a shadow-claim result against the legacy outcome.
pub(crate) fn map_status_from_crate(status: CrateStatus) -> OhStatus {
    match status {
        CrateStatus::Todo => OhStatus::Todo,
        CrateStatus::AwaitingApproval => OhStatus::AwaitingApproval,
        CrateStatus::Ready => OhStatus::Ready,
        CrateStatus::InProgress => OhStatus::InProgress,
        CrateStatus::Blocked => OhStatus::Blocked,
        CrateStatus::Done => OhStatus::Done,
        CrateStatus::Rejected => OhStatus::Rejected,
    }
}

fn map_approval_mode(mode: &OhApprovalMode) -> CrateApprovalMode {
    match mode {
        OhApprovalMode::Required => CrateApprovalMode::Required,
        OhApprovalMode::NotRequired => CrateApprovalMode::NotRequired,
    }
}

/// Converts an OpenHuman [`OhCard`] into the crate [`CrateCard`], preserving the
/// id, status, and all optional metadata so a persisted board round-trips.
pub(crate) fn to_crate_card(card: &OhCard) -> CrateCard {
    CrateCard {
        id: card.id.clone(),
        title: card.title.clone(),
        status: map_status_to_crate(&card.status),
        objective: card.objective.clone(),
        plan: card.plan.clone(),
        assigned_agent: card.assigned_agent.clone(),
        allowed_tools: card.allowed_tools.clone(),
        approval_mode: card.approval_mode.as_ref().map(map_approval_mode),
        acceptance_criteria: card.acceptance_criteria.clone(),
        evidence: card.evidence.clone(),
        notes: card.notes.clone(),
        blocker: card.blocker.clone(),
        session_thread_id: card.session_thread_id.clone(),
        source_metadata: card.source_metadata.clone(),
        order: card.order,
        updated_at: card.updated_at.clone(),
    }
}

/// Builds the crate `Store` rooted at `<workspace>/tinyagents_graph_store`.
pub(crate) fn crate_store_for(workspace_dir: &Path) -> Arc<dyn Store> {
    Arc::new(FileStore::new(workspace_dir.join(SHADOW_STORE_DIR)))
}

/// Returns `(workspace_dir, thread_id)` for a mirrorable `Thread` board, or
/// `None` for the thread-less `Scratch` board (which has no crate target).
fn thread_target(location: &BoardLocation) -> Option<(PathBuf, String)> {
    match location {
        BoardLocation::Thread {
            workspace_dir,
            thread_id,
        } => Some((workspace_dir.clone(), thread_id.clone())),
        BoardLocation::Scratch => None,
    }
}

/// Fire-and-forget: mirror the post-mutation `cards` for `location` into the
/// crate `graph.todos` store. No-op for scratch boards or when no tokio runtime
/// is available (e.g. a sync unit test). Never affects the caller.
pub(crate) fn spawn_mirror(location: &BoardLocation, cards: &[OhCard]) {
    let Some((workspace_dir, thread_id)) = thread_target(location) else {
        tracing::trace!("[todos][graph-shadow] mirror skipped: scratch board has no crate target");
        return;
    };
    let crate_cards: Vec<CrateCard> = cards.iter().map(to_crate_card).collect();
    let in_progress = crate_cards
        .iter()
        .filter(|c| matches!(c.status, CrateStatus::InProgress))
        .count();
    spawn_best_effort(async move {
        let store = crate_store_for(&workspace_dir);
        match crate_todos::replace(&store, &thread_id, crate_cards).await {
            Ok(snap) => {
                tracing::debug!(
                    thread_id = %thread_id,
                    card_count = snap.cards.len(),
                    "[todos][graph-shadow] mirror ok"
                );
            }
            Err(e) => {
                // The most likely divergence is the single-InProgress invariant:
                // the crate rejects >1 in-progress. Product enforces the same
                // rule before save, so a rejection here flags a real mismatch.
                tracing::warn!(
                    thread_id = %thread_id,
                    in_progress,
                    error = %e,
                    "[todos][graph-shadow] mirror DIVERGENCE (crate replace rejected)"
                );
            }
        }
    });
}

/// Fire-and-forget shadow of a `claim_card` CAS. Mirrors `pre_cards` (the board
/// as loaded, before the legacy claim mutated it) into the crate store, replays
/// the crate `claim_card`, and logs whether the crate outcome agrees with the
/// authoritative `legacy_ok`. Log-only: the legacy claim stays authoritative.
pub(crate) fn spawn_shadow_claim(
    location: &BoardLocation,
    pre_cards: Vec<OhCard>,
    card_id: &str,
    expected: Vec<OhStatus>,
    target: OhStatus,
    legacy_ok: bool,
) {
    let Some((workspace_dir, thread_id)) = thread_target(location) else {
        tracing::trace!(
            "[todos][graph-shadow] shadow-claim skipped: scratch board has no crate target"
        );
        return;
    };
    let card_id = card_id.to_string();
    let crate_cards: Vec<CrateCard> = pre_cards.iter().map(to_crate_card).collect();
    let crate_expected: Vec<CrateStatus> = expected.iter().map(map_status_to_crate).collect();
    let crate_target = map_status_to_crate(&target);
    spawn_best_effort(async move {
        let store = crate_store_for(&workspace_dir);
        // Seed the crate board with the pre-claim snapshot so the CAS runs
        // against the same state the legacy claim saw (deterministic regardless
        // of any concurrent mirror task).
        if let Err(e) = crate_todos::replace(&store, &thread_id, crate_cards).await {
            tracing::debug!(
                thread_id = %thread_id,
                card_id = %card_id,
                error = %e,
                "[todos][graph-shadow] shadow-claim seed replace failed; skipping compare"
            );
            return;
        }
        let crate_result =
            crate_todos::claim_card(&store, &thread_id, &card_id, &crate_expected, crate_target)
                .await;
        let crate_ok = crate_result.is_ok();
        if crate_ok == legacy_ok {
            tracing::debug!(
                thread_id = %thread_id,
                card_id = %card_id,
                outcome_ok = legacy_ok,
                "[todos][graph-shadow] shadow-claim parity"
            );
        } else {
            tracing::warn!(
                thread_id = %thread_id,
                card_id = %card_id,
                legacy_ok,
                crate_ok,
                crate_err = crate_result.as_ref().err().map(|e| e.to_string()),
                "[todos][graph-shadow] shadow-claim DIVERGENCE (legacy vs crate CAS disagree)"
            );
        }
    });
}

/// Spawns `fut` onto the current tokio runtime if one exists; otherwise
/// trace-logs and drops it. Keeps the shadow entirely off the caller's path.
fn spawn_best_effort<F>(fut: F)
where
    F: std::future::Future<Output = ()> + Send + 'static,
{
    match tokio::runtime::Handle::try_current() {
        Ok(handle) => {
            handle.spawn(fut);
        }
        Err(_) => {
            tracing::trace!(
                "[todos][graph-shadow] no tokio runtime; shadow task skipped (sync context)"
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn oh_card(id: &str, status: OhStatus) -> OhCard {
        OhCard {
            id: id.to_string(),
            title: format!("card {id}"),
            status,
            objective: Some("obj".to_string()),
            plan: vec!["step-1".to_string()],
            assigned_agent: Some("planner".to_string()),
            allowed_tools: vec!["todo".to_string()],
            approval_mode: Some(OhApprovalMode::Required),
            acceptance_criteria: vec!["tests pass".to_string()],
            evidence: vec!["cargo test".to_string()],
            notes: Some("note".to_string()),
            blocker: None,
            session_thread_id: Some("thread-x".to_string()),
            source_metadata: Some(serde_json::json!({ "urgency": 0.5 })),
            order: 3,
            updated_at: "2026-07-03T00:00:00Z".to_string(),
        }
    }

    #[test]
    fn status_mapping_is_total_and_round_trips() {
        let all = [
            OhStatus::Todo,
            OhStatus::AwaitingApproval,
            OhStatus::Ready,
            OhStatus::InProgress,
            OhStatus::Blocked,
            OhStatus::Done,
            OhStatus::Rejected,
        ];
        for oh in all {
            let crate_status = map_status_to_crate(&oh);
            // The stable string label must survive the mapping unchanged.
            assert_eq!(oh.as_str(), crate_status.as_str());
            // And the mapping must round-trip losslessly.
            assert_eq!(map_status_from_crate(crate_status), oh);
        }
    }

    #[test]
    fn card_conversion_preserves_all_fields() {
        let oh = oh_card("task-1", OhStatus::InProgress);
        let c = to_crate_card(&oh);
        assert_eq!(c.id, "task-1");
        assert_eq!(c.title, "card task-1");
        assert_eq!(c.status, CrateStatus::InProgress);
        assert_eq!(c.objective.as_deref(), Some("obj"));
        assert_eq!(c.plan, vec!["step-1".to_string()]);
        assert_eq!(c.assigned_agent.as_deref(), Some("planner"));
        assert_eq!(c.allowed_tools, vec!["todo".to_string()]);
        assert_eq!(c.approval_mode, Some(CrateApprovalMode::Required));
        assert_eq!(c.acceptance_criteria, vec!["tests pass".to_string()]);
        assert_eq!(c.evidence, vec!["cargo test".to_string()]);
        assert_eq!(c.notes.as_deref(), Some("note"));
        assert_eq!(c.session_thread_id.as_deref(), Some("thread-x"));
        assert_eq!(
            c.source_metadata,
            Some(serde_json::json!({ "urgency": 0.5 }))
        );
        assert_eq!(c.order, 3);
        assert_eq!(c.updated_at, "2026-07-03T00:00:00Z");
    }

    #[test]
    fn approval_mode_maps_both_variants() {
        assert_eq!(
            map_approval_mode(&OhApprovalMode::Required),
            CrateApprovalMode::Required
        );
        assert_eq!(
            map_approval_mode(&OhApprovalMode::NotRequired),
            CrateApprovalMode::NotRequired
        );
    }

    #[test]
    fn scratch_board_has_no_crate_target() {
        assert!(thread_target(&BoardLocation::Scratch).is_none());
        let loc = BoardLocation::Thread {
            workspace_dir: PathBuf::from("/tmp/ws"),
            thread_id: "user-tasks".to_string(),
        };
        let (ws, tid) = thread_target(&loc).expect("thread target");
        assert_eq!(ws, PathBuf::from("/tmp/ws"));
        assert_eq!(tid, "user-tasks");
    }

    /// The crate `store::replace` mirror path applied end-to-end: mapping a
    /// legacy board of OpenHuman cards into the crate store yields a crate board
    /// whose statuses and ids match, proving the mirror adapter round-trips.
    #[tokio::test]
    async fn mirror_round_trips_through_crate_store() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = crate_store_for(dir.path());
        let cards = vec![
            oh_card("task-a", OhStatus::Todo),
            oh_card("task-b", OhStatus::InProgress),
            oh_card("task-c", OhStatus::Blocked),
        ];
        let crate_cards: Vec<CrateCard> = cards.iter().map(to_crate_card).collect();
        let snap = crate_todos::replace(&store, "user-tasks", crate_cards)
            .await
            .expect("replace ok");
        assert_eq!(snap.cards.len(), 3);
        assert_eq!(snap.cards[0].id, "task-a");
        assert_eq!(snap.cards[1].status, CrateStatus::InProgress);
        assert_eq!(snap.cards[2].status, CrateStatus::Blocked);

        // A re-read via the crate list op returns the same board.
        let listed = crate_todos::list(&store, "user-tasks")
            .await
            .expect("list ok");
        assert_eq!(listed.cards.len(), 3);
    }

    /// The single-InProgress invariant is shared: a legacy board that already
    /// violates it (two in-progress) is rejected by the crate mirror exactly as
    /// the product `enforce_single_in_progress` would — the divergence the
    /// mirror is built to surface.
    #[tokio::test]
    async fn crate_mirror_rejects_double_in_progress_like_product() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = crate_store_for(dir.path());
        let cards = vec![
            to_crate_card(&oh_card("task-a", OhStatus::InProgress)),
            to_crate_card(&oh_card("task-b", OhStatus::InProgress)),
        ];
        let err = crate_todos::replace(&store, "user-tasks", cards)
            .await
            .expect_err("double in-progress must be rejected");
        assert!(
            err.to_string().contains("in_progress"),
            "unexpected error: {err}"
        );
    }

    /// The crate `claim_card` CAS agrees with the legacy claim contract:
    /// claiming a `Todo` card to `InProgress` succeeds, and a second claim
    /// expecting `Todo` is rejected because the card already moved on.
    #[tokio::test]
    async fn crate_claim_cas_matches_legacy_contract() {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = crate_store_for(dir.path());
        let cards = vec![to_crate_card(&oh_card("task-a", OhStatus::Todo))];
        crate_todos::replace(&store, "user-tasks", cards)
            .await
            .expect("seed");

        let expected = [CrateStatus::Todo, CrateStatus::Ready];
        let claimed = crate_todos::claim_card(
            &store,
            "user-tasks",
            "task-a",
            &expected,
            CrateStatus::InProgress,
        )
        .await
        .expect("first claim ok");
        assert_eq!(claimed.status, CrateStatus::InProgress);

        // Second claim expecting Todo now loses the CAS — matches the legacy
        // "claim rejected" path the dispatcher relies on.
        let rejected = crate_todos::claim_card(
            &store,
            "user-tasks",
            "task-a",
            &expected,
            CrateStatus::InProgress,
        )
        .await;
        assert!(rejected.is_err(), "stale claim must be rejected");
    }
}
