//! Phase 7 primary interactive entrypoint.
//!
//! It deliberately runs before `Agent::turn`: direct chat therefore cannot
//! trigger the pre-turn memory, goal, skill, or delegation preparation owned by
//! that legacy path.  TinyAgents remains available as the rollback branch.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use tinyinference::{
    message::Message,
    model::{ModelRequest, ToolChoice},
};

use super::super::types::Agent;
use crate::{
    agent::{
        goose::{GooseCheckpointStore, GooseStopReason, GooseTurnAdapter},
        messages::{ChatMessage, ConversationMessage},
        primary_orchestration::{
            build_capability_catalog, plan_capabilities, primary_checkpoint_store,
            resolve_request_intent, CapabilityBackend, CapabilityCatalog, CapabilityCatalogInputs,
            CapabilityPlannerInput, CapabilityPolicy, CatalogRouteClass, MonetaryBoundary,
            PrimaryTurnMode,
        },
        progress::AgentProgress,
    },
    config::{OrchestrationEngine, SearchEngine},
    tools::traits::PermissionLevel,
};

const DIRECT_CHAT_SYSTEM: &str = concat!(
    include_str!("../../../prompts/IDENTITY.md"),
    "\n\n",
    include_str!("../../../prompts/STYLE.md"),
    "\n\n## Chat mode\nAnswer from the supplied conversation and your model knowledge. ",
    "Do not use or claim to use tools, memory, goals, skills, delegation, external services, ",
    "or current-data retrieval. If live information is required, say that an assisted turn is needed."
);

impl Agent {
    /// Execute a primary interactive turn using the already-resolved session
    /// model.  This is the sole web-chat branch added by the mode gate.
    pub(crate) async fn run_primary_interactive(
        &mut self,
        message: &str,
        mode: PrimaryTurnMode,
        engine: OrchestrationEngine,
        checkpoint_id: &str,
        allow_metered_tools: Option<bool>,
    ) -> Result<String> {
        if engine == OrchestrationEngine::Tinyagents {
            return self.run_single(message).await;
        }

        let history_snapshot = self.begin_guarded_run(message)?;
        let result = match mode {
            PrimaryTurnMode::Chat => self.run_direct_chat(message).await,
            PrimaryTurnMode::Assist | PrimaryTurnMode::Agent => {
                self.run_goose_primary(message, mode, checkpoint_id, allow_metered_tools)
                    .await
            }
        };
        self.finish_guarded_run(&history_snapshot, result)
    }

    async fn resolved_primary_model(
        &self,
    ) -> Result<(crate::agent::tinyagents::TurnModels, Option<u64>)> {
        let context_window = self
            .turn_model_source
            .effective_context_window(&self.model_name)
            .await;
        let models =
            self.turn_model_source
                .build(&self.model_name, self.temperature, context_window)?;
        Ok((models, context_window))
    }

    fn direct_messages(&mut self, user_message: &str) -> Vec<Message> {
        self.absorb_resumed_transcript_prefix();
        let mut messages = vec![Message::system(DIRECT_CHAT_SYSTEM)];
        messages.extend(
            self.tool_dispatcher
                .to_provider_messages(&self.history)
                .into_iter()
                .filter(|message| message.role != "system")
                .map(|message| crate::agent::tinyagents::chat_message_to_message(&message)),
        );
        messages.push(Message::user(user_message.to_string()));
        messages
    }

    async fn run_direct_chat(&mut self, user_message: &str) -> Result<String> {
        let started = std::time::Instant::now();
        let (models, context_window) = self.resolved_primary_model().await?;
        let request = ModelRequest::new(self.direct_messages(user_message))
            .with_model(self.model_name.clone())
            .with_temperature(self.temperature)
            .with_tool_choice(ToolChoice::None);

        self.emit_primary_progress(AgentProgress::TurnStarted).await;
        self.emit_primary_progress(AgentProgress::IterationStarted {
            iteration: 1,
            max_iterations: 1,
        })
        .await;
        let response = models.primary.invoke(&(), request).await?;
        let reply = response.text();
        if reply.trim().is_empty() {
            return Err(anyhow!("The model returned an empty response."));
        }
        let usage = response
            .usage
            .or(response.message.usage)
            .unwrap_or_default();
        tracing::info!(
            mode = "chat",
            routes = "",
            monetary_boundaries = "",
            stop_reason = "final_answer",
            context_window = context_window.unwrap_or(0),
            latest_context_occupancy = usage.input_tokens,
            cumulative_input_tokens = usage.input_tokens,
            cumulative_output_tokens = usage.output_tokens,
            primary_calls = 1,
            advertised_tools = 0,
            "[primary-orchestration] direct chat stopped"
        );

        self.history
            .push(ConversationMessage::Chat(ChatMessage::user(
                user_message.to_string(),
            )));
        self.history
            .push(ConversationMessage::Chat(ChatMessage::assistant(
                reply.clone(),
            )));
        self.finish_primary_mode_turn(
            user_message,
            &reply,
            context_window,
            usage.input_tokens,
            usage.output_tokens,
            usage.cache_read_tokens,
            1,
            started,
            true,
        )
        .await;
        Ok(reply)
    }

