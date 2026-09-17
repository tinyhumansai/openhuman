use std::{collections::HashSet, sync::Arc};

use anyhow::Result;
use async_trait::async_trait;
use goose_agent::{
    machine::{StateMachine, Step},
    operation::{messages_since_kickoff, not_applicable, Emitter, Operation, OperationResult},
    tool::ToolOperation,
};
use goose_provider_types::conversation::{
    message::{Message, MessageContent},
    Conversation,
};
use rmcp::model::{CallToolResult, ContentBlock};
use tokio_util::sync::CancellationToken;

use crate::agent::{
    messages::ConversationMessage,
    primary_orchestration::{
        capability::{CapabilityPlan, ToolRoute},
        completion::{
            evaluate_completion, CompletionContract, CompletionObservation, GeneratedArtifact,
            RepositoryChangeRecord, ScheduleRecord, ValidatedImage, VerificationStatus,
        },
    },
    progress::AgentProgress,
};

use super::{
    convert::{
        ensure_nonempty_kickoff, final_text, goose_to_openhuman, openhuman_to_goose,
        rmcp_result_text,
    },
    inference::OpenHumanInference,
    store::{rebuild_action_index, CheckpointRuntime},
    tools::{GooseToolRegistry, GooseToolSecurity, OpenHumanToolProvider},
    types::{GooseCheckpoint, GooseSession, GooseStopReason, GooseTurnOutcome, OpenHumanEffect},
    GooseCheckpointStore,
};

fn is_repeat_call_exempt(tool: &str) -> bool {
    matches!(tool, "wait_subagent")
}

struct CancellationObservation;

#[async_trait]
impl Operation<GooseSession, OpenHumanEffect> for CancellationObservation {
    fn name(&self) -> &'static str {
        "openhuman_cancellation"
    }

    async fn run(
        &self,
        _session: &GooseSession,
        _conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<OpenHumanEffect>> {
        not_applicable()
    }

    async fn cancel(
        &self,
        _session: &GooseSession,
        conversation: &Conversation,
        result: OperationResult<OpenHumanEffect>,
        emit: &Emitter,
    ) -> Result<OperationResult<OpenHumanEffect>> {
        if !matches!(result, OperationResult::NotApplicable) {
            return Ok(result);
        }
        let turn = messages_since_kickoff(conversation)?;
        let answered: HashSet<&str> = turn
            .iter()
            .flat_map(Message::get_tool_response_ids)
            .collect();
        let pending = turn
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(MessageContent::as_tool_request)
            .filter(|request| !answered.contains(request.id.as_str()))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return goose_agent::operation::yielded();
        }
        let mut response = Message::user();
        for request in pending {
            response = response.with_tool_response(
                request.id.clone(),
                Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Tool call was cancelled before execution",
                )])),
            );
        }
        let response = emit.message(response).await;
        goose_agent::operation::yielded_with([OpenHumanEffect::from(response)])
    }
}

struct UnavailableRequestGuard {
    enabled_names: HashSet<String>,
}

#[async_trait]
impl Operation<GooseSession, OpenHumanEffect> for UnavailableRequestGuard {
    fn name(&self) -> &'static str {
        "openhuman_unavailable_request_guard"
    }

    async fn run(
        &self,
        session: &GooseSession,
        conversation: &Conversation,
        emit: &Emitter,
    ) -> Result<OperationResult<OpenHumanEffect>> {
        let turn = messages_since_kickoff(conversation)?;
        let answered: HashSet<&str> = turn
            .iter()
            .flat_map(Message::get_tool_response_ids)
            .collect();
        let pending = turn
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(MessageContent::as_tool_request)
            .filter(|request| !answered.contains(request.id.as_str()))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return not_applicable();
        }

        let unavailable = pending.iter().find(|request| {
            request
                .tool_call
                .as_ref()
                .map(|call| {
                    !self.enabled_names.contains(call.name.as_ref())
                        || session
                            .checkpoint
                            .unavailable_routes
                            .contains(&call.name.to_string())
                })
                .unwrap_or(false)
        });

        let Some(unavail) = unavailable else {
            return not_applicable();
        };

        let tool_name = unavail
            .tool_call
            .as_ref()
            .map(|call| call.name.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let explanation = format!(
            "Turn stopped by loop guard: model requested tool '{tool_name}' which is not in active routes."
        );
        let assistant_msg = emit
            .message(Message::assistant().with_text(explanation))
            .await;

        goose_agent::operation::yielded_with(vec![
            OpenHumanEffect::from(assistant_msg),
            OpenHumanEffect::SetTerminalReason(Some("unavailable_tool".into())),
        ])
    }
}

