//! OpenHuman's model-and-harness implementation of the neutral session driver.
//!
//! This adapter deliberately has no transcript, prefix, or generic history
//! state.  `tinyagents_runtime::Session` owns those concerns and gives this
//! type the complete immutable model history and tool declaration snapshot for
//! one invocation.

mod grounded_close;

use std::{collections::HashSet, sync::Arc, time::Instant};

use async_trait::async_trait;
use tinyagents_runtime::{
    DriverFailure, DriverOutcome, DriverRequest, RuntimeError, SessionDriver, TranscriptPartial,
};
use tinyinference_llm::message::Message;
use tinytools_agent::dialect::ToolDialect;

use crate::agent::{
    messages::ChatMessage,
    session_host::turn::graph::{self, ChatTurnGraph},
    tinyagents::{TurnModelSource, host::OpenHumanHostBase, host::OpenHumanRunContext},
};

/// Immutable host composition consumed by [`OpenHumanSessionDriver`].
///
/// Mutable per-turn policy belongs in `OpenHumanSessionHooks`; the driver only
/// receives its final context, history, and immutable tool declaration view.
pub struct OpenHumanSessionDriver {
    turn_model_source: TurnModelSource,
    dispatcher: Arc<dyn ToolDialect>,
    model_name: String,
    temperature: f64,
    max_iterations: usize,
    model_vision: bool,
    run_queue:
        Option<Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    workspace: Option<tinytools::WorkspaceDescriptor>,
    sandbox_mode: crate::agent::harness::definition::SandboxMode,
    hosted_base: Option<Arc<OpenHumanHostBase>>,
    agent_id: String,
}

impl OpenHumanSessionDriver {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn new(
        turn_model_source: TurnModelSource,
        dispatcher: Arc<dyn ToolDialect>,
        model_name: String,
        temperature: f64,
        max_iterations: usize,
        model_vision: bool,
        run_queue: Option<
            Arc<tinyagents_harness::run_queue::RunQueue<crate::agent::queued_turn::QueuedTurn>>,
        >,
        workspace: Option<tinytools::WorkspaceDescriptor>,
        sandbox_mode: crate::agent::harness::definition::SandboxMode,
        hosted_base: Option<Arc<OpenHumanHostBase>>,
        agent_id: String,
    ) -> Self {
        Self {
            turn_model_source,
            dispatcher,
            model_name,
            temperature,
            max_iterations,
            model_vision,
            run_queue,
            workspace,
            sandbox_mode,
            hosted_base,
            agent_id,
        }
    }
}

