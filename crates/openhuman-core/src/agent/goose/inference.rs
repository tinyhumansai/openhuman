use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use goose_agent::operation::{
    applied, not_applicable, Emitter, Inference, InferenceInput, Operation, OperationResult,
};
use goose_provider_types::conversation::{
    effective_role,
    message::{Message, MessageMetadata},
    token_usage::ProviderUsage,
    Conversation, EffectiveRole,
};
use tinyinference::model::{ChatModel, ModelRequest};

use crate::agent::progress::AgentProgress;

use super::{
    convert::{goose_to_model_messages, rmcp_tools_to_tiny, tiny_response_to_goose},
    qwen::{
        build_protocol_correction_prompt, is_local_qwen_route, normalize_qwen_response,
        protocol_failure_terminal_answer,
    },
    types::{GooseSession, OpenHumanEffect},
};

pub(super) struct OpenHumanInference {
    pub model: Arc<dyn ChatModel<()>>,
    pub model_name: String,
    pub provider_id: String,
    pub max_output_tokens: Option<u32>,
    pub progress: Option<tokio::sync::mpsc::Sender<AgentProgress>>,
}

impl OpenHumanInference {
    async fn progress(&self, event: AgentProgress) {
        if let Some(progress) = &self.progress {
            let _ = progress.send(event).await;
        }
    }
}

#[async_trait]
impl Operation<GooseSession, OpenHumanEffect> for OpenHumanInference {
    fn name(&self) -> &'static str {
        "openhuman_inference"
    }
}

#[async_trait]
impl Inference<GooseSession, OpenHumanEffect> for OpenHumanInference {
    fn applies(&self, conversation: &Conversation) -> bool {
        conversation.last().is_some_and(|message| {
            matches!(
                effective_role(message),
                EffectiveRole::User | EffectiveRole::Tool
            )
        })
    }

    async fn infer(
        &self,
        session: &GooseSession,
        conversation: &Conversation,
        input: InferenceInput,
        emit: &Emitter,
    ) -> Result<OperationResult<OpenHumanEffect>> {
        if !self.applies(conversation) {
            return not_applicable();
        }
        let iteration = session.checkpoint.usage.primary_calls.saturating_add(1);
        self.progress(AgentProgress::IterationStarted {
            iteration,
            max_iterations: u32::MAX,
        })
        .await;

        let mut messages = goose_to_model_messages(conversation);
        if !input.prompt_parts.is_empty() {
            let system = input
                .prompt_parts
                .iter()
                .map(|(name, text)| format!("## {name}\n{text}"))
                .collect::<Vec<_>>()
                .join("\n\n");
            messages.insert(0, tinyinference::message::Message::system(system));
        }
        let advertised_tools = rmcp_tools_to_tiny(&input.tools);
        let mut request = ModelRequest::new(messages).with_tools(advertised_tools.clone());
        if let Some(max_tokens) = self.max_output_tokens {
            request = request.with_max_tokens(max_tokens);
        }

        let response = tokio::select! {
            biased;
            _ = emit.cancelled() => return goose_agent::operation::yielded(),
            response = self.model.invoke(&(), request) => response?,
        };
        let (response, invalid_call) = if is_local_qwen_route(&self.provider_id, &self.model_name) {
            normalize_qwen_response(response, &advertised_tools).into_parts()
        } else {
            (response, None)
        };
        let usage = response
            .usage
            .or(response.message.usage)
            .unwrap_or_default();
        let message = emit.message(tiny_response_to_goose(&response)).await;

        let visible = response.text();
        if !visible.is_empty() {
            self.progress(AgentProgress::TextDelta {
                delta: visible,
                iteration,
            })
            .await;
        }
        let reasoning = response
            .message
            .content
            .iter()
            .filter_map(|block| block.as_thinking().map(|(text, _)| text))
            .collect::<Vec<_>>()
            .join("");
        if !reasoning.is_empty() {
            self.progress(AgentProgress::ThinkingDelta {
                delta: reasoning,
                iteration,
            })
            .await;
        }
        self.progress(AgentProgress::ModelCallCompleted {
            model: self.model_name.clone(),
            provider_id: self.provider_id.clone(),
            subagent_task_id: None,
            input: None,
            output: None,
            iteration,
            input_tokens: usage.input_tokens,
            output_tokens: usage.output_tokens,
            cached_input_tokens: usage.cache_read_tokens,
            cache_creation_tokens: usage.cache_creation_tokens,
            reasoning_tokens: usage.reasoning_tokens,
            cost_usd: 0.0,
        })
        .await;
        self.progress(AgentProgress::TurnCostUpdated {
            model: self.model_name.clone(),
            iteration,
            input_tokens: session
                .checkpoint
                .usage
                .cumulative_input_tokens
                .saturating_add(usage.input_tokens),
            output_tokens: session
                .checkpoint
                .usage
                .cumulative_output_tokens
                .saturating_add(usage.output_tokens),
            cached_input_tokens: session
                .checkpoint
                .usage
                .cumulative_cached_input_tokens
                .saturating_add(usage.cache_read_tokens),
            total_usd: 0.0,
        })
        .await;

        let goose_usage = goose_provider_types::conversation::token_usage::Usage::new(
            Some(usage.input_tokens.min(i32::MAX as u64) as i32),
            Some(usage.output_tokens.min(i32::MAX as u64) as i32),
            Some(usage.effective_total().min(i32::MAX as u64) as i32),
        )
        .with_cache_tokens(
            Some(usage.cache_read_tokens.min(i32::MAX as u64) as i32),
            Some(usage.cache_creation_tokens.min(i32::MAX as u64) as i32),
        );
        emit.emit(goose_agent::events::AgentEvent::Usage(ProviderUsage::new(
            self.model_name.clone(),
            goose_usage,
        )))
        .await;

        let mut effects = vec![
            OpenHumanEffect::from(message),
            OpenHumanEffect::Usage {
                model: self.model_name.clone(),
                input_tokens: usage.input_tokens,
                output_tokens: usage.output_tokens,
                cached_input_tokens: usage.cache_read_tokens,
                cache_creation_tokens: usage.cache_creation_tokens,
                reasoning_tokens: usage.reasoning_tokens,
            },
        ];

        if invalid_call.is_some() {
            if session.checkpoint.protocol_correction_count == 0 {
                let mut correction =
                    Message::user().with_text(build_protocol_correction_prompt(&advertised_tools));
                correction.metadata = MessageMetadata::agent_only();
                let message = emit.message(correction).await;
                effects.push(OpenHumanEffect::from(message));
                effects.push(OpenHumanEffect::IncrementProtocolCorrection);
            } else {
                let terminal = Message::assistant().with_text(protocol_failure_terminal_answer());
                let message = emit.message(terminal).await;
                effects.push(OpenHumanEffect::from(message));
                effects.push(OpenHumanEffect::SetTerminalProtocolFailure);
            }
        }

        applied(effects)
    }
}
