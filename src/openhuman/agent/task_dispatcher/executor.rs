//! Executor resolution and autonomous run logic.
//!
//! Resolves a card's `assigned_agent` to a concrete executor (personality,
//! skill, or built-in agent) and drives the autonomous agent turn, writing
//! the outcome back to the board when done.

use std::path::Path;

use crate::openhuman::agent::harness::definition::{AgentDefinitionRegistry, PromptSource};
use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::harness::subagent_runner::with_autonomous_iter_cap;
use crate::openhuman::agent::task_board::TaskCardStatus;
use crate::openhuman::agent::task_session;
use crate::openhuman::config::Config;
use crate::openhuman::profiles::PersonalityContext;
use crate::openhuman::todos::ops::{self, BoardLocation, CardPatch};
use crate::openhuman::todos::runs::{self, RunOutcome};

use super::types::ResolvedExecutor;

/// Max chars of a personality SOUL.md / MEMORY.md or skill guideline block
/// folded into the agent's system-prompt suffix.
pub(super) const EXECUTOR_PREAMBLE_MAX_CHARS: usize = 800;

/// Tool-iteration ceiling for an autonomous task run. Matches the skill-run
/// cap — a task brief is the same shape of bounded autonomous work.
pub(super) const TASK_RUN_MAX_ITERATIONS: usize = 200;

/// Max chars of the agent's final output retained as board `evidence`.
pub(super) const EVIDENCE_MAX_CHARS: usize = 2_000;

/// Map a card's `assigned_agent` handle to one of three executor presets:
/// **personality** (scoped SOUL/MEMORY folded into the prompt suffix, run as
/// that profile's agent), **skill** (orchestrator seeded with the skill's
/// `SKILL.md` guidelines), or **built-in agent**. An unset or unresolved handle
/// degrades to the default `orchestrator` — "use the personality if valid,
/// otherwise the default agent."
pub(super) fn resolve_executor(workspace_dir: &Path, assigned: Option<&str>) -> ResolvedExecutor {
    let Some(handle) = assigned.map(str::trim).filter(|s| !s.is_empty()) else {
        return ResolvedExecutor::default_agent();
    };
    if handle == "orchestrator" {
        return ResolvedExecutor::default_agent();
    }

    // 1) Personality (#2895): a user-defined profile with scoped identity.
    if let Ok(state) = crate::openhuman::profiles::load_profiles(workspace_dir) {
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
                profile: Some(profile.clone()),
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
            profile: None,
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
            profile: None,
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
pub(super) async fn run_autonomous(
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
        executor.profile.as_ref(),
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
    // Gate memory-source recall for this background run to the profile's
    // allowlist (None = unrestricted), mirroring the web chat turn.
    let memory_scope = executor
        .profile
        .as_ref()
        .and_then(|p| p.memory_sources.clone());
    let run = crate::openhuman::memory::source_scope::with_source_scope(
        memory_scope,
        crate::openhuman::agent::turn_origin::with_origin(
            crate::openhuman::agent::turn_origin::AgentTurnOrigin::Cli,
            with_autonomous_iter_cap(TASK_RUN_MAX_ITERATIONS, agent.run_single(prompt)),
        ),
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
                    // Background/cron turns don't surface in the chat footer; their
                    // token/cost spend is still captured by the global cost tracker.
                    None,
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
pub(super) fn write_back(
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

/// Current persisted status of a card, or `None` if the board can't be read or
/// the card is gone. Used by `write_back` to detect a run that blocked itself.
fn current_card_status(location: &BoardLocation, card_id: &str) -> Option<TaskCardStatus> {
    ops::list(location)
        .ok()
        .and_then(|snap| snap.cards.into_iter().find(|c| c.id == card_id))
        .map(|c| c.status)
}

pub(super) fn truncate_chars(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
