//! Background skill run spawning and outcome polling.
//!
//! `spawn_workflow_run_background` is re-used by both the `skills_run`
//! JSON-RPC controller and the `run_skill` agent tool (skill chaining).
//! `await_run_outcome` lets the model poll a spawned run's log file for a
//! terminal result without busy-waiting.

use serde_json::Value;

use crate::openhuman::agent::harness::session::Agent;
use crate::openhuman::agent::harness::subagent_runner::with_autonomous_iter_cap;
use crate::openhuman::config::Config;
use crate::openhuman::skills::{preflight, registry, run_log};

use crate::openhuman::skills::schemas::resolve_workspace_dir;

/// Iteration cap for an autonomous skill run (orchestrator + sub-agents). High
/// enough to "run until done", while the repeated-failure circuit breaker still
/// stops dead-end grinding — deliberately bounded (not infinite) to cap spend.
const WORKFLOW_RUN_MAX_ITERATIONS: usize = 200;

/// Outcome of [`spawn_workflow_run_background`]: the new run's `run_id`, the
/// canonical `workflow_id` the registry resolved it to, and the path of the
/// streaming log file every step + the footer get written to.
pub struct WorkflowRunStarted {
    pub run_id: String,
    pub workflow_id: String,
    pub log_path: std::path::PathBuf,
}