struct DuplicateSignatureGuard;

#[async_trait]
impl Operation<GooseSession, OpenHumanEffect> for DuplicateSignatureGuard {
    fn name(&self) -> &'static str {
        "openhuman_duplicate_signature_guard"
    }

    async fn run(
        &self,
        session: &GooseSession,
        conversation: &Conversation,
        emit: &Emitter,
    ) -> Result<OperationResult<OpenHumanEffect>> {
        let turn = messages_since_kickoff(conversation)?;
        let answered: HashSet<&str> = turn
            .iter()
            .flat_map(Message::get_tool_response_ids)
            .collect();
        let pending = turn
            .iter()
            .flat_map(|message| message.content.iter())
            .filter_map(MessageContent::as_tool_request)
            .filter(|request| !answered.contains(request.id.as_str()))
            .collect::<Vec<_>>();
        if pending.is_empty() {
            return not_applicable();
        }

        let duplicate = pending.iter().find(|request| {
            let Ok(call) = request.tool_call.as_ref() else {
                return false;
            };
            if is_repeat_call_exempt(&call.name) {
                return false;
            }
            let args = serde_json::Value::Object(call.arguments.clone().unwrap_or_default());
            let sig = format!("{}:{}", call.name, args);
            session.checkpoint.last_call_signature.as_deref() == Some(&sig)
        });

        let Some(dup) = duplicate else {
            return not_applicable();
        };

        let tool_name = dup
            .tool_call
            .as_ref()
            .map(|call| call.name.to_string())
            .unwrap_or_else(|_| "unknown".to_string());

        let mut response = Message::user();
        for request in pending {
            response = response.with_tool_response(
                request.id.clone(),
                Ok(CallToolResult::error(vec![ContentBlock::text(
                    "Tool call rejected: duplicate call signature repeated consecutively",
                )])),
            );
        }
        let explanation = format!(
            "Turn stopped by loop guard: duplicate call signature '{tool_name}' was repeated consecutively."
        );
        let response_msg = emit.message(response).await;
        let assistant_msg = emit
            .message(Message::assistant().with_text(explanation))
            .await;

        goose_agent::operation::yielded_with(vec![
            OpenHumanEffect::from(response_msg),
            OpenHumanEffect::from(assistant_msg),
            OpenHumanEffect::SetTerminalReason(Some("duplicate_signature".into())),
        ])
    }
}

struct ObservationLoopGuard {
    routes: Vec<ToolRoute>,
    max_no_progress_calls: u32,
}

