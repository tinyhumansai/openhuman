//! Execution of `JobType::Agent` and `JobType::Flow` jobs: agent construction
//! (definition selection) and the single scheduled turn.

use super::delivery::is_morning_briefing_job;
use super::failure_classification::classify_agent_anyhow_for_user;
use super::origin_context::build_run_prompt;
use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use {tinyflows_schedule::types::CronJob, tinyflows_schedule::types::SessionTarget};

/// Recency window the morning briefing installs around its turn so Composio
/// task-fetch tools only surface tasks created/changed in the last day. Read
/// by the `composio_execute` handler via `current_task_recency_window`.
pub(super) const MORNING_BRIEFING_TASK_RECENCY_SECS: u64 = 24 * 60 * 60;

/// [`run_agent_job_for_run`] under a fresh run id.
#[cfg(test)]
pub(super) async fn run_agent_job(
    config: &Config,
    job: &CronJob,
) -> (bool, String, Option<String>) {
    let run_id = uuid::Uuid::new_v4().to_string();
    run_agent_job_for_run(config, job, &run_id).await
}

pub(super) async fn run_agent_job_for_run(
    config: &Config,
    job: &CronJob,
    run_id: &str,
) -> (bool, String, Option<String>) {
    let name = job.name.clone().unwrap_or_else(|| "cron-job".to_string());
    let prefixed_prompt = build_run_prompt(config, job, &name).await;

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
            crate::agent::harness::definition::AgentDefinitionRegistry::current()
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
                use crate::agent::harness::definition::ModelSpec;
                let fallback_model = effective
                    .default_model
                    .clone()
                    .unwrap_or_else(|| crate::config::DEFAULT_MODEL.to_string());
                let resolved_model = match &def.model {
                    ModelSpec::Hint(workload) => {
                        // Resolve the workload's configured model id via the crate
                        // `ChatModel` factory (#4249 Phase 1). We only need the
                        // resolved model string here, so the built model is
                        // discarded — `create_chat_model_with_model_id` wraps the
                        // same `create_chat_provider` resolution, so the model id is
                        // identical; temperature is irrelevant to id resolution.
                        match crate::inference::provider::create_chat_model_with_model_id(
                            workload,
                            &effective,
                            effective.default_temperature,
                        ) {
                            Ok((_, m)) => {
                                tracing::debug!(
                                    job_id = %job.id,
                                    agent_id = %agent_id,
                                    workload = %workload,
                                    model = %m,
                                    "[cron] resolved Hint via workload factory"
                                );
                                m
                            }
                            Err(e) => {
                                tracing::warn!(
                                    job_id = %job.id,
                                    agent_id = %agent_id,
                                    workload = %workload,
                                    error = %e,
                                    fallback = %fallback_model,
                                    "[cron] workload factory failed; using fallback model"
                                );
                                fallback_model.clone()
                            }
                        }
                    }
                    ModelSpec::Inherit => fallback_model.clone(),
                    ModelSpec::Exact(name) => name.clone(),
                };
                effective.default_model = Some(resolved_model);
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
            match build_agent_for_cron_job(&effective, job) {
                Ok(BuiltCronAgent { mut agent, context }) => {
                    // Tag events so downstream subscribers can correlate
                    // cron-triggered turns. `cron` is the channel so the
                    // event bus can filter from other flows (`cli`, `web`…).
                    agent.set_event_context(format!("cron:{}", job.id), "cron");
                    // A cron agent has no thread, so its first turn would fall
                    // back to `ResumeMode::LatestForAgent` and resume the newest
                    // unthreaded transcript for the agent name — some unrelated
                    // conversation, with its frozen system prompt and stale tool
                    // names. Every run starts from a fresh prompt instead.
                    start_cron_turn_clean(&mut agent);
                    // Scope the run's origin around the turn. A job with no
                    // channel origin runs as `TrustedAutomation { Cron }`: the
                    // approval gate treats it as user-authorized automation
                    // (the user created the job and authorized its prompt
                    // together). A job created from a channel chat runs as
                    // that channel's turn instead, so its external-effect
                    // tools stay gated like its creator's were.
                    let origin = crate::cron::origin::turn_origin_for_job_run(job, run_id);
                    let turn = crate::agent::turn_origin::with_origin(
                        origin,
                        agent.run_single(&prefixed_prompt),
                    );
                    // A host agent's turn runs in its own context, so every
                    // ambient read (config, domains, tool groups, session
                    // store) sees that agent rather than the process default.
                    let turn: std::pin::Pin<
                        Box<dyn std::future::Future<Output = anyhow::Result<String>> + Send + '_>,
                    > = match context {
                        Some(context) => {
                            Box::pin(crate::core::runtime::CoreContext::scope(context, turn))
                        }
                        None => Box::pin(turn),
                    };
                    // Morning briefing only: install a 24h task-recency window
                    // so Composio task-fetch tools (Linear/ClickUp/Notion/Asana)
                    // surface only recently created/changed tasks. Other cron
                    // agents and all chat turns leave the window unset.
                    if is_morning_briefing_job(job) {
                        tracing::debug!(
                            job_id = %job.id,
                            recency_window_secs = MORNING_BRIEFING_TASK_RECENCY_SECS,
                            "[cron] applying morning-briefing task recency window"
                        );
                        crate::integrations::composio::task_recency_context::with_task_recency_window(
                            std::time::Duration::from_secs(MORNING_BRIEFING_TASK_RECENCY_SECS),
                            turn,
                        )
                        .await
                    } else {
                        tracing::trace!(
                            job_id = %job.id,
                            "[cron] task recency window not applied for this job"
                        );
                        turn.await
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

/// Fires a `JobType::Flow` job. A system job (`system:<name>`, see
/// [`crate::cron::system_jobs`]) publishes `DomainEvent::CronSystemJobDue` and,
/// when a handler is registered for it
/// ([`crate::cron::system_job_handlers`]), awaits that handler and returns its
/// result;
/// any other publishes `DomainEvent::FlowScheduleTick` for
/// the bound flow id (stored in `job.command`, see `JobType::Flow`'s doc) and
/// returns immediately. This job type does no work itself — dispatching the
/// actual `flows::ops::flows_run` happens asynchronously in
/// `flows::bus::FlowTriggerSubscriber`, which is the sole consumer of this
/// event (kept out of the cron domain so cron stays flow-agnostic).
pub(super) async fn run_flow_schedule_job(job: &CronJob) -> (bool, String) {
    if let Some(name) = crate::cron::system_jobs::system_job_name(job) {
        tracing::info!(job_id = %job.id, job = %name, "[cron] system job due — publishing CronSystemJobDue");
        BUS.publish(DomainEvent::CronSystemJobDue {
            job: name.to_string(),
        });
        // A host-registered handler is awaited and its result is the run's
        // result; without one the job is announce-only and `ok` on dispatch.
        let ctx = crate::cron::system_job_handlers::SystemJobContext {
            job_id: job.id.clone(),
            name: name.to_string(),
        };
        return match crate::cron::system_job_handlers::dispatch(ctx).await {
            Some(Ok(())) => (true, format!("system job {name} completed")),
            Some(Err(error)) => (false, format!("system job {name} failed: {error}")),
            None => (true, format!("system job {name} dispatched")),
        };
    }
    let flow_id = job.command.clone();
    tracing::info!(
        target: "flows",
        job_id = %job.id,
        %flow_id,
        "[cron] flow schedule tick — publishing FlowScheduleTick"
    );
    BUS.publish(DomainEvent::FlowScheduleTick {
        flow_id: flow_id.clone(),
    });
    (
        true,
        format!("flow schedule tick emitted for flow {flow_id}"),
    )
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
pub(super) fn build_agent_for_cron_job(
    config: &Config,
    job: &CronJob,
) -> anyhow::Result<BuiltCronAgent> {
    let host_agent = job
        .agent_id
        .as_deref()
        .and_then(crate::agent::host_agents::resolve);
    build_cron_agent(config, job, host_agent)
}

fn build_cron_agent(
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
        let agent = host.session_host(&effective, None)?;
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
    match OpenHumanSessionHost::from_config_for_agent(config, agent_id) {
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
            OpenHumanSessionHost::from_config_for_agent(config, "orchestrator").map(|agent| {
                BuiltCronAgent {
                    agent,
                    context: None,
                }
            })
        }
    }
}
