//! Deterministic task-card dispatcher.
//!
//! Turns a [`TaskBoardCard`] into work: it **claims** the card via a
//! compare-and-set (re-load the board and transition only a `Todo`/`Ready`
//! card to `in_progress`, so a stale/concurrent re-dispatch of the same card
//! is rejected), runs a single **autonomous agent turn** toward the card's
//! objective, and **writes the outcome back** to the board (`done` + evidence
//! on success, `blocked` + reason on failure).
//!
//! This is the one executor both dispatch paths converge on:
//! - the **board poller** (cards that arrived without a proactive trigger), and
//! - the **proactive triage** arm (`agent::triage::apply_decision`), once it has
//!   decided to act on a task-board card.
//!
//! The runner mirrors `skills::spawn_workflow_run_background`: build the
//! `orchestrator` agent fresh inside a detached task, cap tool iterations, and
//! run `agent.run_single` under `with_autonomous_iter_cap`. PR-4 generalises the
//! executor from the default agent to a resolved personality/skill; this module
//! keeps the default-agent path so the pipeline runs end-to-end first.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::time::Duration;

use crate::openhuman::agent::harness::definition::{AgentDefinitionRegistry, PromptSource};
use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::harness::subagent_runner::with_autonomous_iter_cap;
use crate::openhuman::agent::personality_paths::PersonalityContext;
use crate::openhuman::agent::task_board::{TaskApprovalMode, TaskBoardCard, TaskCardStatus};
use crate::openhuman::agent::task_session;
use crate::openhuman::config::Config;
use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch, USER_TASKS_THREAD_ID};
use crate::openhuman::todos::runs::{self, RunLimits, RunOutcome};

/// Max chars of a personality SOUL.md / MEMORY.md or skill guideline block
/// folded into the agent's system-prompt suffix.
const EXECUTOR_PREAMBLE_MAX_CHARS: usize = 800;

/// Tool-iteration ceiling for an autonomous task run. Matches the skill-run
/// cap — a task brief is the same shape of bounded autonomous work.
const TASK_RUN_MAX_ITERATIONS: usize = 200;

/// Max chars of the agent's final output retained as board `evidence`.
const EVIDENCE_MAX_CHARS: usize = 2_000;

/// Handle to an in-flight autonomous run, keyed by its session `thread_id`.
///
/// Autonomous runs are detached `tokio` tasks, not web-channel turns, so they
/// are invisible to the web channel's own in-flight registry — which is why the
/// chat **Cancel** button (which calls `channel_web_cancel`) couldn't stop them.
/// Registering the run's [`AbortHandle`](tokio::task::AbortHandle) here lets
/// [`cancel_session`] abort it from that same cancel path.
struct ActiveRun {
    abort: tokio::task::AbortHandle,
    hb_cancel: tokio::sync::watch::Sender<bool>,
    location: BoardLocation,
    card_id: String,
    run_id: String,
}

static ACTIVE_RUNS: OnceLock<Mutex<HashMap<String, ActiveRun>>> = OnceLock::new();

fn active_runs() -> &'static Mutex<HashMap<String, ActiveRun>> {
    ACTIVE_RUNS.get_or_init(|| Mutex::new(HashMap::new()))
}

fn register_active_run(thread_id: String, run: ActiveRun) {
    active_runs()
        .lock()
        .expect("active_runs mutex poisoned")
        .insert(thread_id, run);
}

/// Remove and return the active-run entry for `thread_id`. The naturally
/// completing run and a concurrent [`cancel_session`] race on this — whoever
/// gets `Some` "owns" the terminal board write-back, so it happens exactly once.
fn take_active_run(thread_id: &str) -> Option<ActiveRun> {
    active_runs()
        .lock()
        .expect("active_runs mutex poisoned")
        .remove(thread_id)
}

/// Cancel the in-flight autonomous run streaming into session `thread_id`.
///
/// Aborts the detached run task, stops its heartbeat, marks the card `blocked`
/// (user-cancelled) so it doesn't dangle `in_progress`, and emits the terminal
/// chat event (broadcast as `"system"`) so the session UI stops "processing".
/// Returns `true` if a run was found and cancelled. Wired into the web channel's
/// `channel_web_cancel` as the fallback when the thread has no web-channel turn.
pub async fn cancel_session(thread_id: &str) -> bool {
    let Some(run) = take_active_run(thread_id) else {
        return false;
    };
    run.abort.abort();
    let _ = run.hb_cancel.send(true);
    // The aborted task never reaches its own write-back — do it here so the
    // card lands in a terminal state instead of a stale `in_progress`.
    write_back(
        &run.location,
        &run.card_id,
        &run.run_id,
        Err("Cancelled by user".to_string()),
    );
    crate::openhuman::channels::providers::web::publish_web_channel_event(
        crate::core::socketio::WebChannelEvent {
            event: "chat_error".to_string(),
            client_id: "system".to_string(),
            thread_id: thread_id.to_string(),
            request_id: run.run_id.clone(),
            message: Some("Cancelled".to_string()),
            error_type: Some("cancelled".to_string()),
            ..Default::default()
        },
    );
    tracing::info!(
        thread_id = %thread_id,
        card_id = %run.card_id,
        run_id = %run.run_id,
        "[task_dispatcher] cancelled autonomous run via chat cancel"
    );
    true
}

