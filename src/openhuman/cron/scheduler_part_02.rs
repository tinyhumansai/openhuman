
async fn execute_job_with_retry(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (bool, String) {
    let mut last_output = String::new();
    let mut last_agent_error: Option<String> = None;
    let retries = config.reliability.scheduler_retries;
    let mut backoff_ms = config.reliability.provider_backoff_ms.max(200);
    let mut session_expired = false;
    let mut credits_exhausted = false;
    let mut budget_exhausted = false;
    let mut key_unset = false;
    let mut local_unreachable = false;

    for attempt in 0..=retries {
        let (success, output, agent_error) = match job.job_type {
            JobType::Shell => {
                let (success, output) = run_job_command(config, security, job).await;
                (success, output, None)
            }
            JobType::Agent => run_agent_job(config, job).await,
            JobType::Flow => {
                let (success, output) = run_flow_schedule_job(job);
                (success, output, None)
            }
        };
        last_output = output;
        if agent_error.is_some() {
            last_agent_error = agent_error;
        }

        if success {
            return (true, last_output);
        }

        if last_output.starts_with("blocked by security policy:") {
            // Deterministic policy violations are not retryable.
            return (false, last_output);
        }

        if is_session_expired_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — the inference layer already
            // published `SessionExpired`, retries cannot recover until the
            // user re-auths, and the classifier considers this expected
            // user state (TAURI-RUST-N). See `is_session_expired_failure`
            // for the full rationale.
            session_expired = true;
            break;
        }

        if is_insufficient_credits_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a BYO provider 402 (out of
            // balance) is permanent across the backoff loop, and the
            // provider emit site already demoted it from Sentry. Skipping
            // the retries-exhausted `report_error` below keeps the residual
            // off Sentry at source, independent of the `before_send` chain
            // (TAURI-RUST-514). See `is_insufficient_credits_failure`.
            // Metadata-only log (no raw provider body — see CLAUDE.md).
            log::debug!(
                "[cron] action=halt_on_insufficient_credits_402 job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            credits_exhausted = true;
            break;
        }

        if is_budget_exhausted_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a managed-backend budget 400
            // (USER_INSUFFICIENT_CREDITS) is permanent across the backoff
            // loop. The tag-gated `is_budget_event` before_send filter never
            // matches this cron re-report, so suppressing the report here
            // keeps it off Sentry at source (TAURI-RUST-BMW). See
            // `is_budget_exhausted_failure`. Metadata-only log (no raw body).
            log::debug!(
                "[cron] action=halt_on_budget_exhausted_400 job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            budget_exhausted = true;
            break;
        }

        if is_api_key_unset_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a configured provider with no
            // API key fails deterministically at the credential guard before
            // any HTTP, so the missing key is permanent across the backoff
            // loop. The bare cron `report_error` below bypasses the
            // `ApiKeyMissing` `expected_error_kind` demotion, so suppressing
            // here keeps the residual off Sentry at source (TAURI-RUST-HCK).
            // The failure stays visible to the user via the alerts tab
            // (`push_cron_alert`) + run history. See `is_api_key_unset_failure`.
            // Metadata-only log (no raw provider body — see CLAUDE.md).
            log::debug!(
                "[cron] action=halt_on_api_key_unset job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            key_unset = true;
            break;
        }

        if is_local_provider_unreachable_failure(
            &job.job_type,
            last_agent_error.as_deref(),
            last_output.as_str(),
        ) {
            // Halt on the first occurrence — a local LLM provider refusing the
            // loopback connection (LM Studio / Ollama not running) cannot
            // recover across the backoff loop, and the provider/agent emit
            // sites already demoted it from Sentry (`LoopbackUnavailable`).
            // The bare cron `report_error` below bypasses that demotion, so
            // suppressing here keeps the residual off Sentry at source
            // (TAURI-RUST-12K). The failure stays visible via the run history
            // + cron alert. See `is_local_provider_unreachable_failure`.
            // Metadata-only log (no raw provider body — see CLAUDE.md).
            log::debug!(
                "[cron] action=halt_on_local_provider_unreachable job_id={} attempt={} retries={}",
                job.id.as_str(),
                attempt,
                retries
            );
            local_unreachable = true;
            break;
        }

        if attempt < retries {
            let jitter_ms = u64::from(Utc::now().timestamp_subsec_millis() % 250);
            time::sleep(Duration::from_millis(backoff_ms + jitter_ms)).await;
            backoff_ms = (backoff_ms.saturating_mul(2)).min(30_000);
        }
    }

    // Permanent user-config / billing states are demoted at source: halt the
    // loop and skip the retries-exhausted report, independent of the tag-gated
    // before_send filters that the cron re-report does not match. Covers BYO
    // 402 out-of-credit + managed-backend 400 out-of-budget (TAURI-RUST-514 /
    // -BMW) and a configured provider with no API key (TAURI-RUST-HCK). The
    // `session_expired` (TAURI-RUST-N) and `local_unreachable` (a local LLM
    // server refusing the loopback connection, TAURI-RUST-12K) halts are the
    // same shape — suppress the bypassing bare report — but carry no
    // user-config remediation surface, so they gate the report directly rather
    // than routing through `permanent_config_halt`'s UserErrorCenter swap.
    let permanent_config_halt = credits_exhausted || budget_exhausted || key_unset;
    if matches!(job.job_type, JobType::Agent)
        && !session_expired
        && !local_unreachable
        && !permanent_config_halt
    {
        let report_message = last_agent_error.as_deref().unwrap_or(last_output.as_str());
        crate::core::observability::report_error(
            report_message,
            "cron",
            "agent_job",
            &[
                ("job_id", job.id.as_str()),
                ("agent_id", job.agent_id.as_deref().unwrap_or("none")),
                (
                    "session_target",
                    agent_session_target_tag(&job.session_target),
                ),
                ("failure", "retries_exhausted"),
            ],
        );
    } else if matches!(job.job_type, JobType::Agent) && permanent_config_halt {
        // Suppressed the retries-exhausted Sentry report for a permanent
        // user-config / billing state. Metadata-only breadcrumb so the
        // suppression is diagnosable in production without the raw provider body.
        let (reason, user_error_kind) = if credits_exhausted {
            ("insufficient_credits_402", "insufficient_credits")
        } else if budget_exhausted {
            ("budget_exhausted_400", "budget_exceeded")
        } else {
            ("api_key_unset", "api_key_missing")
        };
        log::debug!(
            "[cron] action=suppress_retries_exhausted_report reason={reason} job_id={} retries={}",
            job.id.as_str(),
            retries
        );
        // Replace the generic agent-failure copy with the specific, actionable
        // (static, leak-safe) reason so the hoisted /notifications alert + run
        // history tell the user the exact next step rather than "Something went
        // wrong" (CodeRabbit #4169). The raw `last_agent_error` chain is NEVER
        // surfaced here — only the `&'static str` constants from
        // `permanent_halt_message`.
        last_output = permanent_halt_message(credits_exhausted, budget_exhausted).to_string();
        // Also surface the actionable state to the UserErrorCenter so the user
        // can fix it (add an API key / top up credits / raise the budget) even
        // with no chat thread open. Broadcast-only + metadata-only — see
        // `publish_cron_user_error` (#4165 / TAURI-RUST-HCK follow-up).
        publish_cron_user_error(user_error_kind);
    }

    (false, last_output)
}

/// Static, leak-safe actionable alert copy for a permanent cron halt state.
/// Returns the user-facing `/notifications` body matching the halt reason —
/// `&'static str` only, so it can never carry a raw error field (the no-leak
/// contract that governs [`agent_error_to_user_message`]). Precedence mirrors
/// the halt classifiers' evaluation order: credits → budget → missing key.
fn permanent_halt_message(credits_exhausted: bool, budget_exhausted: bool) -> &'static str {
    if credits_exhausted {
        CRON_HALT_INSUFFICIENT_CREDITS_MESSAGE
    } else if budget_exhausted {
        CRON_HALT_BUDGET_EXHAUSTED_MESSAGE
    } else {
        CRON_HALT_API_KEY_UNSET_MESSAGE
    }
}

