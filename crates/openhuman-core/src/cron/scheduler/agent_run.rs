//! Execution of `JobType::Agent` and `JobType::Flow` jobs: agent construction
//! (definition selection) and the single scheduled turn.

use super::delivery::is_morning_briefing_job;
use super::failure_classification::classify_agent_anyhow_for_user;
use super::origin_context::build_run_prompt;
use super::origin_delivery::{fallback_reminder, is_no_reply, job_allows_no_reply};
use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use {tinyflows_bus::schedule::types::CronJob, tinyflows_bus::schedule::types::SessionTarget};

/// Recency window the morning briefing installs around its turn so Composio
/// task-fetch tools only surface tasks created/changed in the last day. Read
/// by the `composio_execute` handler via the run context (`OpenHumanRunExt`).
pub(super) const MORNING_BRIEFING_TASK_RECENCY_SECS: u64 = 24 * 60 * 60;

pub(super) async fn run_agent_job_for_run(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    run_id: &str,
) -> (bool, String, Option<String>) {
    let name = job.name.clone().unwrap_or_else(|| "cron-job".to_string());
    let prefixed_prompt = build_run_prompt(ctx, config, job, &name).await;

    // Apply per-job model override onto a cloned Config, so the Agent
    // sees it through the normal `default_model` path without mutating
    // the caller's config.
    let mut effective = config.clone();
    if let Some(model) = job.model.clone() {
        effective.default_model = Some(model);
    }

    // When an agent_id is set, resolve the built-in definition and apply
    // its model hint, iteration cap, and prompt body so the cron job
    // runs with the definition's constraints instead of the generic
    // OpenHumanSessionHost::from_config defaults.
    let selected_agent_id = job.agent_id.as_deref().unwrap_or("orchestrator");
    // A host-registered agent (`agent::host_agents`) carries its own
    // definition, model and route; the registry overrides below are for
    // registry agents only.
    let is_host_agent = job
        .agent_id
        .as_deref()
        .is_some_and(|id| crate::agent::host_agents::resolve(id).is_some());
    if is_host_agent {
        tracing::debug!(
            job_id = %job.id,
            agent_id = %selected_agent_id,
            "[cron] job targets a host-registered agent"
        );
    } else {
        let agent_id = selected_agent_id;
        if let Some(registry) =
            crate::agent_registry::host_definition::AgentDefinitionRegistry::current()
        {
            if let Some(def) = registry.get(agent_id) {
                tracing::debug!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    max_iterations = def.max_iterations,
                    "[cron] applying agent definition overrides"
                );
                // Resolve the agent definition's model spec into an
                // exact model id. `ModelSpec::resolve` synthesises
                // `{hint}-v1` for Hint specs, which only the OpenHuman
                // backend understands as a tier hint — Anthropic and
                // every other provider 404 on names like `hint:agentic`.
                // Route Hint specs through the per-workload factory so
                // we get the exact model the user has configured for
                // that workload, regardless of which provider it lives
                // on. Inherit / Exact keep their literal `resolve()`
                // behaviour because neither relies on the `-v1` trick.
                use tinyagents_definition::ModelSpec;
                let fallback_model = effective.default_model.clone().or_else(|| {
                    crate::inference::registry::provider_default_model(
                        crate::inference::provider::factory::PROVIDER_OPENHUMAN,
                    )
                });
                let resolved_model = match &def.model {
                    ModelSpec::Hint(workload) => {
                        // Resolve the workload's configured model id via the crate
                        // `ChatModel` factory (#4249 Phase 1). We only need the
                        // resolved model string here, so the built model is
                        // discarded — `create_chat_model_with_model_id` wraps the
                        // same `create_chat_provider` resolution, so the model id is
                        // identical; temperature is irrelevant to id resolution.
                        match crate::inference::provider::create_chat_model_with_model_id_in(
                            ctx,
                            workload,
                            &effective,
                            effective.default_temperature,
                        )
                        .await
                        {
                            Ok((_, m)) => {
                                tracing::debug!(
                                    job_id = %job.id,
                                    agent_id = %agent_id,
                                    workload = %workload,
                                    model = %m,
                                    "[cron] resolved Hint via workload factory"
                                );
                                Some(m)
                            }
                            Err(e) => {
                                tracing::warn!(
                                    job_id = %job.id,
                                    agent_id = %agent_id,
                                    workload = %workload,
                                    error = %e,
                                    fallback = ?fallback_model,
                                    "[cron] workload factory failed; using fallback model"
                                );
                                fallback_model.clone()
                            }
                        }
                    }
                    ModelSpec::Inherit => fallback_model.clone(),
                    ModelSpec::Exact(name) => Some(name.clone()),
                };
                if let Some(model) = resolved_model {
                    effective.default_model = Some(model);
                }
                // Issue #4868 — the iteration cap is no longer set here. The
                // session builder (`build_session_agent_inner`) resolves it
                // from `def.effective_max_iterations()` directly, which (unlike
                // this cron path previously) correctly honors
                // `iteration_policy = "extended"` agents (e.g. `tools_agent`
                // getting 50, not the raw `max_iterations = 10`).
            } else {
                tracing::warn!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    "[cron] agent_id not found in registry — falling back to canonical orchestrator"
                );
            }
        } else {
            tracing::warn!(
                job_id = %job.id,
                "[cron] AgentDefinitionRegistry not initialized — falling back to canonical orchestrator"
            );
        }
    }

    let run_result = match job.session_target {
        SessionTarget::Main | SessionTarget::Isolated | SessionTarget::Current => {
            tracing::debug!(
                job_id = %job.id,
                target = ?job.session_target,
                "[cron] building isolated agent for scheduled job"
            );
            match build_agent_for_cron_job(ctx, &effective, job).await {
                Ok(BuiltCronAgent { mut agent, context }) => {
                    // Tag events so downstream subscribers can correlate
                    // cron-triggered turns. `cron` is the channel so the
                    // event bus can filter from other flows (`cli`, `web`…).
                    agent.set_event_context(format!("cron:{}", job.id), "cron");
                    agent.enable_headless_descendant_settlement();
                    let timeout_secs = effective.agent.agent_timeout_secs;
                    let deadline = tinyagents_harness::turn_deadline::TurnDeadline::starting_now(
                        std::time::Duration::from_secs(timeout_secs),
                    );
                    let absolute_deadline =
                        tokio::time::Instant::from_std(deadline.started() + deadline.backstop());
                    let mut host_env = agent.host_env().clone();
                    host_env.turn_deadline = Some(deadline);
                    agent.set_host_env(host_env);
                    // A cron agent has no thread, so its first turn would fall
                    // back to `ResumeMode::LatestForAgent` and resume the newest
                    // unthreaded transcript for the agent name — some unrelated
                    // conversation, with its frozen system prompt and stale tool
                    // names. Every run starts from a fresh prompt instead.
                    start_cron_turn_clean(&mut agent);
                    // Morning briefing only: Composio task-fetch tools
                    // (Linear/ClickUp/Notion/Asana) surface only recently
                    // created/changed tasks. Other cron agents and all chat
                    // turns leave the window unset. It rides the run context
                    // (`OpenHumanRunExt`), not a task-local.
                    if is_morning_briefing_job(job) {
                        tracing::debug!(
                            job_id = %job.id,
                            recency_window_secs = MORNING_BRIEFING_TASK_RECENCY_SECS,
                            "[cron] applying morning-briefing task recency window"
                        );
                        agent.set_task_recency_window(Some(std::time::Duration::from_secs(
                            MORNING_BRIEFING_TASK_RECENCY_SECS,
                        )));
                    }
                    // One absolute deadline bounds the turn and any corrective turn.
                    let run = Box::pin(run_with_corrective_turn(
                        &mut agent,
                        context.as_ref(),
                        job,
                        run_id,
                        &prefixed_prompt,
                    ));
                    match tokio::time::timeout_at(absolute_deadline, run).await {
                        Ok(result) => result,
                        Err(_) => Err(anyhow::anyhow!(
                            "cron agent turn timed out after {timeout_secs}s"
                        )),
                    }
                }
                Err(e) => Err(e),
            }
        }
    };

    match run_result {
        Ok(response) => (
            true,
            if response.trim().is_empty() {
                EMPTY_AGENT_OUTPUT.to_string()
            } else {
                response
            },
            None,
        ),
        Err(e) => {
            // Classify into a canned user-facing message *before* logging
            // anything that touches `e`. The classifier output is a
            // `&'static str` — it never contains any data derived from `e`.
            // The raw error is preserved as `last_agent_error` for the
            // observability pipeline (`report_error`), where stack traces
            // and provider URLs are appropriate; it must NOT reach the
            // user-visible notification body.
            let user_message = classify_agent_anyhow_for_user(&e);
            // Preserve the FULL anyhow chain (`{:#}`), not just the top-level
            // message: the loopback-unreachable classifier and the observability
            // pipeline key on the transport cause (`… tcp connect error: Connection
            // refused (os error N)`), which a bare `to_string()` drops.
            (false, user_message.to_string(), Some(format!("{e:#}")))
        }
    }
}