    async fn run_goose_primary(
        &mut self,
        user_message: &str,
        mode: PrimaryTurnMode,
        checkpoint_id: &str,
        allow_metered_tools: Option<bool>,
    ) -> Result<String> {
        self.absorb_resumed_transcript_prefix();
        let started = std::time::Instant::now();
        let intent = resolve_request_intent(user_message, mode);

        let durable_tools = self.tools.clone();
        let synthesized_tools = self.synthesized_tools.clone();

        let runtime = self.runtime_config.clone().unwrap_or_default();
        let canonical_search = match runtime.search.effective_engine() {
            SearchEngine::Disabled => None,
            SearchEngine::Managed => Some(CatalogRouteClass {
                backend: CapabilityBackend::Managed,
                monetary_boundary: MonetaryBoundary::ManagedMetered,
            }),
            SearchEngine::Parallel
            | SearchEngine::Brave
            | SearchEngine::Querit
            | SearchEngine::Exa
            | SearchEngine::Tavily => Some(CatalogRouteClass {
                backend: CapabilityBackend::Byok,
                monetary_boundary: MonetaryBoundary::UserSuppliedKey,
            }),
        };

        let catalog_inputs = CapabilityCatalogInputs {
            canonical_search,
            availability: Default::default(),
        };

        let durable_catalog = build_capability_catalog(&durable_tools, &catalog_inputs)?;
        let mut synthesized_catalog =
            build_capability_catalog(&synthesized_tools, &catalog_inputs)?;

        let durable_count = durable_catalog.routes.len();
        for route in &mut synthesized_catalog.routes {
            route.capability.registration_index += durable_count;
        }

        let mut routes = durable_catalog.routes;
        routes.extend(synthesized_catalog.routes);
        let mut diagnostics = durable_catalog.diagnostics;
        diagnostics.extend(synthesized_catalog.diagnostics);
        let catalog = CapabilityCatalog {
            routes,
            diagnostics,
        };

        let managed_allowed =
            runtime.agent.allow_metered_agent_tools && allow_metered_tools.unwrap_or(true);
        let policy = CapabilityPolicy::new(PermissionLevel::Dangerous, managed_allowed, true, true);

        let session_ceiling = if self.visible_tool_names.is_empty() {
            None
        } else {
            Some(&self.visible_tool_names)
        };

        let plan = plan_capabilities(CapabilityPlannerInput::new(
            mode,
            &intent,
            &catalog,
            session_ceiling,
            policy,
        ))?;

        if plan.routes.is_empty() {
            if let Some(reason) = plan.unavailable_reason {
                self.emit_primary_progress(AgentProgress::TurnStarted).await;
                tracing::info!(
                    mode = mode.as_str(),
                    routes = "",
                    monetary_boundaries = "",
                    stop_reason = "unavailable",
                    context_window = 0,
                    latest_context_occupancy = 0,
                    cumulative_input_tokens = 0,
                    cumulative_output_tokens = 0,
                    primary_calls = 0,
                    "[primary-orchestration] Goose turn stopped"
                );
                self.history
                    .push(ConversationMessage::Chat(ChatMessage::user(
                        user_message.to_string(),
                    )));
                self.history
                    .push(ConversationMessage::Chat(ChatMessage::assistant(
                        reason.clone(),
                    )));
                self.finish_primary_mode_turn(
                    user_message,
                    &reason,
                    None,
                    0,
                    0,
                    0,
                    0,
                    started,
                    true,
                )
                .await;
                return Ok(reason);
            }
        }

        let (models, context_window) = self.resolved_primary_model().await?;

        // Gate 3 proves dispatch through Goose while exposing no task tools.
        // Gate 4 supplies the typed ordered capability plan and security-backed
        // tool set.  An empty registry fails closed and cannot execute a tool.
        let mut messages = Vec::with_capacity(self.history.len() + 2);
        messages.push(ConversationMessage::Chat(ChatMessage::system(
            "You are OpenHuman. Complete the user's request directly. If a required capability is unavailable, explain that clearly without claiming it ran.",
        )));
        messages.extend(self.history.iter().filter(|entry| {
            !matches!(entry, ConversationMessage::Chat(chat) if chat.role == "system")
        }).cloned());
        messages.push(ConversationMessage::Chat(ChatMessage::user(
            user_message.to_string(),
        )));

        let store = primary_checkpoint_store(&self.workspace_dir);
        let resume = mode == PrimaryTurnMode::Agent
            && GooseCheckpointStore::load(store.as_ref(), checkpoint_id)
                .await
                .is_ok_and(|checkpoint| checkpoint.is_resumable());
        if !resume {
            let checkpoint = GooseTurnAdapter::checkpoint_from_openhuman(&messages)?;
            store.insert(checkpoint_id, checkpoint)?;
        }
        let store_dyn: Arc<dyn GooseCheckpointStore> = store;

        let policy = Arc::new(crate::security::SecurityPolicy::from_config(
            &runtime.autonomy,
            &runtime.workspace_dir,
            &runtime.action_dir,
        ));
        let security = Arc::new(crate::agent::tinyagents::host::OpenHumanSecurityGate::new(
            policy,
            Vec::new(),
        ));
        let provider_id = models.provider_id().to_string();
        let route_names = plan
            .routes
            .iter()
            .map(|route| route.capability.name.as_str())
            .collect::<Vec<_>>()
            .join(",");
        let monetary_boundaries = plan
            .routes
            .iter()
            .map(|route| match route.capability.monetary_boundary {
                MonetaryBoundary::NonMetered => "non_metered",
                MonetaryBoundary::UserSuppliedKey => "user_supplied_key",
                MonetaryBoundary::ManagedMetered => "managed_metered",
            })
            .collect::<Vec<_>>()
            .join(",");
        let contract =
            crate::agent::primary_orchestration::completion::contract_from_intent(&intent);
        let adapter = GooseTurnAdapter {
            store: store_dyn,
            model: models.primary,
            model_name: self.model_name.clone(),
            provider_id,
            durable_tools,
            synthesized_tools,
            routes: plan.routes,
            security,
            progress: self.on_progress.clone(),
            cancel: tokio_util::sync::CancellationToken::new(),
            max_output_tokens: None,
            max_primary_calls: mode.max_primary_calls(),
            max_no_progress_calls: 2,
            contract: Some(contract),
        };
        let outcome = adapter.run(checkpoint_id).await?;
        tracing::info!(
            mode = mode.as_str(),
            routes = %route_names,
            monetary_boundaries = %monetary_boundaries,
            stop_reason = outcome.stop_reason.as_str(),
            terminal_reason = outcome.checkpoint.terminal_reason.as_deref().unwrap_or(""),
            context_window = context_window.unwrap_or(0),
            latest_context_occupancy = outcome.checkpoint.usage.latest_primary_input_tokens,
            cumulative_input_tokens = outcome.checkpoint.usage.cumulative_input_tokens,
            cumulative_output_tokens = outcome.checkpoint.usage.cumulative_output_tokens,
            primary_calls = outcome.checkpoint.usage.primary_calls,
            "[primary-orchestration] Goose turn stopped"
        );
        let reply = outcome
            .openhuman_messages
            .iter()
            .rev()
            .find_map(|entry| match entry {
                ConversationMessage::Chat(chat)
                    if chat.role == "assistant" && !chat.content.trim().is_empty() =>
                {
                    Some(chat.content.clone())
                }
                _ => None,
            })
            .ok_or_else(|| match outcome.stop_reason {
                GooseStopReason::Cancelled => anyhow!("The turn was cancelled."),
                GooseStopReason::CallCeiling if mode == PrimaryTurnMode::Agent => anyhow!(
                    "The autonomous turn reached its call ceiling and was checkpointed for resume."
                ),
                GooseStopReason::CallCeiling => anyhow!(
                    "The assisted turn reached its call ceiling before producing an answer."
                ),
                GooseStopReason::Yielded => anyhow!("The turn paused before producing an answer."),
                GooseStopReason::FinalAnswer => anyhow!("The model returned an empty response."),
                GooseStopReason::DuplicateSignature => {
                    anyhow!(
                        "The turn was stopped by loop guard: identical call signature repeated."
                    )
                }
                GooseStopReason::RepeatedFailure => {
                    anyhow!("The turn was stopped by loop guard: repeated typed tool failure.")
                }
                GooseStopReason::UnavailableTool => {
                    anyhow!("The turn was stopped by loop guard: requested tool is unavailable.")
                }
                GooseStopReason::NoProgress => {
                    anyhow!(
                        "The turn was stopped by loop guard: no progress across multiple passes."
                    )
                }
                GooseStopReason::Completed => anyhow!("The turn produced no final assistant text."),
            })?;

        self.history
            .push(ConversationMessage::Chat(ChatMessage::user(
                user_message.to_string(),
            )));
        self.history
            .extend(outcome.openhuman_messages.into_iter().skip(messages.len()));
        if !matches!(self.history.last(), Some(ConversationMessage::Chat(chat)) if chat.role == "assistant" && chat.content == reply)
        {
            self.history
                .push(ConversationMessage::Chat(ChatMessage::assistant(
                    reply.clone(),
                )));
        }

        let usage = outcome.checkpoint.usage;
        self.finish_primary_mode_turn(
            user_message,
            &reply,
            context_window,
            usage.cumulative_input_tokens,
            usage.cumulative_output_tokens,
            usage.cumulative_cached_input_tokens,
            usage.primary_calls,
            started,
            false,
        )
        .await;
        Ok(reply)
    }

