
impl Agent {
    /// Borrow the holistic token/cost/context totals for the latest completed
    /// turn (parent + sub-agents) **without consuming them**. `None` until a
    /// turn has run.
    ///
    /// This is the public, non-draining counterpart to
    /// [`take_last_turn_usage_totals`](Self::take_last_turn_usage_totals): a
    /// downstream crate embedding OpenHuman as a library (e.g. the OpenCompany
    /// hosting platform's cost-metering hook) can read per-turn token and USD
    /// totals after [`Agent::turn`](crate::openhuman::agent::Agent) returns,
    /// while leaving the value in place for the web-channel drain path.
    pub fn last_turn_usage(
        &self,
    ) -> Option<&crate::openhuman::agent::harness::turn_subagent_usage::LastTurnUsage> {
        self.last_turn_usage_totals.as_ref()
    }

    /// Drain and return the holistic token/cost/context totals for the latest
    /// completed turn (parent + sub-agents). `None` until a turn has run.
    /// Consumed by web-channel delivery to populate the `chat_done` usage fields.
    pub(crate) fn take_last_turn_usage_totals(
        &mut self,
    ) -> Option<crate::openhuman::agent::harness::turn_subagent_usage::LastTurnUsage> {
        self.last_turn_usage_totals.take()
    }

    /// Whether the most recently completed [`Self::turn`] / [`Self::run_single`]
    /// paused because it hit `max_tool_iterations`, rather than finishing
    /// naturally (see the field doc on `last_turn_hit_cap`). `false` before
    /// any turn has run. Not draining — unlike the usage totals above, a
    /// caller may reasonably check this more than once per turn.
    pub fn last_turn_hit_cap(&self) -> bool {
        self.last_turn_hit_cap
    }

    // ─────────────────────────────────────────────────────────────────
    // Static helpers for turn parsing + telemetry
    // ─────────────────────────────────────────────────────────────────

    pub(super) fn count_iterations(messages: &[ConversationMessage]) -> usize {
        messages
            .iter()
            .filter(|message| matches!(message, ConversationMessage::AssistantToolCalls { .. }))
            .count()
            + 1
    }

    fn conversation_message_eq(left: &ConversationMessage, right: &ConversationMessage) -> bool {
        serde_json::to_string(left).ok() == serde_json::to_string(right).ok()
    }

    fn message_slice_eq(left: &[ConversationMessage], right: &[ConversationMessage]) -> bool {
        left.len() == right.len()
            && left
                .iter()
                .zip(right.iter())
                .all(|(left, right)| Self::conversation_message_eq(left, right))
    }