/// The whole prompt of a corrective turn: a delivery request only, never the
/// job prompt, so an action the first turn took is not asked for again.
pub(super) const NO_REPLY_CORRECTION: &str = "Your last reply was NO_REPLY, which is not allowed for this task. Write the message to send to the user now. Do not call tools or repeat any action.";

/// Run the scheduled turn. A job that must deliver and answers `NO_REPLY`
/// (or nothing) gets one corrective turn; if that still yields no message the
/// host's fallback reminder is the run's output.
async fn run_with_corrective_turn(
    agent: &mut OpenHumanSessionHost,
    context: Option<&std::sync::Arc<crate::core::runtime::CoreContext>>,
    job: &CronJob,
    run_id: &str,
    prompt: &str,
) -> anyhow::Result<String> {
    let response = Box::pin(run_cron_turn(agent, context, job, run_id, prompt)).await?;
    if job_allows_no_reply(job) || !needs_correction(&response) {
        return Ok(response);
    }
    tracing::info!(
        job_id = %job.id,
        run_id = %run_id,
        "[cron] run answered NO_REPLY but the job must deliver; running a corrective turn"
    );
    agent.set_next_turn_overrides(crate::agent::session_host::TurnOverrides {
        suppress_tools: true,
        ..Default::default()
    });
    let response = Box::pin(run_cron_turn(
        agent,
        context,
        job,
        run_id,
        NO_REPLY_CORRECTION,
    ))
    .await?;
    if !needs_correction(&response) {
        return Ok(response);
    }
    tracing::info!(
        job_id = %job.id,
        run_id = %run_id,
        "[cron] corrective turn still produced no message; delivering the fallback reminder"
    );
    Ok(fallback_reminder(job))
}