/// Render a card into the goal prompt handed to the autonomous run.
///
/// The card's `content`/title is the display form; the prompt leads with the
/// clean `objective`, then any `plan` steps and `acceptance_criteria`, and a
/// pointer to the originating source so the agent can pull related context from
/// memory via its `memory_recall` tool (the GitHub/Notion/… activity for this
/// item is ingested into the summary tree by the memory-sources domain).
pub fn build_task_prompt(card: &TaskBoardCard) -> String {
    let mut lines: Vec<String> = Vec::new();

    let objective = card
        .objective
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| card.title.trim());
    lines.push(format!(
        "You are autonomously executing one task to completion. Objective:\n{objective}"
    ));

    if !card.plan.is_empty() {
        lines.push("\nPlan:".to_string());
        for (i, step) in card.plan.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, step.trim()));
        }
    }

    if !card.acceptance_criteria.is_empty() {
        lines.push("\nAcceptance criteria (the task is done only when all hold):".to_string());
        for c in &card.acceptance_criteria {
            lines.push(format!("- {}", c.trim()));
        }
    }

    if let Some(meta) = &card.source_metadata {
        let provider = meta.get("provider").and_then(|v| v.as_str());
        let repo = meta.get("repo").and_then(|v| v.as_str());
        let external_id = meta.get("external_id").and_then(|v| v.as_str());
        let url = meta.get("url").and_then(|v| v.as_str());
        let mut origin = String::new();
        if let Some(p) = provider {
            origin.push_str(p);
        }
        if let Some(r) = repo {
            origin.push_str(&format!(" {r}"));
        }
        if let Some(id) = external_id {
            origin.push_str(&format!("#{id}"));
        }
        // Gate on a known provider so the origin string is always meaningful
        // (an id-only card would render "#123" with a leading space).
        if provider.is_some() {
            lines.push(format!(
                "\nThis task originates from {}. Its activity has been ingested into memory — use \
                 your memory_recall tool to pull related context (prior discussion, linked items) \
                 before and while you work.",
                origin.trim()
            ));
        }
        if let Some(u) = url {
            lines.push(format!("Source link: {u}"));
        }
        // G9b — agent-driven external write-back. When the upstream item is
        // addressable (provider + id), instruct the agent to close the loop on
        // the source itself via its integration tools. Runs under the
        // connection's existing write scope (no extra approval gate); if it
        // can't, it reports that instead of failing.
        if provider.is_some() && external_id.is_some() {
            lines.push(format!(
                "\nWhen the task is complete, record the outcome on the upstream source ({}): use \
                 your integration tools to add a comment summarising the resolution and, if the \
                 work fully addresses it, close/resolve the item. If you lack the permission or \
                 connection to do so, say so in your final summary instead of guessing.",
                origin.trim()
            ));
        }
    }

    lines.push(
        "\nWork the task to completion. Do not pick up unrelated work. When finished, your final \
         message should summarise what you did and the evidence (commits, PRs, results)."
            .to_string(),
    );

    lines.join("\n")
}

/// Instruction appended to the run prompt so the autonomous turn keeps its own
/// task card current via the `update_task` tool while it works.
///
/// The card is already `in_progress` (the dispatcher claimed it before
/// spawning the run), addressed by the exact card id + board the run owns
/// (without the explicit `threadId` the tool defaults to the `task-sources`
/// board and would miss a `user-tasks` card). Two things this asks for:
/// 1. *progress* updates (notes/evidence) as the run works, and
/// 2. an explicit `status: blocked` + `blocker` when the run needs a
///    decision/information from the user or cannot proceed — which
///    [`write_back`] now preserves rather than force-completing, so the task
///    pauses for the user instead of being silently marked done.
fn build_progress_instruction(card_id: &str, thread_id: &str) -> String {
    format!(
        "\n\nThis task is tracked as card `{card_id}` on the `{thread_id}` board. As you work, \
         call the `update_task` tool (id `{card_id}`, threadId `{thread_id}`) to keep the card \
         current — append `notes`/`evidence` as you make progress.\n\nIf you need a decision or \
         information from the user, or you genuinely cannot proceed (missing access, ambiguous \
         requirement, an action that needs the user's confirmation), call `update_task` with \
         `status: blocked` and a `blocker` that states exactly what you need from the user. The \
         task will stay paused in that blocked state until the user responds — do NOT guess, \
         fabricate, or take a risky irreversible action just to avoid blocking. If instead you \
         finish the work, end with a summary of what you did and the evidence; completion is \
         recorded automatically."
    )
}

