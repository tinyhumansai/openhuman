//! Board poller: periodic sweep that dispatches dispatchable cards.
//!
//! Each tick scans the `task-sources` board and the `user-tasks` board,
//! reclaims stale runs, and dispatches the highest-urgency dispatchable card
//! via [`dispatch_card`], gated by background-AI capacity (`scheduler_gate`).

use std::sync::OnceLock;
use std::time::Duration;

use crate::openhuman::agent::task_board::{TaskApprovalMode, TaskBoardCard, TaskCardStatus};
use crate::openhuman::config::Config;
use crate::openhuman::todos::ops::{self, BoardLocation, USER_TASKS_THREAD_ID};
use crate::openhuman::todos::runs::{self, RunLimits};

use super::dispatch::dispatch_card;

/// How often the poller wakes to look for a dispatchable card.
const POLLER_TICK_SECONDS: u64 = 60;

static POLLER_STARTED: OnceLock<()> = OnceLock::new();

/// Spawn the board poller. Idempotent — only the first call installs the loop.
///
/// Each tick it scans the `task-sources` board and dispatches the
/// highest-urgency `todo` card via [`dispatch_card`], gated by background-AI
/// capacity (`scheduler_gate`). This is the catch-all for cards that arrive
/// without a proactive trigger (`TodoOnly` sources, manual cards, or proactive
/// turns the gate skipped). Cards that *did* get a proactive trigger are
/// dispatched by the triage arm; the claim-based lock makes firing both safe.
pub fn start_board_poller() {
    if POLLER_STARTED.set(()).is_err() {
        tracing::debug!("[task_dispatcher:poller] already running, skipping start");
        return;
    }
    tokio::spawn(async move {
        tracing::info!(
            tick_seconds = POLLER_TICK_SECONDS,
            "[task_dispatcher:poller] starting"
        );
        let mut ticker = tokio::time::interval(Duration::from_secs(POLLER_TICK_SECONDS));
        ticker.tick().await; // skip the immediate fire so startup isn't slammed
        loop {
            ticker.tick().await;
            if let Err(e) = poll_once().await {
                tracing::warn!(error = %e, "[task_dispatcher:poller] tick failed (continuing)");
            }
        }
    });
}

/// One poller tick: sweep each executor board and dispatch its highest-urgency
/// dispatchable card, if any and if capacity allows. `pub(crate)` so tests can
/// drive a tick without the real interval.
///
/// Two boards are swept, each independently (own stale-reclaim + single
/// `in_progress` cap):
/// - **`user-tasks`** (the kanban work board) — always swept, but only
///   **agent-assigned** cards are run, so a human's manually-created todo is
///   never auto-executed. This is where tasks approved out of the inbox run.
/// - **`task-sources`** (the proactive inbox) — swept only when ingestion is
///   enabled. With plan-approval required this only ever parks a `todo` at
///   `awaiting_approval`; it runs a card directly only when approval is off.
///   Kept in the sweep so its stale/wedged runs are still reclaimed.
pub(crate) async fn poll_once() -> Result<(), String> {
    // Gate on background-AI capacity (autonomy / power / pause). Dropping the
    // permit immediately is fine: this is a "may background work start now"
    // check; the run itself is detached.
    let Some(_permit) = crate::openhuman::scheduler_gate::wait_for_capacity().await else {
        tracing::debug!("[task_dispatcher:poller] scheduler gate denied capacity; idle tick");
        return Ok(());
    };

    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e:#}"))?;

    // (board location, agent_assigned_only). user-tasks first — it's the real
    // work board; task-sources is only included for parking + reclaim.
    let mut boards: Vec<(BoardLocation, bool)> = vec![(
        BoardLocation::Thread {
            workspace_dir: config.workspace_dir.clone(),
            thread_id: USER_TASKS_THREAD_ID.to_string(),
        },
        true,
    )];
    if config.task_sources.enabled {
        boards.push((
            BoardLocation::Thread {
                workspace_dir: config.workspace_dir.clone(),
                thread_id: crate::openhuman::task_sources::TASK_SOURCES_THREAD_ID.to_string(),
            },
            false,
        ));
    }

    for (location, agent_assigned_only) in boards {
        if let Err(e) = poll_board(&location, agent_assigned_only).await {
            tracing::warn!(
                thread_id = ?location.thread_id(),
                error = %e,
                "[task_dispatcher:poller] board sweep failed (continuing)"
            );
        }
    }
    Ok(())
}