    pub(super) fn new_entries_for_turn<'a>(
        history_snapshot: &[ConversationMessage],
        current_history: &'a [ConversationMessage],
    ) -> &'a [ConversationMessage] {
        let common_prefix_len = history_snapshot
            .iter()
            .zip(current_history.iter())
            .take_while(|(left, right)| Self::conversation_message_eq(left, right))
            .count();

        if common_prefix_len == history_snapshot.len() {
            return &current_history[common_prefix_len..];
        }

        let max_overlap = history_snapshot.len().min(current_history.len());
        for overlap in (0..=max_overlap).rev() {
            let snapshot_suffix = &history_snapshot[history_snapshot.len() - overlap..];
            let current_prefix = &current_history[..overlap];
            if Self::message_slice_eq(snapshot_suffix, current_prefix) {
                return &current_history[overlap..];
            }
        }

        current_history
    }

    pub(super) fn sanitize_event_error_message(err: &anyhow::Error) -> String {
        let kind = match err.downcast_ref::<AgentError>() {
            Some(AgentError::ProviderError { .. }) => Some("provider_error"),
            Some(AgentError::ContextLimitExceeded { .. }) => Some("context_limit_exceeded"),
            Some(AgentError::ToolExecutionError { .. }) => Some("tool_execution_error"),
            Some(AgentError::CostBudgetExceeded { .. }) => Some("cost_budget_exceeded"),
            Some(AgentError::MaxIterationsExceeded { .. }) => Some("max_iterations_exceeded"),
            Some(AgentError::EmptyProviderResponse { .. }) => Some("empty_provider_response"),
            Some(AgentError::CompactionFailed { .. }) => Some("compaction_failed"),
            Some(AgentError::PermissionDenied { .. }) => Some("permission_denied"),
            Some(AgentError::RegistryValidationFailed { .. }) => Some("registry_validation_failed"),
            Some(AgentError::Other(_)) | None => None,
        };

        if let Some(kind) = kind {
            return kind.to_string();
        }

        let scrubbed = provider::sanitize_api_error(&err.to_string())
            .replace(['\n', '\r', '\t'], " ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        truncate_with_ellipsis(&scrubbed, Self::EVENT_ERROR_MAX_CHARS)
    }

    /// Injects unique IDs into tool calls that are missing them.
    ///
    /// This is necessary for some tool dispatchers to correctly track and
    /// associate results.
    pub(super) fn with_fallback_tool_call_ids(
        mut parsed_calls: Vec<ParsedToolCall>,
        iteration: usize,
    ) -> Vec<ParsedToolCall> {
        for (idx, call) in parsed_calls.iter_mut().enumerate() {
            if call.tool_call_id.is_none() {
                call.tool_call_id = Some(format!("parsed-{}-{}", iteration + 1, idx + 1));
            }
        }
        parsed_calls
    }

    /// Converts parsed tool calls into the provider-standard `ToolCall` format.
    ///
    /// If the provider response already contains native tool calls, they are
    /// returned as-is.
    pub(super) fn persisted_tool_calls_for_history(
        response: &crate::openhuman::inference::provider::ChatResponse,
        parsed_calls: &[ParsedToolCall],
        iteration: usize,
    ) -> Vec<ToolCall> {
        if !response.tool_calls.is_empty() {
            return response.tool_calls.clone();
        }

        parsed_calls
            .iter()
            .enumerate()
            .map(|(idx, call)| ToolCall {
                id: call
                    .tool_call_id
                    .clone()
                    .unwrap_or_else(|| format!("parsed-{}-{}", iteration + 1, idx + 1)),
                name: call.name.clone(),
                arguments: call.arguments.to_string(),
                // Prompt-based tool calls carry no provider extra_content.
                extra_content: None,
            })
            .collect()
    }

    // ─────────────────────────────────────────────────────────────────
    // Run helpers — single-shot and interactive loops
    // ─────────────────────────────────────────────────────────────────

    /// Runs a single turn with the given message and returns the response.
    ///
    /// This is the primary high-level method for programmatic interaction with the agent.
    /// It wraps the core `turn` logic with telemetry events (`AgentTurnStarted`,
    /// `AgentTurnCompleted`) and error sanitization.
    pub async fn run_single(&mut self, message: &str) -> Result<String> {
        let guard = enforce_prompt_input(
            message,
            PromptEnforcementContext {
                source: "agent.runtime.run_single",
                request_id: None,
                user_id: Some(self.event_channel()),
                session_id: Some(self.event_session_id()),
            },
        );
        if !matches!(guard.action, PromptEnforcementAction::Allow) {
            let user_message = match guard.action {
                PromptEnforcementAction::Allow => "Message accepted.",
                PromptEnforcementAction::Blocked => "Prompt blocked by security policy.",
                PromptEnforcementAction::ReviewBlocked => {
                    "Prompt flagged for security review and was not processed."
                }
            };
            let action_tag = match guard.action {
                PromptEnforcementAction::Allow => "allow",
                PromptEnforcementAction::Blocked => "blocked",
                PromptEnforcementAction::ReviewBlocked => "review_blocked",
            };
            crate::core::observability::report_error(
                user_message,
                "agent",
                "prompt_injection_blocked",
                &[
                    ("session_id", self.event_session_id()),
                    ("channel", self.event_channel()),
                    ("action", action_tag),
                ],
            );
            BUS.publish(DomainEvent::AgentError {
                session_id: self.event_session_id().to_string(),
                message: user_message.to_string(),
                recoverable: true,
            });
            return Err(anyhow::anyhow!(user_message));
        }

        let history_snapshot = self.history.clone();
        BUS.publish(DomainEvent::AgentTurnStarted {
            session_id: self.event_session_id().to_string(),
            channel: self.event_channel().to_string(),
        });

        match self.turn(message).await {
            Ok(response) => {
                let new_entries = Self::new_entries_for_turn(&history_snapshot, &self.history);
                BUS.publish(DomainEvent::AgentTurnCompleted {
                    session_id: self.event_session_id().to_string(),
                    text_chars: response.chars().count(),
                    iterations: Self::count_iterations(new_entries),
                });
                Ok(response)
            }
            Err(err) => {
                let sanitized_message = Self::sanitize_event_error_message(&err);
                // Some typed `AgentError` variants represent agent / user /
                // provider state that the UI already surfaces — the
                // max-tool-iterations cap (OPENHUMAN-TAURI-99 / -98,
                // chat-rendered "Error: Agent exceeded maximum tool
                // iterations") and the empty-provider-response degeneracy
                // (TAURI-RUST-4JX, "The model returned an empty response.
                // Please try again."). Skip the Sentry funnel for both
                // and emit a structured `log::info!` instead. The
                // suppressed set is owned by `AgentError::skips_sentry()`
                // so the policy stays in one place.
                //
                // Other agent errors go through `report_error_or_expected`
                // so OPENHUMAN-TAURI-5Z and the budget-noise cluster —
                // upstream transient HTTP and backend budget-exhausted 400s
                // that bubble up under `domain=agent` and escape the
                // `domain=llm_provider` filter — get demoted to a
                // warn/info-level breadcrumb without losing genuine bugs.
                // `Err` propagation, the `AgentError` domain event, and
                // downstream `recoverable=false` semantics are preserved.
                let skips_sentry = err
                    .downcast_ref::<AgentError>()
                    .is_some_and(AgentError::skips_sentry);
                if skips_sentry {
                    log::info!(
                        target: "agent",
                        "[agent.run_single] suppressed Sentry emission for user-state agent error \
                         session_id={} channel={} error_kind={} message={}",
                        self.event_session_id(),
                        self.event_channel(),
                        sanitized_message.as_str(),
                        err
                    );
                } else {
                    crate::core::observability::report_error_or_expected(
                        &err,
                        "agent",
                        "run_single",
                        &[
                            ("session_id", self.event_session_id()),
                            ("channel", self.event_channel()),
                            ("error_kind", sanitized_message.as_str()),
                        ],
                    );
                }
                BUS.publish(DomainEvent::AgentError {
                    session_id: self.event_session_id().to_string(),
                    message: sanitized_message,
                    recoverable: false,
                });
                Err(err)
            }
        }
    }

    /// Runs an interactive CLI loop, reading from standard input and printing to standard output.
    ///
    /// This method starts a persistent session where the user can chat with the agent
    /// directly from the console. It handles input until a termination command
    /// (e.g., `/quit`) is received.
    pub async fn run_interactive(&mut self) -> Result<()> {
        println!("🦀 OpenHuman Interactive Mode");
        println!("Type /quit to exit.\n");

        let (tx, mut rx) = tokio::sync::mpsc::channel(32);
        let cli = crate::openhuman::channels::CliChannel::new();

        let listen_handle = tokio::spawn(async move {
            let _ = crate::openhuman::channels::Channel::listen(&cli, tx).await;
        });

        while let Some(msg) = rx.recv().await {
            match self.run_single(&msg.content).await {
                Ok(response) => println!("\n{response}\n"),
                Err(e) => {
                    // `run_single` already publishes `AgentError` and
                    // sanitises the payload; surface a concise line here
                    // for the CLI user and continue the loop.
                    eprintln!("\nError: {e}\n");
                    continue;
                }
            }
        }

        listen_handle.abort();
        Ok(())
    }
}