/// Outcome of a dispatch attempt.
#[derive(Debug)]
pub enum DispatchOutcome {
    /// The card was claimed and a detached autonomous run was spawned.
    Running { run_id: String },
    /// Plan approval is required; the card was parked at `awaiting_approval`
    /// and a `TaskPlanAwaitingApproval` event was emitted. No run was spawned.
    AwaitingApproval,
}

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
            write_back(&location_for_run, &card_id, &run_id, outcome);
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

/// A resolved executor: which built-in agent definition to build, an optional
/// system-prompt suffix carrying a personality identity or skill guidelines,
/// and a label for logs/telemetry.
#[derive(Debug, Clone, PartialEq)]
struct ResolvedExecutor {
    agent_id: String,
    prompt_suffix: Option<String>,
    label: String,
}

impl ResolvedExecutor {
    fn default_agent() -> Self {
        Self {
            agent_id: "orchestrator".to_string(),
            prompt_suffix: None,
            label: "default".to_string(),
        }
    }
}

/// Map a card's `assigned_agent` handle to one of three executor presets:
/// **personality** (scoped SOUL/MEMORY folded into the prompt suffix, run as
/// that profile's agent), **skill** (orchestrator seeded with the skill's
/// `SKILL.md` guidelines), or **built-in agent**. An unset or unresolved handle
/// degrades to the default `orchestrator` — "use the personality if valid,
/// otherwise the default agent."
fn resolve_executor(workspace_dir: &Path, assigned: Option<&str>) -> ResolvedExecutor {
    let Some(handle) = assigned.map(str::trim).filter(|s| !s.is_empty()) else {
        return ResolvedExecutor::default_agent();
    };
    if handle == "orchestrator" {
        return ResolvedExecutor::default_agent();
    }

    // 1) Personality (#2895): a user-defined profile with scoped identity.
    if let Ok(state) = crate::openhuman::agent::profiles::load_profiles(workspace_dir) {
        if let Some(profile) = state.profiles.iter().find(|p| p.id == handle) {
            let ctx = PersonalityContext::from_profile(workspace_dir, profile.clone());
            let mut preamble = format!(
                "You are acting as the personality `{}` (\"{}\"). {}",
                profile.id, profile.name, profile.description
            );
            if let Some(soul) = &ctx.soul_md_override {
                preamble.push_str("\n\n[Personality SOUL.md]\n");
                preamble.push_str(&truncate_chars(soul, EXECUTOR_PREAMBLE_MAX_CHARS));
            }
            if let Some(mem) = &ctx.memory_md_override {
                preamble.push_str("\n\n[Personality MEMORY.md]\n");
                preamble.push_str(&truncate_chars(mem, EXECUTOR_PREAMBLE_MAX_CHARS));
            }
            return ResolvedExecutor {
                agent_id: profile.agent_id.clone(),
                prompt_suffix: Some(preamble),
                label: format!("personality:{handle}"),
            };
        }
    }

    // 2) Workflow (#2824): the same autonomous run, seeded with SKILL.md.
    if let Some(skill) = crate::openhuman::workflows::registry::get_workflow(workspace_dir, handle)
    {
        let guidelines = match &skill.definition.system_prompt {
            PromptSource::Inline(s) => truncate_chars(s, EXECUTOR_PREAMBLE_MAX_CHARS),
            _ => String::new(),
        };
        let suffix = format!(
            "You are executing this task as the skill `{handle}`. Follow these skill \
             guidelines exactly:\n\n{guidelines}"
        );
        return ResolvedExecutor {
            agent_id: "orchestrator".to_string(),
            prompt_suffix: Some(suffix),
            label: format!("skill:{handle}"),
        };
    }

    // 3) Built-in agent definition.
    if AgentDefinitionRegistry::global()
        .and_then(|r| r.get(handle))
        .is_some()
    {
        return ResolvedExecutor {
            agent_id: handle.to_string(),
            prompt_suffix: None,
            label: format!("agent:{handle}"),
        };
    }

    // 4) Unresolved → degrade to the default agent (don't fail the card).
    tracing::warn!(
        handle = %handle,
        "[task_dispatcher] assigned executor did not resolve to a personality/skill/agent; \
         using default orchestrator"
    );
    ResolvedExecutor {
        label: "default-fallback".to_string(),
        ..ResolvedExecutor::default_agent()
    }
}