/// Sweep one board: reclaim stale runs, then (unless one is already running)
/// dispatch its highest-urgency dispatchable card. When `agent_assigned_only`
/// is set, only cards with an `assigned_agent` are eligible — the guard that
/// keeps the poller off a human's manual `user-tasks` cards.
async fn poll_board(location: &BoardLocation, agent_assigned_only: bool) -> Result<(), String> {
    // Reclaim stale/wedged runs before looking for new work. Reclaimed
    // cards move back to `todo` (re-dispatchable) so they appear in the
    // snapshot below and can be picked up in the same tick.
    match runs::reclaim_stale(location, &RunLimits::default()) {
        Ok(result) if result.reclaimed_count > 0 || result.blocked_count > 0 => {
            tracing::info!(
                thread_id = ?location.thread_id(),
                reclaimed = result.reclaimed_count,
                blocked = result.blocked_count,
                "[task_dispatcher:poller] stale runs reclaimed"
            );
        }
        Err(e) => {
            tracing::warn!(
                thread_id = ?location.thread_id(),
                error = %e,
                "[task_dispatcher:poller] stale reclaim failed (continuing)"
            );
        }
        _ => {}
    }

    let snapshot = ops::list(location)?;

    // `enforce_single_in_progress` caps the board at one running card, so if
    // one is already in progress there's nothing for this tick to claim.
    if snapshot
        .cards
        .iter()
        .any(|c| c.status == TaskCardStatus::InProgress)
    {
        return Ok(());
    }

    let Some(card) = pick_next_todo(&snapshot.cards, agent_assigned_only) else {
        return Ok(());
    };

    tracing::info!(
        card_id = %card.id,
        thread_id = ?location.thread_id(),
        urgency = card_urgency(&card),
        agent_assigned_only,
        "[task_dispatcher:poller] dispatching highest-urgency dispatchable card"
    );
    dispatch_card(location.clone(), card).await.map(|_| ())
}

/// Highest-urgency dispatchable card (`todo` or approved `ready`; urgency from
/// `source_metadata.urgency`, default 0.0; ties broken toward the lower board
/// `order`). Returns a clone. `dispatch_card` then either runs a `ready` card
/// or parks a `todo` one for approval, per the autonomy setting.
///
/// When `agent_assigned_only` is set, cards without an `assigned_agent` are
/// excluded — used on the `user-tasks` board so the poller runs only
/// agent-generated tasks and never picks up a human's manually-created card.
pub(super) fn pick_next_todo(
    cards: &[TaskBoardCard],
    agent_assigned_only: bool,
) -> Option<TaskBoardCard> {
    cards
        .iter()
        .filter(|c| matches!(c.status, TaskCardStatus::Todo | TaskCardStatus::Ready))
        .filter(|c| {
            !agent_assigned_only
                || c.assigned_agent
                    .as_deref()
                    .map(|a| !a.trim().is_empty())
                    .unwrap_or(false)
        })
        .max_by(|a, b| {
            card_urgency(a)
                .partial_cmp(&card_urgency(b))
                .unwrap_or(std::cmp::Ordering::Equal)
                // On equal urgency, prefer the lower `order` (earlier card):
                // reversing the order comparison makes it the "greater" pick.
                .then(b.order.cmp(&a.order))
        })
        .cloned()
}

/// Whether a card must be parked at `awaiting_approval` before it can run.
///
/// Per-card `approval_mode` is authoritative when set; the global
/// `require_task_plan_approval` setting is only the fallback for cards with no
/// explicit preference:
/// - `Required` → always park, **even when the global default is off**. The
///   interactive plan-review gate (WebChat turns, see
///   [`crate::openhuman::agent::tools::todo`]) stamps `Required`, and that
///   review must hold regardless of the global switch — otherwise an
///   interactive plan would execute before the user ever sees the review card.
/// - `NotRequired` → never park (already cleared human review, e.g. approved
///   out of the `task-sources` inbox onto `user-tasks`).
/// - unset → fall back to the global default.
pub(super) fn requires_plan_approval(
    global_required: bool,
    approval_mode: Option<&TaskApprovalMode>,
) -> bool {
    match approval_mode {
        Some(TaskApprovalMode::Required) => true,
        Some(TaskApprovalMode::NotRequired) => false,
        None => global_required,
    }
}

pub(super) fn card_urgency(card: &TaskBoardCard) -> f64 {
    card.source_metadata
        .as_ref()
        .and_then(|m| m.get("urgency"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}