/// Surface a permanent cron user-config / billing halt to every connected
/// client's UserErrorCenter.
///
/// Broadcasts a metadata-only `user_error` web-channel event to the `"system"`
/// room (which every socket auto-joins). The payload carries only the stable
/// `kind` token in `error_type` — one of `api_key_missing` / `insufficient_credits`
/// / `budget_exceeded`, mirroring the frontend `UserErrorKind` discriminator —
/// plus `error_source = "cron"`. It NEVER carries the raw provider body (see the
/// metadata-only rule in CLAUDE.md), so no secrets / PII leave the core.
///
/// The frontend `socketService` listens for `user_error` and routes it through
/// the same classifier the chat runtime uses, so a background (no-delivery) job
/// failure is no longer silent — it lands in the shell's UserErrorCenter with a
/// deep-link action even though no chat thread is active.
fn publish_cron_user_error(kind: &str) {
    log::debug!("[cron] action=surface_user_error kind={kind}");
    crate::openhuman::web_chat::publish_web_channel_event(crate::core::socketio::WebChannelEvent {
        event: "user_error".to_string(),
        client_id: "system".to_string(),
        error_type: Some(kind.to_string()),
        error_source: Some("cron".to_string()),
        ..Default::default()
    });
}

async fn process_due_jobs(config: &Config, security: &Arc<SecurityPolicy>, jobs: Vec<CronJob>) {
    let max_concurrent = config.scheduler.max_concurrent.max(1);
    let mut in_flight = stream::iter(jobs.into_iter().map(|job| {
        let config = config.clone();
        let security = Arc::clone(security);
        async move { execute_and_persist_job(&config, security.as_ref(), &job).await }
    }))
    .buffer_unordered(max_concurrent);

    while let Some((job_id, success, failure_message)) = in_flight.next().await {
        if success {
            BUS.publish(DomainEvent::HealthChanged {
                component: "scheduler".to_string(),
                healthy: true,
                message: None,
            });
        } else {
            BUS.publish(DomainEvent::HealthChanged {
                component: "scheduler".to_string(),
                healthy: false,
                message: Some(failure_message.unwrap_or_else(|| format!("job {job_id} failed"))),
            });
        }
    }
}