/// Run the resolved executor as a single autonomous turn using the
/// already-loaded config. The executor's prompt suffix (personality identity or
/// skill guidelines) rides in the system prompt; the card goal is the turn input.
///
/// SECURITY / threat model (prompt injection): the card objective/content and
/// `source_metadata` derive from external, attacker-influenceable text (e.g. a
/// GitHub issue body anyone in a watched repo can file), and this background
/// run is gate-free at the per-tool level (background turns auto-allow, like
/// skill runs) while `build_task_prompt` may instruct it to write back to the
/// upstream item. The interactive checkpoint is therefore the up-front
/// **plan-approval gate** (`require_task_plan_approval`), which a human reviews
/// before the run starts — not per-action egress/write approval. Egress is
/// widened to `*` only when the operator set no explicit allow-list (matching
/// skill runs, since real task work needs broad reach: git, package registries,
/// provider APIs). Tightening egress to the source provider's domains for
/// source-ingested runs is a considered follow-up (it would break general task
/// work, so it needs to key off provenance) — tracked for a later PR.
async fn run_autonomous(
    mut config: Config,
    executor: &ResolvedExecutor,
    prompt: &str,
    run_id: &str,
    session_thread_id: Option<String>,
) -> Result<String, String> {
    config.agent.max_tool_iterations = TASK_RUN_MAX_ITERATIONS;
    // Match skill-run egress handling: only widen to the permissive default
    // when the operator hasn't configured an explicit allow-list. See the
    // threat-model note above on why `*` is the default here.
    if config.http_request.allowed_domains.is_empty() {
        config.http_request.allowed_domains = vec!["*".to_string()];
    }

    let mut agent = Agent::from_config_for_agent_with_profile(
        &config,
        &executor.agent_id,
        None,
        executor.prompt_suffix.clone(),
    )
    .map_err(|e| format!("build agent: {e:#}"))?;
    agent.set_event_context(run_id.to_string(), "task");
    agent.set_agent_definition_name(format!(
        "task-{}-{}",
        executor.label,
        run_id.get(..8).unwrap_or(run_id)
    ));

    // Stream this autonomous run into its task-session thread exactly like a
    // chat turn: wire the agent's progress into the web-channel bridge with the
    // broadcast client id "system" — the same mechanism cron/welcome agents use.
    // The bridge (a) emits live text/tool socket events that any client viewing
    // the thread renders in real time (the frontend keys by thread_id), and
    // (b) persists a TurnStateMirror so the tool timeline replays when the
    // session is opened mid/after run. Best-effort — with no session thread the
    // run is headless, exactly as before this feature.
    let workspace_dir = config.workspace_dir.clone();
    if let Some(thread_id) = session_thread_id.as_deref() {
        let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
        agent.set_on_progress(Some(progress_tx));
        crate::openhuman::channels::providers::web::spawn_progress_bridge(
            progress_rx,
            "system".to_string(),
            thread_id.to_string(),
            run_id.to_string(),
            crate::openhuman::threads::turn_state::TurnStateStore::new(workspace_dir.clone()),
            crate::openhuman::channels::providers::web::ChatRequestMetadata::default(),
            config.clone(),
        );
    }

    // Sub-agent task runs are internal to the agent harness — the user
    // already authorized the parent turn that dispatched this task. Label
    // as CLI so the approval gate doesn't fail closed on internal
    // sub-agent invocations.
    let run = crate::openhuman::agent::turn_origin::with_origin(
        crate::openhuman::agent::turn_origin::AgentTurnOrigin::Cli,
        with_autonomous_iter_cap(TASK_RUN_MAX_ITERATIONS, agent.run_single(prompt)),
    );
    let result = match session_thread_id.as_deref() {
        Some(thread_id) => {
            crate::openhuman::inference::provider::thread_context::with_thread_id(
                thread_id.to_string(),
                run,
            )
            .await
        }
        None => run.await,
    }
    .map_err(|e| format!("{e:#}"));

    // Emit the terminal chat event so a client viewing the session stops
    // "processing" and finalizes the assistant bubble — the SAME chat_done /
    // chat_error the web channel emits at the end of a normal turn. The
    // progress bridge only streams intermediate deltas; without this terminal
    // signal the live-streamed session spins forever. Broadcast as "system" so
    // any viewer of the thread receives it (frontend keys by thread_id).
    if let Some(thread_id) = session_thread_id.as_deref() {
        match &result {
            Ok(response) => {
                crate::openhuman::channels::providers::presentation::deliver_response(
                    "system",
                    thread_id,
                    run_id,
                    response,
                    prompt,
                    &[],
                )
                .await;
            }
            Err(err) => {
                crate::openhuman::channels::providers::web::publish_web_channel_event(
                    crate::core::socketio::WebChannelEvent {
                        event: "chat_error".to_string(),
                        client_id: "system".to_string(),
                        thread_id: thread_id.to_string(),
                        request_id: run_id.to_string(),
                        message: Some(err.clone()),
                        error_type: Some("agent_error".to_string()),
                        ..Default::default()
                    },
                );
            }
        }
        // Persist the final response as the closing assistant message so a
        // reopened session shows the outcome like a finished manual run.
        task_session::append_final(workspace_dir, thread_id, &result);
    }
    result
}

