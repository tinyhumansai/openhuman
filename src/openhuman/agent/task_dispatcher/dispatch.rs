//! Card dispatch: claim a card and spawn an autonomous run.

use crate::openhuman::agent::task_board::{TaskBoardCard, TaskCardStatus};
use crate::openhuman::agent::task_session;
use crate::openhuman::config::Config;
use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch};
use crate::openhuman::todos::runs;

use super::executor::{resolve_executor, run_autonomous, TASK_RUN_MAX_ITERATIONS};
use super::poller::requires_plan_approval;
use super::prompt::{build_progress_instruction, build_task_prompt};
use super::registry::{register_active_run, take_active_run};
use super::types::{ActiveRun, DispatchOutcome};

/// Dispatch one card: gate on plan approval, claim it, run an autonomous turn,
/// write the result back.
///
/// Returns `Ok(Running)` once the card is claimed and the detached run is
/// spawned, `Ok(AwaitingApproval)` if the card was parked for human approval,
/// or `Err` *without* spawning when the card is no longer claimable — its
/// freshly-loaded status isn't `Todo`/`Ready` (already running/done, or another
/// dispatcher won the claim). Benign: the poller retries next tick.
pub async fn dispatch_card(
    location: BoardLocation,
    card: TaskBoardCard,
) -> Result<DispatchOutcome, String> {
    let card_id = card.id.clone();

    let config = Config::load_or_init()
        .await
        .map_err(|e| format!("load config: {e:#}"))?;

    // Plan-approval gate: when required, a `todo` card is parked for human
    // approval before it can run. `Ready` (already approved) bypasses. We
    // attempt the AwaitingApproval claim first so the gate is also atomic —
    // two dispatchers racing the same Todo card won't both park it.
    //
    // A card explicitly marked `approval_mode = NotRequired` also bypasses the
    // gate: it has already cleared human review (e.g. a task approved out of
    // the `task-sources` inbox onto the `user-tasks` board, stamped
    // `not_required` at approval time). Re-parking it under the global default
    // would strand it on a board nobody approves from. Per-card opt-out wins.
    if requires_plan_approval(
        config.autonomy.require_task_plan_approval,
        card.approval_mode.as_ref(),
    ) {
        match ops::claim_card(
            &location,
            &card_id,
            &[TaskCardStatus::Todo],
            TaskCardStatus::AwaitingApproval,
        ) {
            Ok(_parked) => {
                if let Some(thread_id) = location.thread_id() {
                    crate::core::event_bus::publish_global(
                        crate::core::event_bus::DomainEvent::TaskPlanAwaitingApproval {
                            card_id: card_id.clone(),
                            thread_id: thread_id.to_string(),
                        },
                    );
                }
                tracing::info!(card_id = %card_id, "[task_dispatcher] parked card awaiting plan approval");
                return Ok(DispatchOutcome::AwaitingApproval);
            }
            Err(_) => {
                // Card wasn't `Todo` — fall through to the main claim path,
                // which handles `Ready` cards and rejects everything else.
            }
        }
    }

    // Atomic claim: transition Todo|Ready → InProgress under a per-board
    // lock so concurrent dispatchers cannot both succeed. The returned card
    // is the freshly-loaded snapshot — the prompt uses it, not the caller's
    // potentially stale copy.
    let fresh_card = ops::claim_card(
        &location,
        &card_id,
        &[TaskCardStatus::Todo, TaskCardStatus::Ready],
        TaskCardStatus::InProgress,
    )
    .map_err(|e| format!("[task_dispatcher] claim rejected for {card_id}: {e}"))?;

    let mut prompt = build_task_prompt(&fresh_card);
    // Tell the run which card it owns so it can post live progress via the
    // `update_task` tool (notes/evidence) as it works. The terminal
    // `done`/`blocked` transition is still stamped deterministically by
    // `write_back` from the run outcome.
    if let Some(thread_id) = location.thread_id() {
        prompt.push_str(&build_progress_instruction(&card_id, thread_id));
    }

    let run_id = uuid::Uuid::new_v4().to_string();

    // Resolve which executor runs this card: default agent, a personality, or
    // a skill — one autonomous-run interface, three presets (G4 + G3).
    let executor = resolve_executor(&config.workspace_dir, fresh_card.assigned_agent.as_deref());
    tracing::info!(
        card_id = %card_id,
        run_id = %run_id,
        executor = %executor.label,
        agent_id = %executor.agent_id,
        prompt_chars = prompt.chars().count(),
        "[task_dispatcher] card claimed (→in_progress), spawning autonomous run"
    );

    if let Err(e) = runs::create_run(&location, &run_id, &card_id, &executor.label) {
        tracing::warn!(
            run_id = %run_id,
            card_id = %card_id,
            error = %e,
            "[task_dispatcher] failed to create run record (proceeding without liveness tracking)"
        );
    }

    let (hb_cancel_tx, hb_cancel_rx) = tokio::sync::watch::channel(false);
    runs::spawn_heartbeat_task(location.clone(), run_id.clone(), hb_cancel_rx);

    // Materialise this autonomous run as a top-level task-session thread so it
    // surfaces in Conversations → Tasks like a manually-run todo. Best-effort:
    // `None` just means the run streams nowhere (headless), exactly as before.
    let session_thread_id = task_session::create_session_thread(
        config.workspace_dir.clone(),
        &fresh_card,
        &run_id,
        &prompt,
    );

    // Stamp the session thread onto the card so the board UI can offer a
    // "View session" jump into Conversations. Best-effort: a failure here just
    // means the link is unavailable; the run proceeds regardless.
    if let Some(thread_id) = session_thread_id.as_deref() {
        if let Err(e) = ops::set_session_thread(&location, &card_id, Some(thread_id.to_string())) {
            tracing::warn!(
                card_id = %card_id,
                thread_id = %thread_id,
                error = %e,
                "[task_dispatcher] failed to stamp session thread on card (View session link unavailable)"
            );
        }
    }

    let run_id_for_return = run_id.clone();
    let location_for_run = location.clone();
    // Clones for the active-run registry (the originals move into the task).
    let reg_thread = session_thread_id.clone();
    let reg_location = location.clone();
    let reg_card_id = card_id.clone();
    let reg_run_id = run_id.clone();
    let hb_cancel_for_task = hb_cancel_tx.clone();
    let task_thread = session_thread_id.clone();
    // Gate the task on registration: a fast-finishing run could otherwise reach
    // its terminal `take_active_run` before `register_active_run` below has run,
    // see no entry, and skip `write_back` — leaving card/run state inconsistent.
    // The task parks on `start_rx` until we release it after registration.
    let (start_tx, start_rx) = tokio::sync::oneshot::channel::<()>();
    let join = tokio::spawn(async move {
        let _ = start_rx.await;
        let outcome = run_autonomous(config, &executor, &prompt, &run_id, session_thread_id).await;
        let _ = hb_cancel_for_task.send(true);
        // Race with a concurrent cancel: whoever removes the registry entry owns
        // the write-back, so it runs exactly once. No entry (no session thread,
        // or a cancel already took it) → we skip it.
        let still_ours = match &task_thread {
            Some(tid) => take_active_run(tid).is_some(),
            None => true,
        };
        if still_ours {
            super::executor::write_back(&location_for_run, &card_id, &run_id, outcome);
        }
    });

    // Register the run so the chat Cancel (web `channel_web_cancel` →
    // `cancel_session`) can abort it — task threads aren't web-channel turns.
    if let Some(tid) = reg_thread {
        register_active_run(
            tid,
            ActiveRun {
                abort: join.abort_handle(),
                hb_cancel: hb_cancel_tx,
                location: reg_location,
                card_id: reg_card_id,
                run_id: reg_run_id,
            },
        );
    }
    // Registration (if any) is in place — release the task to start running.
    let _ = start_tx.send(());

    Ok(DispatchOutcome::Running {
        run_id: run_id_for_return,
    })
}

#[allow(unused_imports)]
use CardPatch as _CardPatch;