fn needs_correction(response: &str) -> bool {
    response.trim().is_empty() || is_no_reply(response)
}

/// One scheduled turn under the run's origin, in the host agent's context
/// when there is one.
async fn run_cron_turn(
    agent: &mut OpenHumanSessionHost,
    context: Option<&std::sync::Arc<crate::core::runtime::CoreContext>>,
    job: &CronJob,
    run_id: &str,
    prompt: &str,
) -> anyhow::Result<String> {
    // A job with no channel origin runs as `TrustedAutomation { Cron }`: the
    // approval gate treats it as user-authorized automation (the user created
    // the job and authorized its prompt together). A job created from a
    // channel chat runs as that channel's turn instead, so its external-effect
    // tools stay gated like its creator's were.
    let origin = crate::cron::origin::turn_origin_for_job_run(job, run_id);
    let turn =
        crate::core::runtime::turn_origin::with_origin(origin, Box::pin(agent.run_single(prompt)));
    // A host agent's turn runs in its own context, so every ambient read
    // (config, domains, tool groups, session store) sees that agent rather
    // than the process default.
    let turn: std::pin::Pin<
        Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>,
    > = match context {
        Some(context) => Box::pin(crate::core::runtime::CoreContext::scope(
            context.clone(),
            turn,
        )),
        None => Box::pin(turn),
    };
    turn.await
}

