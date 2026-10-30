//! `beforeSubmitPrompt` for channel turns: the native runner's ingress policy.

use async_trait::async_trait;
use tinyagents_harness::turn_runner::{IngressDecision, IngressPolicy, IngressStage, TurnRequest};

use crate::agent::queued_turn::QueuedPayload;

pub(super) struct NativeIngress;

impl NativeIngress {
    /// Configured `beforeSubmitPrompt` hooks, as the web chat runs them. Added
    /// context reaches the model on both inputs a channel turn may carry: the
    /// text, and the typed message a media turn is answered from. A block
    /// rejects the turn; [`super::run`] replies with the hook's reason.
    async fn before_submit(request: &mut TurnRequest<QueuedPayload>) -> IngressDecision {
        let identity = tinyagents_runtime::command_hooks::context::TurnIdentity {
            conversation_id: Some(request.thread_id.clone()),
            ..Default::default()
        };
        let prompt = request
            .payload
            .native_user_text
            .as_deref()
            .unwrap_or(&request.text);
        match crate::hooks::ops::prompt_submitted(identity, prompt, Vec::new()).await {
            crate::hooks::PromptVerdict::Submit {
                additional_context: Some(context),
            } => {
                log::debug!(
                    "[channels::dispatch] beforeSubmitPrompt hook added {} chars of context thread_id={}",
                    context.chars().count(),
                    request.thread_id
                );
                if let Some(tinyinference_llm::message::Message::User(input)) =
                    request.payload.native_message_input.as_mut()
                {
                    input
                        .content
                        .push(tinyinference_llm::message::ContentBlock::Text(
                            context.clone(),
                        ));
                }
                request.text = format!("{}\n\n{context}", request.text);
                IngressDecision::Continue
            }
            crate::hooks::PromptVerdict::Submit { .. } => IngressDecision::Continue,
            crate::hooks::PromptVerdict::Block(reason) => {
                // The reason is hook-authored and may quote the prompt: it goes
                // to the sender, never to the log.
                log::info!(
                    "[channels::dispatch] prompt blocked by a configured hook thread_id={}",
                    request.thread_id
                );
                IngressDecision::Reject(reason)
            }
        }
    }
}

#[async_trait]
impl IngressPolicy<QueuedPayload> for NativeIngress {
    async fn apply(
        &self,
        stage: IngressStage,
        request: &mut TurnRequest<QueuedPayload>,
    ) -> Result<IngressDecision, String> {
        Ok(match stage {
            IngressStage::BeforeSubmit => Self::before_submit(request).await,
            _ => IngressDecision::Continue,
        })
    }
}