#[async_trait]
impl Operation<GooseSession, OpenHumanEffect> for ObservationLoopGuard {
    fn name(&self) -> &'static str {
        "openhuman_observation_loop_guard"
    }

    async fn run(
        &self,
        session: &GooseSession,
        conversation: &Conversation,
        emit: &Emitter,
    ) -> Result<OperationResult<OpenHumanEffect>> {
        let Some(last_msg) = conversation.last() else {
            return not_applicable();
        };
        if last_msg.role != rmcp::model::Role::User || !last_msg.is_tool_response() {
            return not_applicable();
        }

        let responses = last_msg
            .content
            .iter()
            .filter_map(MessageContent::as_tool_response)
            .collect::<Vec<_>>();
        let Some(last_resp) = responses.last() else {
            return not_applicable();
        };

        let Some(action) = session.checkpoint.actions.get(&last_resp.id) else {
            return not_applicable();
        };

        let current_sig = format!("{}:{}", action.tool_name, action.arguments);
        if session.checkpoint.last_call_signature.as_deref() == Some(&current_sig) {
            return not_applicable();
        }

        let (success, output) = match &last_resp.tool_result {
            Ok(result) => (!result.is_error.unwrap_or(false), rmcp_result_text(result)),
            Err(error) => (false, error.message.to_string()),
        };

        if !success {
            let error_line = output.lines().next().unwrap_or("").trim();
            let failure_type = format!("{}:{}", action.tool_name, error_line);
            let is_terminal = super::tools::is_terminal_route_failure(&output);
            if session.checkpoint.last_failure_type.as_deref() == Some(&failure_type) {
                let explanation = format!(
                    "Turn stopped by loop guard: tool '{}' failed repeatedly with error: {}",
                    action.tool_name, output
                );
                let assistant_msg = emit
                    .message(Message::assistant().with_text(explanation))
                    .await;
                let mut effects = vec![
                    OpenHumanEffect::from(assistant_msg),
                    OpenHumanEffect::SetLastCallSignature(Some(current_sig)),
                    OpenHumanEffect::RecordFailure(failure_type),
                    OpenHumanEffect::SetTerminalReason(Some("repeated_failure".into())),
                ];
                if is_terminal {
                    effects.push(OpenHumanEffect::MarkRouteUnavailable(
                        action.tool_name.clone(),
                    ));
                }
                return goose_agent::operation::yielded_with(effects);
            }
            let mut effects = vec![
                OpenHumanEffect::SetLastCallSignature(Some(current_sig)),
                OpenHumanEffect::RecordFailure(failure_type),
            ];
            if is_terminal {
                effects.push(OpenHumanEffect::MarkRouteUnavailable(
                    action.tool_name.clone(),
                ));
            }
            return goose_agent::operation::applied(effects);
        }

        let is_mutation = self.routes.iter().any(|r| {
            r.capability.name == action.tool_name
                && matches!(
                    r.capability.side_effect,
                    crate::agent::primary_orchestration::capability::CapabilitySideEffect::LocalWrite
                        | crate::agent::primary_orchestration::capability::CapabilitySideEffect::ExternalWrite
                )
        });

        if !is_mutation {
            let next_count = session.checkpoint.no_progress_count.saturating_add(1);
            if next_count >= self.max_no_progress_calls {
                let explanation = format!(
                    "Turn stopped by loop guard: reached {next_count} consecutive informational tool calls without progress."
                );
                let assistant_msg = emit
                    .message(Message::assistant().with_text(explanation))
                    .await;
                return goose_agent::operation::yielded_with(vec![
                    OpenHumanEffect::from(assistant_msg),
                    OpenHumanEffect::SetLastCallSignature(Some(current_sig)),
                    OpenHumanEffect::ResetFailure,
                    OpenHumanEffect::IncrementNoProgress,
                    OpenHumanEffect::SetTerminalReason(Some("no_progress".into())),
                ]);
            }
            return goose_agent::operation::applied(vec![
                OpenHumanEffect::SetLastCallSignature(Some(current_sig)),
                OpenHumanEffect::ResetFailure,
                OpenHumanEffect::IncrementNoProgress,
            ]);
        }

        goose_agent::operation::applied(vec![
            OpenHumanEffect::SetLastCallSignature(Some(current_sig)),
            OpenHumanEffect::ResetFailure,
            OpenHumanEffect::ResetNoProgress,
        ])
    }
}

struct PrimaryCallCeiling {
    max_primary_calls: u32,
}

#[async_trait]
impl Operation<GooseSession, OpenHumanEffect> for PrimaryCallCeiling {
    fn name(&self) -> &'static str {
        "openhuman_primary_call_ceiling"
    }

    async fn run(
        &self,
        session: &GooseSession,
        _conversation: &Conversation,
        _emit: &Emitter,
    ) -> Result<OperationResult<OpenHumanEffect>> {
        if session.checkpoint.usage.primary_calls >= self.max_primary_calls {
            return goose_agent::operation::yielded();
        }
        not_applicable()
    }
}

/// Direct adapter around the vendored `goose_agent::machine::StateMachine`.
/// It enforces loop guards and completion contracts for local-Qwen primary turns.
pub struct GooseTurnAdapter {
    pub store: Arc<dyn GooseCheckpointStore>,
    pub model: Arc<dyn tinyinference::model::ChatModel<()>>,
    pub model_name: String,
    pub provider_id: String,
    pub durable_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
    pub synthesized_tools: Arc<Vec<Box<dyn crate::tools::Tool>>>,
    pub routes: Vec<ToolRoute>,
    pub security: Arc<dyn GooseToolSecurity>,
    pub progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
    pub cancel: CancellationToken,
    pub max_output_tokens: Option<u32>,
    pub max_primary_calls: u32,
    pub max_no_progress_calls: u32,
    pub contract: Option<CompletionContract>,
}

impl GooseTurnAdapter {
    pub fn checkpoint_from_openhuman(messages: &[ConversationMessage]) -> Result<GooseCheckpoint> {
        let conversation = openhuman_to_goose(messages)?;
        ensure_nonempty_kickoff(&conversation)?;
        let mut checkpoint = GooseCheckpoint::new(conversation);
        rebuild_action_index(&mut checkpoint)?;
        Ok(checkpoint)
    }

    async fn progress(&self, event: AgentProgress) {
        if let Some(progress) = &self.progress {
            let _ = progress.send(event).await;
        }
    }