/// Deterministic board write-back: the dispatcher owns the card lifecycle.
/// Success → `done` + evidence; failure → `blocked` + blocker reason. An
/// external write failure here is logged, never propagated — the run already
/// happened.
/// Current persisted status of a card, or `None` if the board can't be read or
/// the card is gone. Used by `write_back` to detect a run that blocked itself.
fn current_card_status(location: &BoardLocation, card_id: &str) -> Option<TaskCardStatus> {
    ops::list(location)
        .ok()
        .and_then(|snap| snap.cards.into_iter().find(|c| c.id == card_id))
        .map(|c| c.status)
}

fn write_back(
    location: &BoardLocation,
    card_id: &str,
    run_id: &str,
    outcome: Result<String, String>,
) {
    // Respect a status the run set for itself: if the agent marked the card
    // `blocked` via `update_task` (it needs a decision/input from the user, or
    // genuinely cannot proceed), leave it blocked — do NOT force-complete it.
    // The task then stays paused in that state until the user responds, instead
    // of a "clean turn" being silently recorded as done. Otherwise mark done
    // with evidence; a run error marks blocked with the error as the blocker.
    let agent_self_blocked =
        outcome.is_ok() && current_card_status(location, card_id) == Some(TaskCardStatus::Blocked);

    let patch = if agent_self_blocked {
        tracing::info!(
            card_id = %card_id,
            run_id = %run_id,
            "[task_dispatcher] run ended with card self-blocked → leaving blocked (awaiting user input), not auto-completing"
        );
        None
    } else {
        match &outcome {
            Ok(output) => {
                tracing::info!(
                    card_id = %card_id,
                    run_id = %run_id,
                    output_chars = output.chars().count(),
                    "[task_dispatcher] run complete → done"
                );
                Some(CardPatch {
                    status: Some(TaskCardStatus::Done),
                    evidence: Some(vec![truncate_chars(output.trim(), EVIDENCE_MAX_CHARS)]),
                    ..Default::default()
                })
            }
            Err(err) => {
                tracing::warn!(
                    card_id = %card_id,
                    run_id = %run_id,
                    error = %err,
                    "[task_dispatcher] run failed → blocked"
                );
                Some(CardPatch {
                    status: Some(TaskCardStatus::Blocked),
                    blocker: Some(truncate_chars(err, EVIDENCE_MAX_CHARS)),
                    ..Default::default()
                })
            }
        }
    };

    if let Some(patch) = patch {
        if let Err(e) = ops::edit(location, card_id, patch) {
            tracing::error!(
                card_id = %card_id,
                run_id = %run_id,
                error = %e,
                "[task_dispatcher] board write-back failed (run outcome lost from board)"
            );
        }
    }

    let (run_outcome, run_error, run_evidence) = match &outcome {
        Ok(output) => (
            RunOutcome::Success,
            None,
            vec![truncate_chars(output.trim(), EVIDENCE_MAX_CHARS)],
        ),
        Err(err) => (
            RunOutcome::Failed,
            Some(truncate_chars(err, EVIDENCE_MAX_CHARS)),
            Vec::new(),
        ),
    };
    if let Err(e) = runs::complete_run(location, run_id, run_outcome, run_error, run_evidence) {
        tracing::warn!(
            run_id = %run_id,
            error = %e,
            "[task_dispatcher] run record completion failed"
        );
    }
}

fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}

