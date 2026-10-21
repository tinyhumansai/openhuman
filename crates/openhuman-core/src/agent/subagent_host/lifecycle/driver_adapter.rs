use super::*;
use tinyagents_harness::tool::ToolSnapshot;

pub(crate) struct OpenHumanDriverBundle {
    pub(crate) driver:
        Arc<SubagentDriver<crate::agent::tinyagents::host::OpenHumanRunContext, HostRequest>>,
    pub(crate) failures: Arc<Mutex<HashMap<SubagentTaskKey, SubagentRunError>>>,
    pub(crate) host_outcome: Arc<Mutex<HashMap<SubagentTaskKey, SubagentRunOutcome>>>,
    pub(super) configured_tool_result: Arc<Mutex<HashMap<SubagentTaskKey, tinytools::ToolResult>>>,
    pub(crate) persistence: Arc<OpenHumanPersistence>,
}

impl OpenHumanDriverBundle {
    #[cfg(test)]
    pub(crate) fn new(checkpoint_dir: PathBuf) -> Result<Self, SubagentRunError> {
        Self::with_in_flight(
            checkpoint_dir,
            tinyagents_orchestration::subagent::SubagentInFlight::default(),
        )
    }

    #[cfg(test)]
    pub(crate) fn with_in_flight(
        checkpoint_dir: PathBuf,
        in_flight: tinyagents_orchestration::subagent::SubagentInFlight,
    ) -> Result<Self, SubagentRunError> {
        Self::with_in_flight_for(
            checkpoint_dir,
            in_flight,
            crate::core::runtime::DispatchCtx::ambient(),
        )
    }

    pub(crate) fn with_in_flight_for(
        checkpoint_dir: PathBuf,
        in_flight: tinyagents_orchestration::subagent::SubagentInFlight,
        dispatch: crate::core::runtime::DispatchCtx,
    ) -> Result<Self, SubagentRunError> {
        let failures = Arc::new(Mutex::new(HashMap::new()));
        let host_outcome = Arc::new(Mutex::new(HashMap::new()));
        let configured_tool_result = Arc::new(Mutex::new(HashMap::new()));
        let plans = Arc::new(Mutex::new(HashMap::new()));
        let planner = Arc::new(OpenHumanPlanner {
            plans: Arc::clone(&plans),
        });
        let executor = Arc::new(OpenHumanExecutor {
            plans,
            failures: Arc::clone(&failures),
            host_outcome: Arc::clone(&host_outcome),
            configured_tool_result: Arc::clone(&configured_tool_result),
        });
        let persistence = Arc::new(
            OpenHumanPersistence::new(checkpoint_dir)
                .with_dispatch(dispatch)
                .with_host_outcome(Arc::clone(&host_outcome)),
        );
        let lifecycle_persistence: Arc<
            dyn tinyagents_orchestration::subagent::SubagentPersistence,
        > = persistence.clone();
        let driver = Arc::new(
            SubagentDriver::new(SubagentCapabilities {
                planner: Some(planner),
                executor: Some(executor),
                persistence: Some(lifecycle_persistence),
            })
            .map_err(map_lifecycle_error)?
            .with_shared_in_flight(in_flight),
        );
        Ok(Self {
            driver,
            failures,
            host_outcome,
            configured_tool_result,
            persistence,
        })
    }
}

#[derive(Clone)]
pub(crate) struct HostRequest {
    pub(super) invocation: HostInvocation,
    pub(crate) options: SubagentRunOptions,
}

#[derive(Clone)]
pub(super) enum HostInvocation {
    Agent(crate::agent_registry::host_definition::AgentDefinition),
    ConfiguredTool {
        agent_id: String,
        tool: Arc<dyn tinytools::Tool>,
        arguments: serde_json::Value,
    },
}

impl HostRequest {
    pub(super) fn agent_id(&self) -> &str {
        match &self.invocation {
            HostInvocation::Agent(definition) => &definition.id,
            HostInvocation::ConfiguredTool { agent_id, .. } => agent_id,
        }
    }
}

struct PreparedHostPlan {
    invocation: HostInvocation,
    options: SubagentRunOptions,
}

struct OpenHumanPlanner {
    plans: Arc<Mutex<HashMap<SubagentTaskKey, PreparedHostPlan>>>,
}