#[async_trait]
impl SessionDriver<OpenHumanRunContext> for OpenHumanSessionDriver {
    async fn execute(
        &self,
        request: DriverRequest<OpenHumanRunContext>,
    ) -> Result<DriverOutcome, DriverFailure> {
        // These two inputs are prepared by the OpenHuman lifecycle hook for
        // this exact request.  Falling back to the snapshots captured when the
        // long-lived driver was constructed would let a later channel/tool
        // policy or context change execute under stale authority.
        let tool_policy = request
            .run_context
            .data
            .tool_policy
            .clone()
            .ok_or_else(|| {
                driver_failure("session turn is missing its request-scoped tool policy")
            })?;
        let mut context_mw = request
            .run_context
            .data
            .context_middleware
            .clone()
            .ok_or_else(|| driver_failure("session turn is missing its request-scoped context"))?;
        // The harness drops its `AgentRun` on an error.  Snapshot the exact
        // request boundary before assembling the graph so the error path can
        // persist only provider-accepted rounds, plus a display-only failure
        // record, without manufacturing a second host history.
        let snapshot = context_mw
            .transcript_snapshot
            .get_or_insert_with(|| {
                Arc::new(std::sync::Mutex::new(
                    crate::agent::tinyagents::TranscriptSnapshot {
                        request_base_len: request.history.len(),
                        ..Default::default()
                    },
                ))
            })
            .clone();
        if let Ok(mut snapshot_state) = snapshot.lock() {
            snapshot_state
                .pricing_model
                .get_or_insert_with(|| self.model_name.clone());
        }
        let sidecar = request.run_context.data.session_sidecar.clone();
        let started = Instant::now();
        let user_message = request
            .history
            .iter()
            .rev()
            .find(|message| matches!(message, Message::User(_)))
            .map(Message::text)
            .unwrap_or_default();
        let context_window = self
            .turn_model_source
            .effective_context_window(&self.model_name)
            .await;
        let turn_models = self
            .turn_model_source
            .build(
                &self.model_name,
                self.temperature,
                context_window,
                request.run_context.data.thread_id.as_deref(),
            )
            .map_err(driver_error)?;

        let mut messages: Vec<ChatMessage> = request
            .history
            .iter()
            .map(crate::agent::message_convert::message_to_native_chat_message)
            .collect();
        if (turn_models.supports_vision() || self.model_vision)
            && crate::agent::multimodal::has_image_placeholders(&messages)
        {
            messages = crate::agent::multimodal::rehydrate_image_placeholders(&messages);
        }

        // Runtime's `ToolSnapshot` remains the model declaration and final
        // allowlist. The actual executable instances must be supplied by the
        // request hook as well: falling back to a long-lived driver's tool
        // registry would let revoked or hidden tools execute under stale
        // authority.
        let tools = request
            .run_context
            .data
            .current_tools
            .clone()
            .ok_or_else(|| driver_failure("session turn is missing request-scoped tools"))?;
        let synthesized_tools = request
            .run_context
            .data
            .current_synthesized_tools
            .clone()
            .ok_or_else(|| {
                driver_failure("session turn is missing request-scoped synthesized tools")
            })?;
        let visible_tool_names: HashSet<String> = request
            .tools
            .specs()
            .iter()
            .map(|spec| spec.name.clone())
            .collect();
        ensure_snapshot_tools_are_executable(&visible_tool_names, &tools, &synthesized_tools)?;
        let run_context = request.run_context.data.clone();
        let outcome = match graph::run_chat_turn_graph(ChatTurnGraph {
            turn_models,
            model: self.model_name.clone(),
            messages,
            tools,
            synthesized_tools,
            visible_tool_names,
            max_iterations: self.max_iterations,
            on_progress: run_context.progress.clone(),
            context_window,
            run_queue: self.run_queue.clone(),
            context_mw,
            tool_policy: Some(tool_policy),
            workspace_descriptor: self.workspace.clone(),
            sandbox_mode: self.sandbox_mode,
            thread_id: run_context.thread_id.clone(),
            run_context,
            hosted_base: self.hosted_base.clone(),
            agent_id: self.agent_id.clone(),
        })
        .await
        {
            Ok(outcome) => outcome,
            Err(error) => {
                return Err(driver_error_with_snapshot(
                    error,
                    &snapshot,
                    &sidecar,
                    started.elapsed(),
                    &self.model_name,
                ));
            }
        };

        let close = grounded_close::close_if_needed(
            &self.turn_model_source,
            &self.model_name,
            self.temperature,
            request.run_context.data.thread_id.as_deref(),
            self.dispatcher.as_ref(),
            &request.history,
            &user_message,
            &outcome,
        )
        .await;
        let mut output = close
            .as_ref()
            .map(|close| close.output.clone())
            .unwrap_or_else(|| outcome.text.clone());
        let mut history = request.history;
        let conversation = crate::agent::message_convert::provider_messages_from_conversation(
            self.dispatcher.as_ref(),
            &outcome.conversation,
        );
        let mut appended = crate::agent::message_convert::history_to_messages(&conversation);
        if close.is_some() {
            // A blank or breaker-halt terminal row is not the durable answer.
            // Retain every completed tool round and replace only that final
            // assistant row with the verified/deterministic grounded close.
            if appended
                .last()
                .is_some_and(|message| matches!(message, Message::Assistant(_)))
            {
                appended.pop();
            }
            appended.push(Message::assistant(output.clone()));
        }
        history.extend(appended);

        let required_output = request.run_context.data.required_output.clone();
        let required_repair = match required_output.as_ref() {
            Some(contract) => {
                grounded_close::repair_required_output(
                    &self.turn_model_source,
                    &self.model_name,
                    self.temperature,
                    request.run_context.data.thread_id.as_deref(),
                    self.dispatcher.as_ref(),
                    contract,
                    &history,
                    &output,
                    close.is_none() && request.run_context.data.progress.is_some(),
                    request.run_context.data.progress.as_ref(),
                    outcome.model_calls.saturating_add(1) as u32,
                )
                .await
            }
            None => None,
        };
        if let Some(repair) = &required_repair {
            output = repair.output.clone();
            if history
                .last()
                .is_some_and(|message| matches!(message, Message::Assistant(_)))
            {
                history.pop();
            }
            history.push(Message::assistant(output.clone()));
        }

        // This is deliberately an out-of-band observation rather than a
        // second history or transcript.  The runtime only reads it from
        // `after_commit`, so a failed/cancelled turn cannot publish its usage
        // or tool outcomes as if they had become durable.
        {
            let repair_usage = required_repair.as_ref().map(|repair| &repair.usage);
            let mut observed = sidecar
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            observed.model_calls = outcome.model_calls
                + close
                    .as_ref()
                    .map(|close| close.usage.model_calls)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.model_calls)
                    .unwrap_or_default();
            observed.tool_calls = outcome.tool_calls;
            observed.input_tokens = outcome.input_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.input_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.input_tokens)
                    .unwrap_or_default();
            observed.output_tokens = outcome.output_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.output_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.output_tokens)
                    .unwrap_or_default();
            observed.cached_input_tokens = outcome.cached_input_tokens
                + close
                    .as_ref()
                    .map(|close| close.usage.cached_input_tokens)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.cached_input_tokens)
                    .unwrap_or_default();
            observed.cost_usd = outcome.charged_amount_usd
                + close
                    .as_ref()
                    .map(|close| close.usage.charged_amount_usd)
                    .unwrap_or_default()
                + repair_usage
                    .map(|usage| usage.charged_amount_usd)
                    .unwrap_or_default();
            observed.duration = Some(started.elapsed());
            observed.tool_outcomes = outcome.tool_outcomes.clone();
            observed.hit_cap = outcome.hit_cap;
            observed.wrap_up_injected = outcome.wrap_up_injected;
            observed.resolved_route = outcome.resolved_route.clone();
        }
        Ok(DriverOutcome {
            history,
            output: Some(output),
            partial: None,
            interrupted: outcome.early_exit_tool.is_some() || outcome.hit_cap,
        })
    }
}