// ── Board poller ──────────────────────────────────────────────────────────

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
fn pick_next_todo(cards: &[TaskBoardCard], agent_assigned_only: bool) -> Option<TaskBoardCard> {
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
/// The global `require_task_plan_approval` setting applies *unless* the card is
/// explicitly marked `approval_mode = NotRequired` — a per-card opt-out for
/// tasks that have already cleared human review (e.g. approved out of the
/// `task-sources` inbox onto `user-tasks`). Per-card opt-out wins over the
/// global default; without this, an already-approved card would be re-parked
/// and stranded.
fn requires_plan_approval(global_required: bool, approval_mode: Option<&TaskApprovalMode>) -> bool {
    global_required && approval_mode != Some(&TaskApprovalMode::NotRequired)
}

fn card_urgency(card: &TaskBoardCard) -> f64 {
    card.source_metadata
        .as_ref()
        .and_then(|m| m.get("urgency"))
        .and_then(serde_json::Value::as_f64)
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn active_run_registry_take_is_once() {
        // Race-safety: the completing run and a concurrent cancel both call
        // `take_active_run`; exactly one gets `Some` (and owns the write-back).
        let (tx, _rx) = tokio::sync::watch::channel(false);
        let handle = tokio::spawn(async { std::future::pending::<()>().await });
        let key = "task-cancel-registry-test";
        register_active_run(
            key.to_string(),
            ActiveRun {
                abort: handle.abort_handle(),
                hb_cancel: tx,
                location: BoardLocation::Scratch,
                card_id: "c1".to_string(),
                run_id: "r1".to_string(),
            },
        );
        assert!(take_active_run(key).is_some(), "first take owns the run");
        assert!(
            take_active_run(key).is_none(),
            "second take gets nothing — write-back happens exactly once"
        );
        handle.abort();
    }

    fn card(objective: Option<&str>) -> TaskBoardCard {
        TaskBoardCard {
            id: "task-1".into(),
            title: "[GitHub] Fix login bug".into(),
            status: TaskCardStatus::Todo,
            objective: objective.map(str::to_string),
            plan: vec![],
            assigned_agent: None,
            allowed_tools: vec![],
            approval_mode: None,
            acceptance_criteria: vec![],
            evidence: vec![],
            notes: None,
            blocker: None,
            session_thread_id: None,
            source_metadata: None,
            order: 0,
            updated_at: String::new(),
        }
    }

    #[test]
    fn prompt_uses_objective_then_falls_back_to_title() {
        let p = build_task_prompt(&card(Some("Fix the login bug")));
        assert!(p.contains("Fix the login bug"));
        assert!(!p.contains("[GitHub]"));

        let p2 = build_task_prompt(&card(None));
        assert!(p2.contains("[GitHub] Fix login bug"));
    }

    #[test]
    fn prompt_includes_plan_and_acceptance_criteria() {
        let mut c = card(Some("Do it"));
        c.plan = vec!["step one".into(), "step two".into()];
        c.acceptance_criteria = vec!["tests pass".into()];
        let p = build_task_prompt(&c);
        assert!(p.contains("Plan:"));
        assert!(p.contains("1. step one"));
        assert!(p.contains("2. step two"));
        assert!(p.contains("Acceptance criteria"));
        assert!(p.contains("- tests pass"));
    }

    #[test]
    fn prompt_points_at_source_and_memory_when_metadata_present() {
        let mut c = card(Some("Resolve issue"));
        c.source_metadata = Some(json!({
            "provider": "github",
            "repo": "octo/repo",
            "external_id": "123",
            "url": "https://github.com/octo/repo/issues/123",
        }));
        let p = build_task_prompt(&c);
        assert!(p.contains("github octo/repo#123"));
        assert!(p.contains("memory_recall"));
        assert!(p.contains("https://github.com/octo/repo/issues/123"));
    }

    #[test]
    fn prompt_omits_source_block_without_metadata() {
        let p = build_task_prompt(&card(Some("Do it")));
        assert!(!p.contains("memory_recall"));
        assert!(!p.contains("record the outcome on the upstream source"));
    }

    #[test]
    fn prompt_includes_external_writeback_when_addressable() {
        let mut c = card(Some("Resolve issue"));
        c.source_metadata = Some(json!({
            "provider": "github",
            "repo": "octo/repo",
            "external_id": "123",
        }));
        let p = build_task_prompt(&c);
        assert!(p.contains("record the outcome on the upstream source"));
        assert!(p.contains("close/resolve the item"));
    }

    #[test]
    fn prompt_omits_writeback_when_not_addressable() {
        // Urgency-only metadata (no provider/external_id) can't address an
        // upstream item, so no write-back instruction.
        let mut c = card(Some("Do it"));
        c.source_metadata = Some(json!({ "urgency": 0.5 }));
        let p = build_task_prompt(&c);
        assert!(!p.contains("record the outcome on the upstream source"));
    }

    #[test]
    fn truncate_caps_long_strings() {
        let s = "x".repeat(5_000);
        let out = truncate_chars(&s, EVIDENCE_MAX_CHARS);
        assert!(out.chars().count() <= EVIDENCE_MAX_CHARS);
        assert!(out.ends_with('…'));
    }

    fn card_with(
        id: &str,
        status: TaskCardStatus,
        urgency: Option<f64>,
        order: u32,
    ) -> TaskBoardCard {
        let mut c = card(Some("obj"));
        c.id = id.into();
        c.status = status;
        c.order = order;
        c.source_metadata = urgency.map(|u| json!({ "urgency": u }));
        c
    }

    #[test]
    fn poller_picks_highest_urgency_todo_skipping_other_statuses() {
        let cards = vec![
            card_with("a", TaskCardStatus::Todo, Some(0.3), 0),
            card_with("b", TaskCardStatus::Done, Some(0.99), 1),
            card_with("c", TaskCardStatus::Todo, Some(0.8), 2),
            card_with("d", TaskCardStatus::Todo, None, 3),
        ];
        let picked = pick_next_todo(&cards, false).expect("a todo card is available");
        assert_eq!(
            picked.id, "c",
            "highest-urgency todo wins, done card ignored"
        );
    }

    #[test]
    fn poller_breaks_urgency_ties_toward_lower_order() {
        let cards = vec![
            card_with("late", TaskCardStatus::Todo, Some(0.5), 5),
            card_with("early", TaskCardStatus::Todo, Some(0.5), 2),
        ];
        assert_eq!(pick_next_todo(&cards, false).unwrap().id, "early");
    }

    #[test]
    fn poller_returns_none_when_no_todo_cards() {
        let cards = vec![card_with("a", TaskCardStatus::Done, Some(0.9), 0)];
        assert!(pick_next_todo(&cards, false).is_none());
    }

    #[test]
    fn poller_dispatches_ready_cards_and_skips_approval_states() {
        // Approved `ready` cards are dispatchable; `awaiting_approval` and
        // `rejected` are not.
        let cards = vec![
            card_with("await", TaskCardStatus::AwaitingApproval, Some(0.99), 0),
            card_with("rej", TaskCardStatus::Rejected, Some(0.95), 1),
            card_with("ready", TaskCardStatus::Ready, Some(0.5), 2),
        ];
        assert_eq!(pick_next_todo(&cards, false).unwrap().id, "ready");
    }

    #[test]
    fn poller_prefers_higher_urgency_across_todo_and_ready() {
        let cards = vec![
            card_with("ready-low", TaskCardStatus::Ready, Some(0.3), 0),
            card_with("todo-high", TaskCardStatus::Todo, Some(0.9), 1),
        ];
        assert_eq!(pick_next_todo(&cards, false).unwrap().id, "todo-high");
    }

    #[test]
    fn poller_agent_only_skips_unassigned_cards() {
        // On the user-tasks board we run only agent-assigned cards. A human's
        // manual todo (no assigned_agent) must be skipped even at high urgency.
        let mut human = card_with("human", TaskCardStatus::Todo, Some(0.99), 0);
        human.assigned_agent = None;
        let mut agent = card_with("agent", TaskCardStatus::Todo, Some(0.20), 1);
        agent.assigned_agent = Some("orchestrator".into());
        let cards = vec![human, agent];

        // Agent-only: the lower-urgency assigned card wins; the human card is invisible.
        assert_eq!(pick_next_todo(&cards, true).unwrap().id, "agent");
        // Unfiltered (task-sources behaviour): highest urgency wins regardless.
        assert_eq!(pick_next_todo(&cards, false).unwrap().id, "human");
    }

    #[test]
    fn poller_agent_only_returns_none_when_all_unassigned() {
        let mut a = card_with("a", TaskCardStatus::Todo, Some(0.9), 0);
        a.assigned_agent = None;
        let mut b = card_with("b", TaskCardStatus::Todo, Some(0.5), 1);
        b.assigned_agent = Some("   ".into()); // blank handle is not "assigned"
        let cards = vec![a, b];
        assert!(pick_next_todo(&cards, true).is_none());
    }

    #[test]
    fn approval_gate_respects_global_and_per_card_optout() {
        // Global off → never park.
        assert!(!requires_plan_approval(false, None));
        assert!(!requires_plan_approval(
            false,
            Some(&TaskApprovalMode::Required)
        ));
        // Global on → park, unless the card opts out via NotRequired.
        assert!(requires_plan_approval(true, None));
        assert!(requires_plan_approval(
            true,
            Some(&TaskApprovalMode::Required)
        ));
        assert!(!requires_plan_approval(
            true,
            Some(&TaskApprovalMode::NotRequired)
        ));
    }

    #[test]
    fn progress_instruction_names_card_thread_and_tool() {
        let s = build_progress_instruction("task-42", "user-tasks");
        assert!(s.contains("task-42"));
        assert!(s.contains("user-tasks"));
        assert!(s.contains("update_task"));
        // It must instruct the agent to self-block (status: blocked + blocker)
        // when it needs the user, so write_back can preserve that state.
        assert!(s.contains("status: blocked"));
        assert!(s.contains("blocker"));
    }

    #[test]
    fn resolver_defaults_to_orchestrator_for_unset_or_orchestrator_handle() {
        let dir = tempfile::tempdir().unwrap();
        for handle in [None, Some(""), Some("   "), Some("orchestrator")] {
            let r = resolve_executor(dir.path(), handle);
            assert_eq!(r.agent_id, "orchestrator");
            assert_eq!(r.label, "default");
            assert!(r.prompt_suffix.is_none());
        }
    }

    #[test]
    fn resolver_uses_personality_branch_for_builtin_profile() {
        // `load_profiles` returns built-in profiles for any empty workspace, so
        // the personality branch is reachable with no fixture file. "research"
        // is a built-in profile backed by the "researcher" agent.
        let dir = tempfile::tempdir().unwrap();
        let r = resolve_executor(dir.path(), Some("research"));
        assert_eq!(r.label, "personality:research");
        assert_eq!(r.agent_id, "researcher");
        let suffix = r.prompt_suffix.expect("personality preamble present");
        assert!(suffix.contains("acting as the personality `research`"));
    }

    #[test]
    fn resolver_degrades_to_default_for_unresolved_handle() {
        let dir = tempfile::tempdir().unwrap();
        let r = resolve_executor(dir.path(), Some("no-such-executor-xyz"));
        assert_eq!(r.agent_id, "orchestrator");
        assert_eq!(r.label, "default-fallback");
        assert!(r.prompt_suffix.is_none());
    }

    fn board_loc(dir: &std::path::Path) -> BoardLocation {
        BoardLocation::Thread {
            workspace_dir: dir.to_path_buf(),
            thread_id: "t1".to_string(),
        }
    }

    #[test]
    fn write_back_marks_done_with_evidence_on_success() {
        let dir = tempfile::tempdir().unwrap();
        let loc = board_loc(dir.path());
        let id = ops::add(&loc, "do the thing", CardPatch::default())
            .unwrap()
            .cards[0]
            .id
            .clone();
        ops::update_status(&loc, &id, TaskCardStatus::InProgress).unwrap();

        write_back(
            &loc,
            &id,
            "run-1",
            Ok("completed: opened PR #5".to_string()),
        );

        let card = ops::list(&loc)
            .unwrap()
            .cards
            .into_iter()
            .find(|c| c.id == id)
            .unwrap();
        assert_eq!(card.status, TaskCardStatus::Done);
        assert!(card.evidence.iter().any(|e| e.contains("opened PR #5")));
    }

    #[test]
    fn write_back_preserves_agent_set_blocked_on_clean_run() {
        // The run marked its own card `blocked` (needs user input) via
        // update_task, then ended cleanly. write_back must NOT force it to
        // `done` — the task stays blocked, with the agent's blocker intact,
        // awaiting the user.
        let dir = tempfile::tempdir().unwrap();
        let loc = board_loc(dir.path());
        let id = ops::add(&loc, "update alan", CardPatch::default())
            .unwrap()
            .cards[0]
            .id
            .clone();
        ops::update_status(&loc, &id, TaskCardStatus::InProgress).unwrap();
        // Agent self-blocks mid-run, as build_progress_instruction asks it to.
        ops::edit(
            &loc,
            &id,
            CardPatch {
                status: Some(TaskCardStatus::Blocked),
                blocker: Some("Slack isn't connected — confirm how to reach Alan".to_string()),
                ..Default::default()
            },
        )
        .unwrap();

        // Run returns Ok (the turn finished) — but the card is self-blocked.
        write_back(
            &loc,
            &id,
            "run-2",
            Ok("I checked GitHub and memory…".to_string()),
        );

        let card = ops::list(&loc)
            .unwrap()
            .cards
            .into_iter()
            .find(|c| c.id == id)
            .unwrap();
        assert_eq!(
            card.status,
            TaskCardStatus::Blocked,
            "a clean run over a self-blocked card must stay blocked, not auto-done"
        );
        assert_eq!(
            card.blocker.as_deref(),
            Some("Slack isn't connected — confirm how to reach Alan"),
            "the agent's blocker reason is preserved"
        );
    }

    #[test]
    fn write_back_marks_blocked_with_reason_on_failure() {
        let dir = tempfile::tempdir().unwrap();
        let loc = board_loc(dir.path());
        let id = ops::add(&loc, "do the thing", CardPatch::default())
            .unwrap()
            .cards[0]
            .id
            .clone();
        ops::update_status(&loc, &id, TaskCardStatus::InProgress).unwrap();

        write_back(&loc, &id, "run-1", Err("agent build failed".to_string()));

        let card = ops::list(&loc)
            .unwrap()
            .cards
            .into_iter()
            .find(|c| c.id == id)
            .unwrap();
        assert_eq!(card.status, TaskCardStatus::Blocked);
        assert!(card
            .blocker
            .as_deref()
            .unwrap_or_default()
            .contains("agent build failed"));
    }
}
