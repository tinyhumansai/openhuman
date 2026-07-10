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

/// Base cadence: how often the poller wakes to look for a dispatchable card
/// while there is fresh work to do.
const POLLER_TICK_SECONDS: u64 = 60;

/// Ceiling on the backed-off cadence (issue #4090). After a run of idle ticks
/// the interval doubles up to this cap — an effective self-suspend that still
/// rechecks periodically. 15 minutes: long enough to stop hammering an idle
/// board, short enough that newly-arrived work is still picked up without an
/// unbounded stall.
const POLLER_MAX_BACKOFF_SECONDS: u64 = 15 * 60;

/// Number of consecutive idle ticks tolerated at the base cadence before the
/// backoff starts to grow — a small grace so a briefly-empty board doesn't
/// immediately slow down.
const POLLER_IDLE_GRACE_TICKS: u32 = 2;

static POLLER_STARTED: OnceLock<()> = OnceLock::new();

/// Compute the next sleep before a poll tick given how many consecutive idle
/// ticks have elapsed (issue #4090). Pure + deterministic so the backoff curve
/// is unit-testable without the real timer.
///
/// - Fresh work (`idle_ticks == 0`) or within the grace window → base cadence.
/// - Beyond the grace window → exponential backoff (double per extra idle tick)
///   saturating at [`POLLER_MAX_BACKOFF_SECONDS`].
fn next_poll_delay(idle_ticks: u32) -> Duration {
    let over = idle_ticks.saturating_sub(POLLER_IDLE_GRACE_TICKS);
    if over == 0 {
        return Duration::from_secs(POLLER_TICK_SECONDS);
    }
    // Double per idle tick past the grace window, saturating at the cap. Clamp
    // the shift so a long idle streak can't overflow the multiply.
    let factor = 1u64.checked_shl(over.min(20)).unwrap_or(u64::MAX);
    let secs = POLLER_TICK_SECONDS
        .saturating_mul(factor)
        .min(POLLER_MAX_BACKOFF_SECONDS);
    Duration::from_secs(secs)
}

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
            max_backoff_seconds = POLLER_MAX_BACKOFF_SECONDS,
            "[task_dispatcher:poller] starting"
        );
        // Diminishing-returns backoff (issue #4090): a run of idle ticks (no
        // card dispatched) stretches the interval up to POLLER_MAX_BACKOFF so an
        // idle board isn't swept every 60s forever; a dispatch resets it to the
        // base cadence. The capped backoff still rechecks within
        // POLLER_MAX_BACKOFF, so newly-arrived work is picked up without an
        // unbounded stall (an event-driven instant wake is a follow-up).
        let mut idle_ticks: u32 = 0;
        loop {
            tokio::time::sleep(next_poll_delay(idle_ticks)).await;
            match poll_once().await {
                Ok(true) => {
                    // Dispatched work → reset to fast cadence.
                    idle_ticks = 0;
                }
                Ok(false) => {
                    idle_ticks = idle_ticks.saturating_add(1);
                    if idle_ticks == POLLER_IDLE_GRACE_TICKS + 1 {
                        tracing::debug!(
                            "[task_dispatcher:poller] board idle — backing off toward the {POLLER_MAX_BACKOFF_SECONDS}s ceiling"
                        );
                    }
                }
                Err(e) => {
                    tracing::warn!(error = %e, "[task_dispatcher:poller] tick failed (continuing)");
                    // Treat an errored tick as idle for backoff purposes so a
                    // persistently failing sweep doesn't spin at base cadence.
                    idle_ticks = idle_ticks.saturating_add(1);
                }
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
pub(crate) async fn poll_once() -> Result<bool, String> {
    // Gate on background-AI capacity (autonomy / power / pause). Dropping the
    // permit immediately is fine: this is a "may background work start now"
    // check; the run itself is detached. No capacity → an idle tick (returns
    // `false` so the caller backs off, #4090).
    let Some(_permit) = crate::openhuman::scheduler_gate::wait_for_capacity().await else {
        tracing::debug!("[task_dispatcher:poller] scheduler gate denied capacity; idle tick");
        return Ok(false);
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

    let mut dispatched_any = false;
    for (location, agent_assigned_only) in boards {
        match poll_board(&location, agent_assigned_only).await {
            Ok(dispatched) => dispatched_any |= dispatched,
            Err(e) => tracing::warn!(
                thread_id = ?location.thread_id(),
                error = %e,
                "[task_dispatcher:poller] board sweep failed (continuing)"
            ),
        }
    }
    Ok(dispatched_any)
}

/// Sweep one board: reclaim stale runs, then (unless one is already running)
/// dispatch its highest-urgency dispatchable card. When `agent_assigned_only`
/// is set, only cards with an `assigned_agent` are eligible — the guard that
/// keeps the poller off a human's manual `user-tasks` cards.
/// Returns `true` when this sweep dispatched a card (real work), `false` when
/// the board was idle (nothing to claim). The idle signal drives the poller's
/// diminishing-returns backoff (issue #4090).
async fn poll_board(location: &BoardLocation, agent_assigned_only: bool) -> Result<bool, String> {
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
        return Ok(false);
    }

    let Some(card) = pick_next_todo(&snapshot.cards, agent_assigned_only) else {
        return Ok(false);
    };

    tracing::info!(
        card_id = %card.id,
        thread_id = ?location.thread_id(),
        urgency = card_urgency(&card),
        agent_assigned_only,
        "[task_dispatcher:poller] dispatching highest-urgency dispatchable card"
    );
    dispatch_card(location.clone(), card).await.map(|_| true)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn next_poll_delay_holds_base_cadence_within_the_grace_window() {
        let base = Duration::from_secs(POLLER_TICK_SECONDS);
        // Fresh work and the first few idle ticks all poll at the base cadence.
        assert_eq!(next_poll_delay(0), base);
        assert_eq!(next_poll_delay(POLLER_IDLE_GRACE_TICKS), base);
    }

    #[test]
    fn next_poll_delay_backs_off_exponentially_past_the_grace_window() {
        // One tick past the grace window doubles, then doubles again.
        assert_eq!(
            next_poll_delay(POLLER_IDLE_GRACE_TICKS + 1),
            Duration::from_secs(POLLER_TICK_SECONDS * 2)
        );
        assert_eq!(
            next_poll_delay(POLLER_IDLE_GRACE_TICKS + 2),
            Duration::from_secs(POLLER_TICK_SECONDS * 4)
        );
        assert_eq!(
            next_poll_delay(POLLER_IDLE_GRACE_TICKS + 3),
            Duration::from_secs(POLLER_TICK_SECONDS * 8)
        );
    }

    #[test]
    fn next_poll_delay_saturates_at_the_ceiling() {
        let cap = Duration::from_secs(POLLER_MAX_BACKOFF_SECONDS);
        // A long idle streak caps out (self-suspend) and never overflows.
        assert_eq!(next_poll_delay(50), cap);
        assert_eq!(next_poll_delay(u32::MAX), cap);
        // The backoff is monotonic non-decreasing and never exceeds the ceiling.
        let mut prev = next_poll_delay(0);
        for idle in 1..40u32 {
            let d = next_poll_delay(idle);
            assert!(d >= prev, "backoff must not shrink as idle grows");
            assert!(d <= cap, "backoff must never exceed the ceiling");
            prev = d;
        }
    }
}