fn driver_error(error: impl std::fmt::Display) -> DriverFailure {
    driver_failure(error)
}

fn driver_failure(error: impl std::fmt::Display) -> DriverFailure {
    DriverFailure {
        error: RuntimeError::Driver(error.to_string()),
        partial: None,
    }
}

/// Refuse a provider declaration that has no executable request-scoped source.
/// Sources may contain additional durable tools because the runtime snapshot is
/// the final provider-visible allowlist, but every declared name must resolve
/// here before the graph begins.
fn ensure_snapshot_tools_are_executable(
    visible_tool_names: &HashSet<String>,
    tools: &Arc<Vec<Box<dyn tinytools::Tool>>>,
    synthesized_tools: &Arc<Vec<Box<dyn tinytools::Tool>>>,
) -> Result<(), DriverFailure> {
    let executable_tool_names = tools
        .iter()
        .chain(synthesized_tools.iter())
        .map(|tool| tool.name().to_string())
        .collect::<HashSet<_>>();
    let missing = visible_tool_names
        .difference(&executable_tool_names)
        .cloned()
        .collect::<Vec<_>>();
    if missing.is_empty() {
        Ok(())
    } else {
        Err(driver_failure(format!(
            "session tool snapshot declares non-executable tools: {}",
            missing.join(", ")
        )))
    }
}

