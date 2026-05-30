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
//! The runner mirrors `skills::spawn_skill_run_background`: build the
//! `orchestrator` agent fresh inside a detached task, cap tool iterations, and
//! run `agent.run_single` under `with_autonomous_iter_cap`. PR-4 generalises the
//! executor from the default agent to a resolved personality/skill; this module
//! keeps the default-agent path so the pipeline runs end-to-end first.

use std::path::Path;
use std::sync::OnceLock;
use std::time::Duration;

use crate::openhuman::agent::harness::definition::{AgentDefinitionRegistry, PromptSource};
use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::harness::subagent_runner::with_autonomous_iter_cap;
use crate::openhuman::agent::personality_paths::PersonalityContext;
use crate::openhuman::agent::task_board::{TaskBoardCard, TaskCardStatus};
use crate::openhuman::config::Config;
use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch};

/// Max chars of a personality SOUL.md / MEMORY.md or skill guideline block
/// folded into the agent's system-prompt suffix.
const EXECUTOR_PREAMBLE_MAX_CHARS: usize = 800;

/// Tool-iteration ceiling for an autonomous task run. Matches the skill-run
/// cap — a task brief is the same shape of bounded autonomous work.
const TASK_RUN_MAX_ITERATIONS: usize = 200;

/// Max chars of the agent's final output retained as board `evidence`.
const EVIDENCE_MAX_CHARS: usize = 2_000;

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

    // Claim CAS: re-load the card's CURRENT status before acting. The passed-in
    // `card` may be stale — the poller and the proactive triage arm can target
    // the same card, and a re-triggered card may already be running or done.
    // Only `Todo`/`Ready` are claimable. This is what actually dedupes a
    // double-dispatch: `enforce_single_in_progress` alone does NOT, because
    // re-flipping an already-`InProgress` card to `InProgress` keeps the count
    // at one (→ `Ok`). Thread boards aren't lock-serialised, so a narrow TOCTOU
    // window between this read and the write remains; a fully race-free claim
    // needs a per-thread-locked compare-and-set `ops` primitive (follow-up).
    // Re-load the full current card (not just its status): the passed-in `card`
    // can be stale (edited between selection and claim), so the run must use the
    // fresh objective / plan / assigned_agent, not the snapshot the caller held.
    let fresh_card = ops::list(&location)
        .map_err(|e| format!("[task_dispatcher] reload before claim failed for {card_id}: {e}"))?
        .cards
        .into_iter()
        .find(|c| c.id == card_id)
        .ok_or_else(|| format!("[task_dispatcher] card {card_id} not found on board; skipping"))?;
    if !matches!(
        fresh_card.status,
        TaskCardStatus::Todo | TaskCardStatus::Ready
    ) {
        return Err(format!(
            "[task_dispatcher] card {card_id} not claimable (status: {}); skipping",
            fresh_card.status.as_str()
        ));
    }

    // Plan-approval gate: when required, a `todo` card is parked for human
    // approval before it can run. `Ready` (already approved) bypasses.
    if config.autonomy.require_task_plan_approval && fresh_card.status == TaskCardStatus::Todo {
        ops::update_status(&location, &card_id, TaskCardStatus::AwaitingApproval).map_err(|e| {
            format!("[task_dispatcher] park-for-approval failed for {card_id}: {e}")
        })?;
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

    let prompt = build_task_prompt(&fresh_card);

    // Claim: Todo|Ready→InProgress.
    ops::update_status(&location, &card_id, TaskCardStatus::InProgress)
        .map_err(|e| format!("[task_dispatcher] claim failed for card {card_id}: {e}"))?;

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

    let run_id_for_return = run_id.clone();
    let location_for_run = location.clone();
    tokio::spawn(async move {
        let outcome = run_autonomous(config, &executor, &prompt, &run_id).await;
        write_back(&location_for_run, &card_id, &run_id, outcome);
    });

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

    // 2) Skill (#2824): the same autonomous run, seeded with SKILL.md.
    if let Some(skill) = crate::openhuman::skills::registry::get_skill(workspace_dir, handle) {
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

    with_autonomous_iter_cap(TASK_RUN_MAX_ITERATIONS, agent.run_single(prompt))
        .await
        .map_err(|e| format!("{e:#}"))
}

/// Deterministic board write-back: the dispatcher owns the card lifecycle.
/// Success → `done` + evidence; failure → `blocked` + blocker reason. An
/// external write failure here is logged, never propagated — the run already
/// happened.
fn write_back(
    location: &BoardLocation,
    card_id: &str,
    run_id: &str,
    outcome: Result<String, String>,
) {
    let patch = match &outcome {
        Ok(output) => {
            tracing::info!(
                card_id = %card_id,
                run_id = %run_id,
                output_chars = output.chars().count(),
                "[task_dispatcher] run complete → done"
            );
            CardPatch {
                status: Some(TaskCardStatus::Done),
                evidence: Some(vec![truncate_chars(output.trim(), EVIDENCE_MAX_CHARS)]),
                ..Default::default()
            }
        }
        Err(err) => {
            tracing::warn!(
                card_id = %card_id,
                run_id = %run_id,
                error = %err,
                "[task_dispatcher] run failed → blocked"
            );
            CardPatch {
                status: Some(TaskCardStatus::Blocked),
                blocker: Some(truncate_chars(err, EVIDENCE_MAX_CHARS)),
                ..Default::default()
            }
        }
    };

    if let Err(e) = ops::edit(location, card_id, patch) {
        tracing::error!(
            card_id = %card_id,
            run_id = %run_id,
            error = %e,
            "[task_dispatcher] board write-back failed (run outcome lost from board)"
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

/// One poller tick: dispatch the highest-urgency `todo` card on the
/// task-sources board, if any and if capacity allows. `pub(crate)` so tests can
/// drive a tick without the real interval.
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
    if !config.task_sources.enabled {
        return Ok(());
    }

    let location = BoardLocation::Thread {
        workspace_dir: config.workspace_dir.clone(),
        thread_id: crate::openhuman::task_sources::TASK_SOURCES_THREAD_ID.to_string(),
    };
    let snapshot = ops::list(&location)?;

    // `enforce_single_in_progress` caps the board at one running card, so if
    // one is already in progress there's nothing for this tick to claim.
    if snapshot
        .cards
        .iter()
        .any(|c| c.status == TaskCardStatus::InProgress)
    {
        return Ok(());
    }

    let Some(card) = pick_next_todo(&snapshot.cards) else {
        return Ok(());
    };

    tracing::info!(
        card_id = %card.id,
        urgency = card_urgency(&card),
        "[task_dispatcher:poller] dispatching highest-urgency todo card"
    );
    dispatch_card(location, card).await.map(|_| ())
}

/// Highest-urgency dispatchable card (`todo` or approved `ready`; urgency from
/// `source_metadata.urgency`, default 0.0; ties broken toward the lower board
/// `order`). Returns a clone. `dispatch_card` then either runs a `ready` card
/// or parks a `todo` one for approval, per the autonomy setting.
fn pick_next_todo(cards: &[TaskBoardCard]) -> Option<TaskBoardCard> {
    cards
        .iter()
        .filter(|c| matches!(c.status, TaskCardStatus::Todo | TaskCardStatus::Ready))
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
        let picked = pick_next_todo(&cards).expect("a todo card is available");
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
        assert_eq!(pick_next_todo(&cards).unwrap().id, "early");
    }

    #[test]
    fn poller_returns_none_when_no_todo_cards() {
        let cards = vec![card_with("a", TaskCardStatus::Done, Some(0.9), 0)];
        assert!(pick_next_todo(&cards).is_none());
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
        assert_eq!(pick_next_todo(&cards).unwrap().id, "ready");
    }

    #[test]
    fn poller_prefers_higher_urgency_across_todo_and_ready() {
        let cards = vec![
            card_with("ready-low", TaskCardStatus::Ready, Some(0.3), 0),
            card_with("todo-high", TaskCardStatus::Todo, Some(0.9), 1),
        ];
        assert_eq!(pick_next_todo(&cards).unwrap().id, "todo-high");
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