    pub async fn run(&self, session_id: &str) -> Result<GooseTurnOutcome> {
        self.progress(AgentProgress::TurnStarted).await;

        let provider = Arc::new(OpenHumanToolProvider {
            registry: GooseToolRegistry::new(
                self.durable_tools.clone(),
                self.synthesized_tools.clone(),
                self.routes.clone(),
            ),
            security: self.security.clone(),
            store: self.store.clone(),
            progress: self.progress.clone(),
        });
        let tools = ToolOperation::<GooseSession>::new().with_provider(provider);
        let inference = OpenHumanInference {
            model: self.model.clone(),
            model_name: self.model_name.clone(),
            provider_id: self.provider_id.clone(),
            max_output_tokens: self.max_output_tokens,
            progress: self.progress.clone(),
        };
        let enabled_names = CapabilityPlan::derive_enabled_names(&self.routes);
        let machine = StateMachine::new(
            vec![
                Step::Operation(Arc::new(CancellationObservation)),
                Step::Operation(Arc::new(UnavailableRequestGuard {
                    enabled_names: enabled_names.clone(),
                })),
                Step::Operation(Arc::new(DuplicateSignatureGuard)),
                Step::Operation(Arc::new(tools)),
                Step::Operation(Arc::new(ObservationLoopGuard {
                    routes: self.routes.clone(),
                    max_no_progress_calls: self.max_no_progress_calls,
                })),
                Step::Operation(Arc::new(PrimaryCallCeiling {
                    max_primary_calls: self.max_primary_calls,
                })),
                Step::Inference(Arc::new(inference)),
            ],
            self.cancel.clone(),
        );
        let runtime = CheckpointRuntime {
            store: self.store.clone(),
        };
        let (events_tx, mut events_rx) = tokio::sync::mpsc::channel(64);
        let emit = Emitter::new(events_tx, self.cancel.clone());
        let drain = tokio::spawn(async move { while events_rx.recv().await.is_some() {} });
        let mut session = machine.run(&runtime, session_id, &emit).await?;
        drop(emit);
        let _ = drain.await;

        let final_answer = final_text(&session.checkpoint.conversation);

        fn extract_completion_observation(
            final_answer: Option<String>,
            actions: &std::collections::BTreeMap<String, super::types::AcceptedToolAction>,
        ) -> CompletionObservation {
            let mut informational_tools_executed = Vec::new();
            let mut web_sources = Vec::new();
            let mut validated_images = Vec::new();
            let mut generated_artifacts = Vec::new();
            let mut repository_changes = Vec::new();
            let mut scheduling_records = Vec::new();
            let yielded_question = None;
            let yielded_approval = None;

            for action in actions.values() {
                if let Some(obs) = &action.observation {
                    if obs.success {
                        informational_tools_executed.push(action.tool_name.clone());

                        if let Ok(v) = serde_json::from_str::<serde_json::Value>(&obs.output) {
                            if let Some(url) = v.get("url").and_then(|u| u.as_str()) {
                                let source = v
                                    .get("source")
                                    .or_else(|| v.get("source_origin"))
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("")
                                    .to_string();
                                let mime = v
                                    .get("mime_type")
                                    .and_then(|m| m.as_str())
                                    .map(ToString::to_string);
                                if action.tool_name.contains("image")
                                    || url.ends_with(".png")
                                    || url.ends_with(".jpg")
                                    || url.ends_with(".jpeg")
                                    || url.ends_with(".webp")
                                    || url.ends_with(".svg")
                                {
                                    validated_images.push(ValidatedImage {
                                        url_or_path: url.to_string(),
                                        source_origin: source.clone(),
                                        mime_type: mime,
                                    });
                                }
                            }
                            if let Some(sources) = v.get("sources").and_then(|s| s.as_array()) {
                                for s in sources {
                                    if let Some(url_str) = s.as_str() {
                                        web_sources.push(url_str.to_string());
                                    }
                                }
                            }
                            if let Some(path) = v.get("path").and_then(|p| p.as_str()) {
                                let desc = v
                                    .get("description")
                                    .and_then(|d| d.as_str())
                                    .map(ToString::to_string);
                                generated_artifacts.push(GeneratedArtifact {
                                    path_or_url: path.to_string(),
                                    description: desc,
                                });
                            }
                            if let Some(sched_id) = v.get("schedule_id").and_then(|s| s.as_str()) {
                                let state = v
                                    .get("state")
                                    .and_then(|s| s.as_str())
                                    .unwrap_or("scheduled")
                                    .to_string();
                                scheduling_records.push(ScheduleRecord {
                                    schedule_id: sched_id.to_string(),
                                    state,
                                });
                            }
                            if let Some(files) = v.get("file_paths").and_then(|f| f.as_array()) {
                                let file_paths: Vec<String> = files
                                    .iter()
                                    .filter_map(|s| s.as_str().map(ToString::to_string))
                                    .collect();
                                let ver_str = v
                                    .get("verification")
                                    .and_then(|ver| ver.as_str())
                                    .unwrap_or("unverified");
                                let verification = match ver_str {
                                    "verified" => VerificationStatus::Verified,
                                    "failed" => VerificationStatus::Failed,
                                    _ => VerificationStatus::Unverified,
                                };
                                let summary = v
                                    .get("summary")
                                    .and_then(|s| s.as_str())
                                    .map(ToString::to_string);
                                repository_changes.push(RepositoryChangeRecord {
                                    file_paths,
                                    verification,
                                    summary,
                                });
                            }
                        }

                        if action.tool_name.contains("image") && validated_images.is_empty() {
                            if let Some(start) = obs
                                .output
                                .find("http://")
                                .or_else(|| obs.output.find("https://"))
                            {
                                let slice = &obs.output[start..];
                                let end = slice
                                    .find(|c: char| {
                                        c.is_whitespace()
                                            || c == '"'
                                            || c == '\''
                                            || c == ')'
                                            || c == ']'
                                    })
                                    .unwrap_or(slice.len());
                                let url = &slice[..end];
                                validated_images.push(ValidatedImage {
                                    url_or_path: url.to_string(),
                                    source_origin: action.tool_name.clone(),
                                    mime_type: None,
                                });
                            }
                        }
                    }
                }
            }

            if validated_images.is_empty() {
                if let Some(text) = &final_answer {
                    if let Some(img_idx) = text.find("![") {
                        if let Some(paren_open) = text[img_idx..].find('(') {
                            let from_paren = &text[img_idx + paren_open + 1..];
                            if let Some(paren_close) = from_paren.find(')') {
                                let url = from_paren[..paren_close].trim();
                                if !url.is_empty() {
                                    validated_images.push(ValidatedImage {
                                        url_or_path: url.to_string(),
                                        source_origin: "final_text".into(),
                                        mime_type: None,
                                    });
                                }
                            }
                        }
                    }
                }
            }

            CompletionObservation {
                final_assistant_text: final_answer,
                informational_tools_executed,
                web_sources,
                validated_images,
                generated_artifacts,
                repository_changes,
                scheduling_records,
                yielded_question,
                yielded_approval,
            }
        }

        if let Some(contract) = &self.contract {
            let obs =
                extract_completion_observation(final_answer.clone(), &session.checkpoint.actions);
            let status = evaluate_completion(contract, &obs);
            session.checkpoint.completion_state = Some(status.clone());
            if status.is_complete() {
                session.checkpoint.terminal_reason = Some("completed".into());
            }
            let expected = session.checkpoint.revision;
            session.checkpoint.revision = expected.saturating_add(1);
            let _ = self
                .store
                .compare_and_swap(session_id, expected, session.checkpoint.clone())
                .await;
        }

        let stop_reason = if self.cancel.is_cancelled() {
            GooseStopReason::Cancelled
        } else if let Some(terminal) = session.checkpoint.terminal_reason.as_deref() {
            match terminal {
                "duplicate_signature" => GooseStopReason::DuplicateSignature,
                "repeated_failure" => GooseStopReason::RepeatedFailure,
                "unavailable_tool" => GooseStopReason::UnavailableTool,
                "no_progress" => GooseStopReason::NoProgress,
                "completed" => GooseStopReason::Completed,
                _ => GooseStopReason::Yielded,
            }
        } else if final_answer.is_some() {
            GooseStopReason::FinalAnswer
        } else if session.checkpoint.usage.primary_calls >= self.max_primary_calls {
            GooseStopReason::CallCeiling
        } else {
            GooseStopReason::Yielded
        };
        if let Some(answer) = final_answer {
            self.progress(AgentProgress::TurnContent {
                input: None,
                output: Some(answer),
            })
            .await;
            self.progress(AgentProgress::TurnCompleted {
                iterations: session.checkpoint.usage.primary_calls,
            })
            .await;
        }
        Ok(GooseTurnOutcome {
            openhuman_messages: goose_to_openhuman(&session.checkpoint.conversation),
            checkpoint: session.checkpoint,
            stop_reason,
        })
    }
}