async fn execute_and_persist_job(
    config: &Config,
    security: &SecurityPolicy,
    job: &CronJob,
) -> (String, bool, Option<String>) {
    warn_if_high_frequency_agent_job(job);

    let started_at = Utc::now();

    BUS.publish(DomainEvent::CronJobTriggered {
        job_id: job.id.clone(),
        job_name: job.name.clone().unwrap_or_default(),
        job_type: format!("{:?}", job.job_type),
    });

    let (execution_success, output) = execute_job_with_retry(config, security, job).await;
    let finished_at = Utc::now();
    let success = persist_job_result(
        config,
        job,
        execution_success,
        &output,
        started_at,
        finished_at,
    )
    .await;

    BUS.publish(DomainEvent::CronJobCompleted {
        job_id: job.id.clone(),
        success,
        output: crate::openhuman::util::truncate_with_ellipsis(&output, 512),
    });
    let failure_message =
        (!success).then(|| crate::openhuman::util::truncate_with_ellipsis(&output, 256));

    (job.id.clone(), success, failure_message)
}

async fn run_agent_job(config: &Config, job: &CronJob) -> (bool, String, Option<String>) {
    let name = job.name.clone().unwrap_or_else(|| "cron-job".to_string());
    let prompt = job.prompt.clone().unwrap_or_default();
    let prefixed_prompt = format!("[cron:{} {name}] {prompt}", job.id);

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
    // Agent::from_config defaults.
    if let Some(ref agent_id) = job.agent_id {
        if let Some(registry) =
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
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
                // every other provider 404 on names like `agentic-v1`.
                // Route Hint specs through the per-workload factory so
                // we get the exact model the user has configured for
                // that workload, regardless of which provider it lives
                // on. Inherit / Exact keep their literal `resolve()`
                // behaviour because neither relies on the `-v1` trick.
                use crate::openhuman::agent::harness::definition::ModelSpec;
                let fallback_model = effective
                    .default_model
                    .clone()
                    .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string());
                let resolved_model = match &def.model {
                    ModelSpec::Hint(workload) => {
                        // Resolve the workload's configured model id via the crate
                        // `ChatModel` factory (#4249 Phase 1). We only need the
                        // resolved model string here, so the built model is
                        // discarded — `create_chat_model_with_model_id` wraps the
                        // same `create_chat_provider` resolution, so the model id is
                        // identical; temperature is irrelevant to id resolution.
                        match crate::openhuman::inference::provider::create_chat_model_with_model_id(
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
                    "[cron] agent_id not found in registry — falling back to generic agent"
                );
            }
        } else {
            tracing::warn!(
                job_id = %job.id,
                "[cron] AgentDefinitionRegistry not initialized — falling back to generic agent"
            );
        }
    }

    let run_result = match job.session_target {
        SessionTarget::Main | SessionTarget::Isolated => {
            tracing::debug!(
                job_id = %job.id,
                target = ?job.session_target,
                "[cron] building isolated agent for scheduled job"
            );
            match build_agent_for_cron_job(&effective, job) {
                Ok(BuiltCronAgent { mut agent, profile }) => {
                    // Tag events so downstream subscribers can correlate
                    // cron-triggered turns. `cron` is the channel so the
                    // event bus can filter from other flows (`cli`, `web`…).
                    agent.set_event_context(format!("cron:{}", job.id), "cron");
                    // Scope a `TrustedAutomation { Cron }` origin around the
                    // turn. The approval gate treats this as user-authorized
                    // automation and lets external_effect tools run without
                    // an in-app prompt — the user explicitly created this
                    // cron job and authorized its prompt at the same time.
                    let origin =
                        crate::openhuman::agent::turn_origin::AgentTurnOrigin::TrustedAutomation {
                            job_id: job.id.clone(),
                            source:
                                crate::openhuman::agent::turn_origin::TrustedAutomationSource::Cron,
                        };
                    let turn = crate::openhuman::memory::source_scope::with_source_scope(
                        profile.and_then(|profile| profile.memory_sources),
                        crate::openhuman::agent::turn_origin::with_origin(
                            origin,
                            agent.run_single(&prefixed_prompt),
                        ),
                    );
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
                        crate::openhuman::agent::harness::with_task_recency_window(
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

/// Fires a `JobType::Flow` job: publishes `DomainEvent::FlowScheduleTick` for
/// the bound flow id (stored in `job.command`, see `JobType::Flow`'s doc) and
/// returns immediately. This job type does no work itself — dispatching the
/// actual `flows::ops::flows_run` happens asynchronously in
/// `flows::bus::FlowTriggerSubscriber`, which is the sole consumer of this
/// event (kept out of the cron domain so cron stays flow-agnostic).
fn run_flow_schedule_job(job: &CronJob) -> (bool, String) {
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
const EMPTY_AGENT_OUTPUT: &str = "agent job executed";

/// Resolve the agent profile a cron job is attributed to, if any.
///
/// Returns `Some(profile)` only when `job.profile_id` is set AND that profile
/// still exists in the store. A deleted profile yields `Ok(None)` so the caller
/// runs the job without a profile rather than failing it (2b). Profile-store
/// failures are returned: attribution must not fail open when the scheduler
/// cannot determine whether the referenced profile still exists.
fn resolve_cron_profile(
    config: &Config,
    job: &CronJob,
) -> anyhow::Result<Option<crate::openhuman::agent::profiles::AgentProfile>> {
    let Some(profile_id) = job.profile_id.as_deref() else {
        return Ok(None);
    };
    match crate::openhuman::agent::profiles::load_profiles(&config.workspace_dir) {
        Ok(state) => {
            let found = state.profiles.into_iter().find(|p| p.id == profile_id);
            if found.is_none() {
                tracing::warn!(
                    job_id = %job.id,
                    profile_id = %profile_id,
                    "[cron] attributed profile no longer exists — running job without a profile"
                );
            }
            Ok(found)
        }
        Err(e) => Err(anyhow::anyhow!(
            "failed to load attributed profile {profile_id:?} for cron job {}: {e}",
            job.id
        )),
    }
}

struct BuiltCronAgent {
    agent: Agent,
    profile: Option<crate::openhuman::agent::profiles::AgentProfile>,
}

fn apply_cron_profile_runtime_defaults(
    config: &Config,
    job: &CronJob,
    profile: &crate::openhuman::agent::profiles::AgentProfile,
) -> Config {
    let mut effective = config.clone();
    if let Some(model) = profile.model_override.clone() {
        effective.default_model = Some(model);
    }
    if let Some(temperature) = profile.temperature {
        effective.default_temperature = temperature;
    }
    // A job-level pin is the most specific model choice.
    if let Some(model) = job.model.clone() {
        effective.default_model = Some(model);
    }
    effective
}

fn build_agent_for_cron_job(config: &Config, job: &CronJob) -> anyhow::Result<BuiltCronAgent> {
    // 2b — profile attribution. When the job names a profile that still exists,
    // build the run under it via the SAME profile-aware session path the task
    // dispatcher uses (`from_config_for_agent_with_profile`), so the run inherits
    // the profile's SOUL, memory scope, dedicated-workspace descriptor, and
    // tool/skill/MCP allowlists. A deleted profile falls through (warned in
    // `resolve_cron_profile`) to the profile-less path below.
    if let Some(profile) = resolve_cron_profile(config, job)? {
        // Apply the same profile runtime defaults as interactive chat. A
        // per-job model pin remains the most specific choice and therefore
        // wins over the profile model. The profile-aware builder consumes the
        // prompt suffix directly and gives profile temperature precedence over
        // the selected agent definition.
        let effective = apply_cron_profile_runtime_defaults(config, job, &profile);
        // A job may pin a built-in `agent_id`; otherwise the profile picks its
        // own agent definition.
        let agent_id = job
            .agent_id
            .clone()
            .unwrap_or_else(|| profile.agent_id.clone());
        let agent = Agent::from_config_for_agent_with_profile(
            &effective,
            &agent_id,
            profile.system_prompt_suffix.clone(),
            Some(&profile),
        )
        .inspect(|_| {
            tracing::debug!(
                job_id = %job.id,
                profile_id = %profile.id,
                agent_id = %agent_id,
                "[cron] built scheduled job agent under attributed profile"
            );
        })
        .map_err(|e| {
            anyhow::anyhow!(
                "failed to build cron job {} under attributed profile {:?} with agent {:?}: {e:#}",
                job.id,
                profile.id,
                agent_id
            )
        })?;
        return Ok(BuiltCronAgent {
            agent,
            profile: Some(profile),
        });
    }

    if let Some(agent_id) = job.agent_id.as_deref() {
        match Agent::from_config_for_agent(config, agent_id) {
            Ok(agent) => {
                tracing::debug!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    "[cron] built scheduled job agent from definition"
                );
                Ok(BuiltCronAgent {
                    agent,
                    profile: None,
                })
            }
            Err(e) => {
                tracing::warn!(
                    job_id = %job.id,
                    agent_id = %agent_id,
                    error = %e,
                    "[cron] failed to build agent from definition; falling back to generic agent"
                );
                Agent::from_config(config).map(|agent| BuiltCronAgent {
                    agent,
                    profile: None,
                })
            }
        }
    } else {
        Agent::from_config(config).map(|agent| BuiltCronAgent {
            agent,
            profile: None,
        })
    }
}