/// Placeholder recorded in run history when an agent job succeeds but returns
/// no text. Never delivered to chat — used only for the run-history record.
pub(super) const EMPTY_AGENT_OUTPUT: &str = "agent job executed";

/// Keep a scheduled turn from auto-resuming another session's transcript.
///
/// Mirrors `flows::ops::builder::start_builder_turn_clean`: an unthreaded
/// session's first turn otherwise resumes the newest transcript on disk for
/// the agent name, whatever conversation that was.
pub(super) fn start_cron_turn_clean(agent: &mut OpenHumanSessionHost) {
    tracing::debug!("[cron] suppressing transcript autoload for scheduled turn");
    agent.set_next_turn_overrides(cron_turn_overrides());
}

/// Overrides applied to each scheduled agent turn before its first dispatch.
pub(super) fn cron_turn_overrides() -> crate::agent::session_host::TurnOverrides {
    crate::agent::session_host::TurnOverrides {
        suppress_transcript_autoload: true,
        ..Default::default()
    }
}

pub(super) struct BuiltCronAgent {
    pub(crate) agent: OpenHumanSessionHost,
    /// The host agent's context the turn must run in, when `job.agent_id`
    /// named a host-registered agent (`agent::host_agents`).
    pub(crate) context: Option<std::sync::Arc<crate::core::runtime::CoreContext>>,
}

/// Build the session a cron agent job runs: the host-registered agent named
/// by `job.agent_id` when there is one, else the registry definition.
pub(super) async fn build_agent_for_cron_job(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
) -> anyhow::Result<BuiltCronAgent> {
    let host_agent = job
        .agent_id
        .as_deref()
        .and_then(crate::agent::host_agents::resolve);
    build_cron_agent(ctx, config, job, host_agent).await
}

#[cfg(test)]
#[path = "agent_run_tests.rs"]
pub(super) mod tests;

#[cfg(test)]
pub(super) use tests::run_agent_job;

async fn build_cron_agent(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    job: &CronJob,
    host_agent: Option<crate::agent::host_agents::HostAgent>,
) -> anyhow::Result<BuiltCronAgent> {
    if let Some(host) = host_agent {
        // The host agent's own config (provider model and route applied),
        // with the job's model override on top, as for a registry agent. The
        // caller's config is not used: it is the process default's.
        let mut effective = host.config.clone();
        if let Some(model) = job.model.clone() {
            effective.default_model = Some(model);
        }
        let agent = host.session_host(&effective, None).await?;
        tracing::debug!(
            job_id = %job.id,
            agent_id = %host.definition.id,
            "[cron] built scheduled job agent from host agent"
        );
        return Ok(BuiltCronAgent {
            agent,
            context: Some(host.context),
        });
    }
    let agent_id = job.agent_id.as_deref().unwrap_or("orchestrator");
    match OpenHumanSessionHost::from_config_for_agent(ctx, config, agent_id).await {
        Ok(agent) => {
            tracing::debug!(
                job_id = %job.id,
                agent_id = %agent_id,
                "[cron] built scheduled job agent from definition"
            );
            Ok(BuiltCronAgent {
                agent,
                context: None,
            })
        }
        Err(e) => {
            tracing::warn!(
                job_id = %job.id,
                agent_id = %agent_id,
                error = %e,
                "[cron] failed to build agent from definition; falling back to canonical orchestrator"
            );
            OpenHumanSessionHost::from_config_for_agent(ctx, config, "orchestrator")
                .await
                .map(|agent| BuiltCronAgent {
                    agent,
                    context: None,
                })
        }
    }
}