fn driver_error_with_snapshot(
    error: impl std::fmt::Display,
    snapshot: &crate::agent::tinyagents::TranscriptSnapshotSink,
    sidecar: &std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    >,
    elapsed: std::time::Duration,
    fallback_model: &str,
) -> DriverFailure {
    let error = error.to_string();
    let guard = snapshot
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    // A graph error drops the harness's `AgentRun`, but its snapshot has
    // already observed provider-accepted usage and completed tool outcomes.
    // Copy those into the per-turn sidecar *before* producing the partial so
    // runtime's single atomic append carries the same usage/failure truth as a
    // successful turn. The partial history below remains accepted-only.
    {
        let mut observed = sidecar
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        observed.model_calls = guard.model_calls as usize;
        observed.tool_calls = guard.tool_outcomes.len();
        observed.input_tokens = guard.input_tokens;
        observed.output_tokens = guard.output_tokens;
        observed.cached_input_tokens = guard.cached_input_tokens;
        observed.cost_usd = if guard.charged_amount_usd > 0.0 {
            guard.charged_amount_usd
        } else {
            let pricing_model = guard
                .resolved_route
                .as_ref()
                .map(|route| route.route.as_str())
                .filter(|route| !route.trim().is_empty())
                .unwrap_or(fallback_model);
            crate::agent::cost::estimate_call_cost_usd(
                pricing_model,
                &crate::inference::provider::UsageInfo {
                    input_tokens: guard.input_tokens,
                    output_tokens: guard.output_tokens,
                    context_window: 0,
                    cached_input_tokens: guard.cached_input_tokens,
                    cache_creation_tokens: 0,
                    reasoning_tokens: 0,
                    charged_amount_usd: 0.0,
                },
            )
        };
        observed.duration = Some(elapsed);
        observed.tool_outcomes = guard.tool_outcomes.clone();
        observed.resolved_route = guard.resolved_route.clone();
    }
    if guard.messages.is_empty() {
        return driver_failure(error);
    }
    let accepted_end = guard.accepted_end();
    let history = guard.messages[..accepted_end].to_vec();
    let unanswered =
        crate::agent::tinyagents::render_unanswered_steps(&guard.messages[accepted_end..]);
    let display = match unanswered {
        Some(steps) => format!("The turn stopped before completion: {error}.\n\n{steps}"),
        None => format!("The turn stopped before completion: {error}."),
    };
    DriverFailure {
        error: RuntimeError::Driver(error),
        partial: Some(DriverOutcome {
            history,
            output: None,
            partial: Some(TranscriptPartial::new(display)),
            interrupted: true,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sidecar() -> std::sync::Arc<
        std::sync::Mutex<crate::agent::tinyagents::host::run_context::SessionTurnSidecar>,
    > {
        crate::agent::tinyagents::host::OpenHumanRunContext::new()
            .session_sidecar
            .clone()
    }

    #[test]
    fn graph_failure_persists_only_accepted_snapshot_history() {
        let snapshot = Arc::new(std::sync::Mutex::new(
            crate::agent::tinyagents::TranscriptSnapshot {
                messages: vec![
                    Message::system("stable prefix"),
                    Message::user("request"),
                    Message::assistant("provider reply not yet accepted by a later request"),
                ],
                accepted_len: 2,
                request_base_len: 2,
                ..Default::default()
            },
        ));

        let failure = driver_error_with_snapshot(
            "provider rejected follow-up",
            &snapshot,
            &sidecar(),
            std::time::Duration::from_millis(1),
            "chat-v1",
        );
        let partial = failure.partial.expect("snapshot should produce a partial");
        assert_eq!(partial.history.len(), 2);
        assert_eq!(partial.history[0].text(), "stable prefix");
        assert_eq!(partial.history[1].text(), "request");
        assert!(
            partial
                .partial
                .expect("display partial")
                .content
                .contains("provider rejected follow-up")
        );
    }

    #[test]
    fn graph_failure_copies_snapshot_usage_and_failed_tool_outcome_to_sidecar() {
        let snapshot = Arc::new(std::sync::Mutex::new(
            crate::agent::tinyagents::TranscriptSnapshot {
                messages: vec![Message::user("request"), Message::tool("call-1", "denied")],
                accepted_len: 1,
                request_base_len: 1,
                input_tokens: 21,
                output_tokens: 8,
                cached_input_tokens: 3,
                charged_amount_usd: 0.004,
                resolved_route: Some(tinyinference_llm::model::ResolvedModelRoute::new(
                    "openhuman",
                    "chat-concrete",
                    "chat-v1",
                )),
                model_calls: 2,
                tool_outcomes: vec![crate::agent::tinyagents::ToolCallOutcome {
                    call_id: "call-1".into(),
                    name: "write_file".into(),
                    arguments: serde_json::json!({"path": "blocked.txt"}),
                    success: false,
                    content: "denied".into(),
                    duration_ms: 17,
                }],
            },
        ));
        let sidecar = sidecar();

        let failure = driver_error_with_snapshot(
            "tool follow-up was rejected",
            &snapshot,
            &sidecar,
            std::time::Duration::from_millis(25),
            "chat-v1",
        );

        let partial = failure.partial.expect("recoverable snapshot partial");
        assert_eq!(partial.history, vec![Message::user("request")]);
        let observed = sidecar.lock().expect("sidecar");
        assert_eq!(observed.model_calls, 2);
        assert_eq!(observed.input_tokens, 21);
        assert_eq!(observed.output_tokens, 8);
        assert_eq!(observed.cached_input_tokens, 3);
        assert!((observed.cost_usd - 0.004).abs() < f64::EPSILON);
        let route = observed
            .resolved_route
            .as_ref()
            .expect("accepted route reaches sidecar");
        assert_eq!(route.provider, "openhuman");
        assert_eq!(route.model, "chat-concrete");
        assert_eq!(route.route, "chat-v1");
        assert_eq!(observed.tool_calls, 1);
        assert_eq!(
            observed.duration,
            Some(std::time::Duration::from_millis(25))
        );
        assert_eq!(observed.tool_outcomes.len(), 1);
        let failure = &observed.tool_outcomes[0];
        assert!(
            !failure.success,
            "completed failure state survives graph error"
        );
        assert_eq!(failure.content, "denied");
        assert_eq!(
            failure.arguments,
            serde_json::json!({"path": "blocked.txt"})
        );
        assert_eq!(failure.duration_ms, 17);
    }

    #[test]
    fn tool_snapshot_with_no_executable_source_fails_closed_before_graph() {
        let visible = HashSet::from(["revoked_tool".to_string()]);
        let error = ensure_snapshot_tools_are_executable(
            &visible,
            &Arc::new(Vec::new()),
            &Arc::new(Vec::new()),
        )
        .expect_err("a declared tool must have a request-scoped executable source");
        assert!(error.error.to_string().contains("revoked_tool"));
    }
}