#[async_trait]
impl SubagentPlanner<crate::agent::tinyagents::host::OpenHumanRunContext, HostRequest>
    for OpenHumanPlanner
{
    async fn prepare(
        &self,
        request: SubagentRequest<crate::agent::tinyagents::host::OpenHumanRunContext, HostRequest>,
    ) -> Result<PreparedSubagent<crate::agent::tinyagents::host::OpenHumanRunContext>, SubagentError>
    {
        let request = request.into_parts();
        let task_key = request.task_key.clone();
        let agent_id = request.host_request.agent_id().to_owned();
        let mut options = request.host_request.options;
        let input = request.input;
        // The generic persistence seam supplies the pause *before* host
        // options are prepared. A continuation's new user answer belongs
        // after that durable history, exactly once; never overwrite a
        // caller-provided answer with the loaded checkpoint.
        let planned_input = if let Some(resume) = request.resume {
            let history = super::merged_resume_history(resume.history, input);
            options.initial_history = Some(
                history
                    .iter()
                    .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
                    .collect(),
            );
            history
        } else {
            vec![Message::user(input)]
        };
        options.task_id = Some(request.task_key.task_id.clone());
        options.run_context = request.run_context.data.clone();
        self.plans.lock().unwrap_or_else(|p| p.into_inner()).insert(
            task_key,
            PreparedHostPlan {
                invocation: request.host_request.invocation.clone(),
                options,
            },
        );
        // The host leaf resolves the real tool snapshot and policy; an empty
        // declaration here advertises no unresolved authority.
        Ok(PreparedSubagent::new(
            request.task_key.task_id,
            agent_id,
            planned_input,
            ToolSnapshot::default(),
            request.run_context,
        ))
    }
}

struct OpenHumanExecutor {
    plans: Arc<Mutex<HashMap<SubagentTaskKey, PreparedHostPlan>>>,
    failures: Arc<Mutex<HashMap<SubagentTaskKey, SubagentRunError>>>,
    host_outcome: Arc<Mutex<HashMap<SubagentTaskKey, SubagentRunOutcome>>>,
    configured_tool_result: Arc<Mutex<HashMap<SubagentTaskKey, tinytools::ToolResult>>>,
}

#[async_trait]
impl SubagentExecutor<crate::agent::tinyagents::host::OpenHumanRunContext> for OpenHumanExecutor {
    async fn execute(
        &self,
        execution: SubagentExecution<crate::agent::tinyagents::host::OpenHumanRunContext>,
    ) -> Result<SubagentOutcome, SubagentError> {
        let task_key = execution.task_key.clone();
        let PreparedHostPlan {
            invocation,
            mut options,
        } = self
            .plans
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .remove(&task_key)
            .ok_or_else(|| {
                SubagentError::Execution("OpenHuman subagent plan was consumed".into())
            })?;
        let native_run_config = execution.prepared.run_context.config.clone();
        let parent_events = execution.prepared.run_context.events.clone();
        // Both carriers observe one token.  The generic driver installs this
        // token on the prepared RunContext; the host graph polls the copy in
        // its explicit OpenHuman carrier.
        options.run_context = execution.prepared.run_context.data.clone();
        options.run_context.cancellation = execution.cancellation.clone();
        options.message_run_queue = execution.prepared.run_context.run_queue.clone();
        let dispatch = options.run_context.dispatch_ctx();
        let origin = options.run_context.origin.clone();
        let input = execution
            .prepared
            .input
            .last()
            .map(Message::text)
            .unwrap_or_default();
        match invocation {
            HostInvocation::Agent(definition) => {
                let native = crate::agent::tinyagents::NativeSubagentRunContext {
                    depth: native_run_config.lineage.depth,
                    config: native_run_config,
                    parent_events,
                    child_events: std::sync::Arc::default(),
                    child_journal: std::sync::Arc::default(),
                    name: definition.id.clone(),
                };
                let run = Box::pin(
                    crate::agent::subagent_host::ops::run_subagent_direct_with_native_context(
                        &definition,
                        &input,
                        options.clone(),
                        native,
                    ),
                );
                let result = match dispatch.into_core() {
                    Some(core) => {
                        crate::core::runtime::CoreContext::scope_with_turn_origin(core, origin, run)
                            .await
                    }
                    None => run.await,
                };
                match result {
                    Ok(outcome) => {
                        self.host_outcome
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(task_key, outcome.clone());
                        Ok(host_outcome_to_neutral(outcome, &definition, &options))
                    }
                    Err(error) => {
                        self.failures
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(task_key, error);
                        Err(SubagentError::Execution(
                            "OpenHuman execution failed".into(),
                        ))
                    }
                }
            }
            HostInvocation::ConfiguredTool {
                agent_id,
                tool,
                arguments,
            } => {
                let tool_context = tinyagents_harness::tool::ToolExecutionContext::from_run_context(
                    &execution.prepared.run_context,
                    tinyagents_harness::CallId::new(format!("delegate-{}", task_key.task_id)),
                );
                let cancellation = execution.cancellation.clone();
                let invoke = Box::pin(async move {
                    tokio::select! {
                        _ = cancellation.cancelled() => (
                            Ok(tinytools::ToolResult::error(
                                "delegate: cancelled before the configured agent completed"
                            )),
                            true,
                        ),
                        result = tool.execute_with_context(
                            arguments,
                            tinytools::ToolCallOptions::default(),
                            Some(&tool_context),
                        ) => (result, false),
                    }
                });
                let result = match dispatch.into_core() {
                    Some(core) => {
                        crate::core::runtime::CoreContext::scope_with_turn_origin(
                            core, origin, invoke,
                        )
                        .await
                    }
                    None => invoke.await,
                };
                match result {
                    (Ok(tool_result), cancelled) => {
                        let output = tool_result.output();
                        let status = if cancelled {
                            SubagentRunStatus::Cancelled
                        } else if tool_result.is_error {
                            SubagentRunStatus::Incomplete {
                                reason: "configured delegate reported failure".into(),
                                failure_cause: None,
                            }
                        } else {
                            SubagentRunStatus::Completed
                        };
                        self.host_outcome
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(
                                task_key.clone(),
                                SubagentRunOutcome {
                                    task_id: task_key.task_id.clone(),
                                    agent_id: agent_id.clone(),
                                    output: output.clone(),
                                    iterations: 1,
                                    failure_cause: None,
                                    elapsed: std::time::Duration::ZERO,
                                    mode: crate::agent::subagent_host::SubagentMode::Typed,
                                    status,
                                    final_history: Vec::new(),
                                    usage: SubagentUsage::default(),
                                    artifact_paths: Vec::new(),
                                    persistence_disposition: tinyagents_orchestration::subagent::SubagentPersistenceDisposition::TerminalInserted,
                                },
                            );
                        let mut neutral = SubagentOutcome::cancelled(task_key.task_id.clone());
                        neutral.output = output;
                        neutral
                            .extensions
                            .insert("agent_id".into(), serde_json::Value::String(agent_id));
                        neutral
                            .extensions
                            .insert("iterations".into(), serde_json::Value::Number(1.into()));
                        neutral.status = if cancelled {
                            SubagentOutcomeKind::Cancelled
                        } else if tool_result.is_error {
                            SubagentOutcomeKind::Incomplete(SubagentIncomplete::new(
                                "configured delegate reported failure",
                            ))
                        } else {
                            SubagentOutcomeKind::Completed
                        };
                        self.configured_tool_result
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(task_key, tool_result);
                        Ok(neutral)
                    }
                    (Err(error), _) => {
                        self.failures
                            .lock()
                            .unwrap_or_else(|p| p.into_inner())
                            .insert(task_key, SubagentRunError::Provider(error));
                        Err(SubagentError::Execution(
                            "configured delegate execution failed".into(),
                        ))
                    }
                }
            }
        }
    }
}