/// Spawn a single autonomous workflow_run as a detached `tokio::spawn`. Used by
/// both the `openhuman.skills_run` JSON-RPC controller and the `run_skill`
/// agent tool (which lets the orchestrator chain one skill into another —
/// e.g. `github-issue-crusher` → `pr-review-shepherd` once the draft PR is
/// open).
///
/// Returns immediately with the run handle; the actual work runs in the
/// background until DONE / DEGENERATE / FAILED. Errors (unknown skill,
/// missing required inputs) surface as `Err(String)` *before* the spawn so
/// callers can reject malformed invocations synchronously.
pub async fn spawn_workflow_run_background(
    skill_id_param: String,
    inputs_param: Option<Value>,
) -> Result<WorkflowRunStarted, String> {
    let workspace = resolve_workspace_dir().await;
    let skill = registry::get_workflow(&workspace, &skill_id_param)
        .ok_or_else(|| format!("workflow_run: unknown skill '{skill_id_param}'"))?;
    let inputs = inputs_param.unwrap_or(Value::Null);
    let missing = registry::missing_required_inputs(&skill.inputs, &inputs);
    if !missing.is_empty() {
        return Err(format!(
            "workflow_run: missing required inputs: {}",
            missing.join(", ")
        ));
    }

    // ── Preflight gates ─────────────────────────────────────────────
    // Run BEFORE the orchestrator is built so failures surface
    // synchronously to the caller (skills_run RPC or the run_skill
    // agent tool) instead of leaking through as cryptic orchestrator
    // output. Today only the [github] gate exists; future gates can
    // chain here.
    if let Some(github_cfg) = skill.github.as_ref() {
        let config_snapshot = match Config::load_or_init().await {
            Ok(c) => c,
            Err(e) => {
                return Err(format!(
                    "workflow_run preflight: failed to load config to gate `{}`: {e:#}",
                    skill.definition.id
                ));
            }
        };
        let probes = preflight::LivePreflightProbes::new(&config_snapshot);
        if let Err(gate_err) = preflight::run_github_preflight(Some(github_cfg), &probes).await {
            let tag = gate_err.tag();
            // Materialise a run-log entry on disk so the gate failure
            // shows up in `<workspace>/skills/.runs/` (and therefore
            // in the FE's "Recent runs" list / log viewer) even though
            // the orchestrator never booted. We write a header then a
            // matching FAILED footer so `scan_runs` parses it cleanly.
            let gate_run_id = uuid::Uuid::new_v4().to_string();
            let gate_log_path =
                run_log::run_log_path(&workspace, &skill.definition.id, &gate_run_id);
            let body = gate_err.to_user_message(Some(&gate_log_path.display().to_string()));
            let header_prompt = format!(
                "preflight gate: github\n\
                 gate decision: FAILED ({tag})\n\
                 detail: {body}"
            );
            if let Err(e) = run_log::write_header(
                &gate_log_path,
                &skill.definition.id,
                &gate_run_id,
                &inputs,
                &header_prompt,
            )
            .await
            {
                tracing::warn!(
                    error = %e,
                    "[skills] preflight gate: failed to write run-log header"
                );
            }
            if let Err(e) = run_log::write_footer(&gate_log_path, "FAILED", 0, &body).await {
                tracing::warn!(
                    error = %e,
                    "[skills] preflight gate: failed to write run-log footer"
                );
            }
            tracing::warn!(
                workflow_id = %skill.definition.id,
                gate = "github",
                tag = %tag,
                gate_log = %gate_log_path.display(),
                "[skills] spawn_workflow_run_background: preflight gate failed"
            );
            return Err(format!("[preflight:github:{tag}] {body}"));
        }
        tracing::info!(
            workflow_id = %skill.definition.id,
            "[skills] spawn_workflow_run_background: github preflight passed"
        );
    }

    // Focus the orchestrator on this single skill: its SKILL.md rides in
    // the task prompt as guidelines + the resolved inputs; the
    // orchestrator's own system prompt and full tool access are kept.
    let guidelines = match &skill.definition.system_prompt {
        crate::openhuman::agent::harness::definition::PromptSource::Inline(s) => s.clone(),
        _ => String::new(),
    };
    let inputs_block = registry::render_inputs_block(&skill.inputs, &inputs);
    let workflow_id = skill.definition.id.clone();
    let task_prompt = format!(
        "You are running a single skill: **{workflow_id}**. Follow these guidelines exactly and \
         focus solely on completing this one task — do not pick up unrelated work.\n\n\
         # Workflow guidelines\n{guidelines}\n\n{inputs_block}",
    );
    let run_id = uuid::Uuid::new_v4().to_string();
    let log_path = run_log::run_log_path(&workspace, &workflow_id, &run_id);
    tracing::info!(
        workflow_id = %workflow_id,
        run_id = %run_id,
        log = %log_path.display(),
        "[skills] spawn_workflow_run_background: starting orchestrator run"
    );

    // Detached: build the orchestrator Agent inside the spawn so config /
    // toolchain are loaded fresh per run; the parent returns the handle
    // immediately. Same flow handle_skills_run used to inline — extracted
    // so the `run_skill` agent tool can re-use it for skill chaining.
    let inherited_origin = crate::openhuman::agent::turn_origin::current()
        .unwrap_or(crate::openhuman::agent::turn_origin::AgentTurnOrigin::Cli);
    {
        let run_id = run_id.clone();
        let workflow_id = workflow_id.clone();
        let inputs = inputs.clone();
        let log_path = log_path.clone();
        let inherited_origin = inherited_origin.clone();
        tokio::spawn(async move {
            if let Err(e) =
                run_log::write_header(&log_path, &workflow_id, &run_id, &inputs, &task_prompt).await
            {
                tracing::warn!(run_id = %run_id, error = %e, "[skills] workflow_run: header write failed");
            }
            let mut config = match Config::load_or_init().await {
                Ok(c) => c,
                Err(e) => {
                    let _ = run_log::write_footer(
                        &log_path,
                        "FAILED",
                        0,
                        &format!("load config: {e:#}"),
                    )
                    .await;
                    return;
                }
            };
            // Only apply the permissive wildcard default when the operator
            // hasn't configured an explicit allow-list — preserve any
            // configured egress policy instead of unconditionally widening it.
            if config.http_request.allowed_domains.is_empty() {
                config.http_request.allowed_domains = vec!["*".to_string()];
            }
            let mut agent = match Agent::from_config_for_agent(&config, "orchestrator") {
                Ok(a) => a,
                Err(e) => {
                    let _ = run_log::write_footer(
                        &log_path,
                        "FAILED",
                        0,
                        &format!("build agent: {e:#}"),
                    )
                    .await;
                    return;
                }
            };
            // Issue #4868 — apply the workflow-run iteration budget AFTER
            // construction. The session builder now stamps `orchestrator`'s
            // definition cap (15) onto the agent; a full workflow run
            // intentionally needs a much larger budget (200), so this must be
            // a post-construction override rather than a pre-set on `config`
            // (which the builder would otherwise clobber).
            agent.set_max_tool_iterations(WORKFLOW_RUN_MAX_ITERATIONS);
            tracing::debug!(
                run_id = %run_id,
                max_tool_iterations = WORKFLOW_RUN_MAX_ITERATIONS,
                "[skills] workflow_run: pinned iteration budget post-construction (overrides \
                 the session builder's orchestrator definition cap)"
            );
            agent.set_event_context(run_id.clone(), "skill");
            agent.set_agent_definition_name(format!(
                "orchestrator-skill-{}",
                &run_id.get(..8).unwrap_or(&run_id)
            ));
            let (tx, rx) = tokio::sync::mpsc::channel(256);
            agent.set_on_progress(Some(tx));
            let bridge = tokio::spawn(run_log::drain_to_log(rx, log_path.clone()));

            // Register the cancellation token now (after the run can actually
            // start) so `skills_cancel` can stop it; a config/agent-build
            // failure above returns before this, leaving nothing to leak.
            let cancel_token = run_log::register_run_cancel(&run_id);

            let started = std::time::Instant::now();
            // Inherit the parent turn's origin so a skill triggered from an
            // ExternalChannel / tainted context retains its provenance
            // through the approval gate. Falls back to Cli for direct
            // user-initiated RPC / CLI flows.
            //
            // Race the run against its cancellation token: if `skills_cancel`
            // fires the token, the run future is dropped (cancelled at its next
            // await) and we record a CANCELLED footer. `Some(_)` ⇒ ran to a
            // natural end; `None` ⇒ cancelled.
            let result = tokio::select! {
                biased;
                _ = cancel_token.cancelled() => None,
                r = crate::openhuman::agent::turn_origin::with_origin(
                    inherited_origin,
                    with_autonomous_iter_cap(
                        WORKFLOW_RUN_MAX_ITERATIONS,
                        agent.run_single(&task_prompt),
                    ),
                ) => Some(r),
            };
            agent.set_on_progress(None);
            drop(agent);
            let _ = bridge.await;

            let ms = started.elapsed().as_millis() as u64;
            run_log::unregister_run_cancel(&run_id);
            match result {
                None => {
                    let _ =
                        run_log::write_footer(&log_path, "CANCELLED", ms, "Run stopped by user.")
                            .await;
                    tracing::info!(run_id = %run_id, "[workflows] workflow_run: cancelled");
                }
                Some(Ok(out)) => {
                    if let Some((line, count)) = run_log::detect_repeated_line(&out, 30, 4) {
                        let preview = line.chars().take(160).collect::<String>();
                        let body = format!(
                            "degenerate-response: autonomous run halted before marking DONE.\n\
                             the model's final assistant message repeats the same line {count}× — \
                             this is the known one-generation low-entropy loop failure mode, not a real result.\n\n\
                             repeated line (truncated to 160 chars):\n  {preview}\n\n\
                             full final output follows below for forensic review:\n\n{out}",
                        );
                        let _ = run_log::write_footer(&log_path, "DEGENERATE", ms, &body).await;
                        tracing::warn!(
                            run_id = %run_id,
                            repeats = count,
                            "[skills] workflow_run: degenerate final response rejected"
                        );
                    } else {
                        let _ = run_log::write_footer(&log_path, "DONE", ms, &out).await;
                        tracing::info!(run_id = %run_id, "[skills] workflow_run: completed");
                    }
                }
                Some(Err(e)) => {
                    let _ = run_log::write_footer(&log_path, "FAILED", ms, &format!("{e:#}")).await;
                    tracing::warn!(run_id = %run_id, error = ?e, "[skills] workflow_run: failed");
                }
            }
        });
    }

    Ok(WorkflowRunStarted {
        run_id,
        workflow_id,
        log_path,
    })
}

/// Poll a spawned run's log file until its terminal footer lands or the
/// `budget` elapses. Returns `Some(outcome)` the moment the footer is
/// readable (DONE / DEGENERATE / FAILED), or `None` if the run is still
/// `RUNNING` when the budget runs out — the caller then auto-detaches and
/// hands back the `run_id` so the work continues in the background.
///
/// The poll happens in the runtime (a tokio sleep loop), NOT in the LLM —
/// the model issues one `run_workflow` tool call and gets either the result
/// or a "still running" handle back, never a busy-wait it has to drive.
pub async fn await_run_outcome(
    log_path: &std::path::Path,
    budget: std::time::Duration,
) -> Option<run_log::RunOutcome> {
    // Tight enough that a fast workflow returns inline promptly; loose
    // enough that polling a finished-but-slow log isn't a hot spin.
    const POLL_INTERVAL: std::time::Duration = std::time::Duration::from_millis(750);
    let deadline = tokio::time::Instant::now() + budget;
    loop {
        if let Some(outcome) = run_log::read_terminal_outcome(log_path) {
            return Some(outcome);
        }
        if tokio::time::Instant::now() >= deadline {
            return None;
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        tokio::time::sleep(POLL_INTERVAL.min(remaining)).await;
    }
}