    #[allow(clippy::too_many_arguments)]
    async fn finish_primary_mode_turn(
        &mut self,
        user_message: &str,
        reply: &str,
        context_window: Option<u64>,
        input_tokens: u64,
        output_tokens: u64,
        cached_input_tokens: u64,
        iterations: u32,
        started: std::time::Instant,
        emit_terminal_progress: bool,
    ) {
        self.trim_history();
        self.last_turn_hit_cap = false;
        self.last_turn_citations.clear();
        self.pending_citations = None;
        self.last_memory_context = None;
        self.last_turn_usage_totals =
            Some(crate::agent::harness::turn_subagent_usage::LastTurnUsage {
                input_tokens,
                output_tokens,
                cached_input_tokens,
                cost_usd: 0.0,
                context_window: context_window.unwrap_or(0),
                context_used_tokens: input_tokens.saturating_add(output_tokens),
                subagents: Vec::new(),
            });

        let persisted = self.tool_dispatcher.to_provider_messages(&self.history);
        let turn_usage = crate::agent::harness::session::transcript::TurnUsage {
            provider: self.event_channel.clone(),
            model: self.model_name.clone(),
            usage: crate::agent::harness::session::transcript::MessageUsage {
                input: input_tokens,
                output: output_tokens,
                cached_input: cached_input_tokens,
                context_window: context_window.unwrap_or(0),
                cost_usd: 0.0,
            },
            ts: chrono::Utc::now().to_rfc3339(),
            reasoning_content: None,
            tool_calls: Vec::new(),
            iteration: iterations,
        };
        self.persist_session_transcript(
            &persisted,
            input_tokens,
            output_tokens,
            cached_input_tokens,
            0.0,
            Some(&turn_usage),
        );

        let capture_content = self
            .runtime_config
            .as_ref()
            .map(|config| config.observability.agent_tracing.capture_content)
            .unwrap_or(false);
        if emit_terminal_progress && capture_content {
            self.emit_primary_progress(AgentProgress::TurnContent {
                input: Some(user_message.to_string()),
                output: Some(reply.to_string()),
            })
            .await;
        }
        if emit_terminal_progress {
            self.emit_primary_progress(AgentProgress::TurnCompleted { iterations })
                .await;
        }

        if !self.post_turn_hooks.is_empty() {
            crate::agent::hooks::fire_hooks(
                &self.post_turn_hooks,
                crate::agent::hooks::TurnContext {
                    user_message: user_message.to_string(),
                    assistant_response: reply.to_string(),
                    tool_calls: Vec::new(),
                    turn_duration_ms: started.elapsed().as_millis() as u64,
                    session_id: Some(self.event_session_id.clone())
                        .filter(|value| !value.trim().is_empty()),
                    agent_id: Some(self.agent_definition_id.clone())
                        .filter(|value| !value.trim().is_empty()),
                    entrypoint: Some(self.event_channel.clone())
                        .filter(|value| !value.trim().is_empty()),
                    iteration_count: iterations as usize,
                },
            );
        }
    }

    async fn emit_primary_progress(&self, event: AgentProgress) {
        if let Some(progress) = &self.on_progress {
            let _ = progress.send(event).await;
        }
    }
}

#[cfg(test)]
#[path = "primary_turn_tests.rs"]
mod primary_turn_tests;