/// Durable OpenHuman pause/terminal records scoped by the complete neutral
/// task key.  The key is encoded rather than using a task id as a filename, so
/// equally named children of separate roots/parents can never resume or close
/// one another's lifecycle.
use super::outcome::host_outcome_to_neutral;
use super::persistence::OpenHumanPersistence;

pub(in crate::agent::subagent_host) fn outcome_to_host(
    result: SubagentRunResult,
    host: Option<SubagentRunOutcome>,
    persistence: &OpenHumanPersistence,
    task_key: &SubagentTaskKey,
    fallback_agent_id: &str,
) -> SubagentRunOutcome {
    let persistence_disposition = result.disposition;
    let outcome = result.outcome;
    let neutral_failure_cause = match &outcome.status {
        SubagentOutcomeKind::Incomplete(_) => failure_cause_from_extensions(&outcome.extensions),
        SubagentOutcomeKind::Completed
        | SubagentOutcomeKind::AwaitingInput(_)
        | SubagentOutcomeKind::Cancelled => None,
    };
    let elapsed_from_neutral = outcome
        .extensions
        .get("elapsed_ns")
        .and_then(serde_json::Value::as_u64)
        .map(std::time::Duration::from_nanos);
    // A driver that lost a pause/terminal CAS may have a perfectly valid local
    // execution result, but it is not the authoritative lifecycle outcome.
    // Reconstruct the durable winner instead of leaking the loser’s output,
    // usage, or artifacts to an observer.
    let host = if persistence_disposition.should_emit_host_effects() {
        host
    } else {
        None
    };
    let durable_metadata = if host.is_none() {
        let extensions = &outcome.extensions;
        let agent_id = extensions
            .get("agent_id")
            .and_then(serde_json::Value::as_str)
            .filter(|agent_id| !agent_id.is_empty())
            .map(str::to_owned);
        let elapsed_ms = extensions
            .get("elapsed_ms")
            .and_then(serde_json::Value::as_u64);
        match (agent_id, elapsed_ms) {
            (Some(agent_id), elapsed_ms) => Some((agent_id, elapsed_ms.unwrap_or_default())),
            (None, Some(elapsed_ms)) => persistence
                .durable_host_metadata(task_key)
                .ok()
                .flatten()
                .or_else(|| Some((fallback_agent_id.to_owned(), elapsed_ms))),
            (None, None) => persistence.durable_host_metadata(task_key).ok().flatten(),
        }
    } else {
        None
    };
    let mut host = host.unwrap_or_else(|| SubagentRunOutcome {
        task_id: outcome.task_id.clone(),
        agent_id: durable_metadata
            .as_ref()
            .map(|(agent_id, _)| agent_id.clone())
            .unwrap_or_else(|| fallback_agent_id.to_owned()),
        output: outcome.output.clone(),
        iterations: outcome.usage.calls as usize,
        failure_cause: neutral_failure_cause.clone(),
        elapsed: std::time::Duration::from_millis(
            durable_metadata
                .map(|(_, elapsed_ms)| elapsed_ms)
                .unwrap_or_default(),
        ),
        mode: crate::agent::subagent_host::SubagentMode::Typed,
        status: SubagentRunStatus::Completed,
        final_history: outcome
            .history
            .iter()
            .filter_map(tinyagents_runtime::transcript::message_to_native_transcript)
            .collect(),
        usage: SubagentUsage {
            input_tokens: outcome.usage.usage.input_tokens,
            output_tokens: outcome.usage.usage.output_tokens,
            cached_input_tokens: outcome.usage.usage.cache_read_tokens,
            charged_amount_usd: outcome
                .usage
                .usage
                .charged_amount
                .map(|amount| amount.micros as f64 / 1_000_000.0)
                .unwrap_or_default(),
            cost_source: outcome
                .cost_source
                .unwrap_or_else(|| cost_source_of(outcome.usage.usage.charged_amount.as_ref())),
        },
        artifact_paths: outcome
            .artifacts
            .iter()
            .map(|artifact| artifact.id.clone())
            .collect(),
        persistence_disposition,
    });
    if let Some(elapsed) = elapsed_from_neutral {
        host.elapsed = elapsed;
    }
    let is_awaiting_input = matches!(outcome.status, SubagentOutcomeKind::AwaitingInput(_));
    let status = match outcome.status {
        SubagentOutcomeKind::Completed => SubagentRunStatus::Completed,
        SubagentOutcomeKind::AwaitingInput(pause) => SubagentRunStatus::AwaitingUser {
            question: pause.reason,
            options: None,
            checkpoint: None,
        },
        SubagentOutcomeKind::Incomplete(incomplete) => SubagentRunStatus::Incomplete {
            reason: incomplete.reason,
            failure_cause: neutral_failure_cause.clone(),
        },
        SubagentOutcomeKind::Cancelled => SubagentRunStatus::Cancelled,
    };
    host.status = status;
    host.persistence_disposition = persistence_disposition;
    if matches!(host.status, SubagentRunStatus::Incomplete { .. }) {
        if host.failure_cause.is_none() {
            host.failure_cause = neutral_failure_cause;
        }
    } else {
        host.failure_cause = None;
    }
    if is_awaiting_input {
        host.status = SubagentRunStatus::AwaitingUser {
            question: match host.status {
                SubagentRunStatus::AwaitingUser { ref question, .. } => question.clone(),
                _ => String::new(),
            },
            options: None,
            checkpoint: Some(persistence.pause_path(task_key)),
        };
    }
    host
}

fn failure_cause_from_extensions(
    extensions: &std::collections::BTreeMap<String, serde_json::Value>,
) -> Option<tinyagents_harness::no_progress::ClassifiedStopCause> {
    let cause = extensions.get("failure_cause")?.as_object()?;
    Some(tinyagents_harness::no_progress::ClassifiedStopCause {
        class: cause.get("class")?.as_str()?.to_owned(),
        operation: cause.get("operation")?.as_str()?.to_owned(),
        summary: cause.get("summary")?.as_str()?.to_owned(),
    })
}

pub(super) fn map_lifecycle_error(error: SubagentError) -> SubagentRunError {
    SubagentRunError::Provider(anyhow::anyhow!(error.to_string()))
}
/// A child run's cost as it comes back across the orchestration boundary.
/// That `Usage` can only say whether a charge is present, so a present one is
/// read as charged and an absent one as unknown (it was sent as absent).
fn cost_source_of(charged: Option<&ChargedAmount>) -> tinyagents_harness::cost::CostSource {
    match charged {
        Some(_) => tinyagents_harness::cost::CostSource::Charged,
        None => tinyagents_harness::cost::CostSource::Unknown,
    }
}

#[cfg(test)]
#[path = "driver_adapter_tests.rs"]
mod tests;
