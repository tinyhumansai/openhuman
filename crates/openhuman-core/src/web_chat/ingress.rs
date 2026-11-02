//! What a message must clear before it may start, or join, a turn.
//!
//! The turn runner calls [`WebIngress`] once per [`IngressStage`], in its fixed
//! order: stage attachments, apply the prompt guardrail, intercept an approval
//! reply, run the configured `beforeSubmitPrompt` hooks. A refusal or a
//! consumed approval reply never starts a turn; the submitter learns which one
//! through its [`Admission`], and nothing is published on the wire.

use std::sync::{Arc, OnceLock, Weak};

use async_trait::async_trait;
use tinyagents_harness::turn_runner::{
    IngressDecision, IngressPolicy, IngressStage, QueueDisposition, TurnRequest,
};

use super::ops::StartChatError;
use super::runner::{scope_of, Admission, CancelNote, Shared, WebRunner};
use crate::agent::queued_turn::QueuedPayload;
use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;
use crate::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};

const LOG: &str = "[web-channel]";

pub(super) struct WebIngress {
    shared: Arc<Shared>,
    /// The runner this policy is a port of, to see which turn a new message
    /// would interrupt. Set once the runner exists.
    runner: OnceLock<Weak<WebRunner>>,
}

impl WebIngress {
    pub(super) fn new(shared: Arc<Shared>) -> Self {
        Self {
            shared,
            runner: OnceLock::new(),
        }
    }

    pub(super) fn bind(&self, runner: &Arc<WebRunner>) {
        let _ = self.runner.set(Arc::downgrade(runner));
    }

    fn refuse(
        &self,
        request: &TurnRequest<QueuedPayload>,
        error: StartChatError,
    ) -> IngressDecision {
        let reason = error.to_string();
        self.shared
            .admit(&request.request_id, Admission::Rejected(error));
        IngressDecision::Reject(reason)
    }

    /// Save uploaded originals before the message is scanned, remembered or
    /// queued. A missing config or root cannot safely accept an upload.
    async fn stage_attachments(
        &self,
        ctx: &DispatchCtx,
        request: &mut TurnRequest<QueuedPayload>,
    ) -> IngressDecision {
        if !(request.text.contains("[FILE:") || request.text.contains("[IMAGE:")) {
            return IngressDecision::Continue;
        }
        let staged = async {
            let config = ctx
                .load_config()
                .await
                .map_err(|error| format!("upload configuration unavailable: {error}"))?;
            // Stage where the turn will read: the thread's working folder when
            // one is bound (the turn applies the same folder), else action_dir.
            let workspace = crate::attachments::thread_action_dir(
                ctx,
                &config,
                &config.workspace_dir,
                Some(&request.thread_id),
            )
            .await
            .map_err(|error| format!("upload could not be saved: {error}"))?;
            crate::attachments::stage(
                &request.text,
                &request.thread_id,
                &config,
                &crate::attachments::AttachmentAccessScope {
                    external_channel: false,
                    workspace: Some(workspace),
                    ..Default::default()
                },
            )
            .await
            .map_err(|error| format!("upload could not be saved: {error}"))
        }
        .await;
        match staged {
            Ok(text) => {
                request.text = text;
                IngressDecision::Continue
            }
            Err(message) => self.refuse(request, StartChatError::Other(message)),
        }
    }

    fn guardrail(&self, request: &TurnRequest<QueuedPayload>) -> IngressDecision {
        let decision = enforce_prompt_input(
            &request.text,
            PromptEnforcementContext {
                source: "web_chat.start_chat",
                request_id: Some(&request.request_id),
                user_id: Some(&request.payload.client_id),
                session_id: Some(&request.thread_id),
            },
        );
        let verdict = match decision.action {
            PromptEnforcementAction::Allow => return IngressDecision::Continue,
            PromptEnforcementAction::Blocked => "block",
            PromptEnforcementAction::ReviewBlocked => "review_blocked",
        };
        log::warn!(
            "{LOG} prompt rejected client_id={} thread_id={} request_id={} action={verdict} score={:.2} reasons={} hash={} chars={}",
            request.payload.client_id,
            request.thread_id,
            request.request_id,
            decision.score,
            decision
                .reasons
                .iter()
                .map(|r| r.code.as_str())
                .collect::<Vec<_>>()
                .join(","),
            decision.prompt_hash,
            decision.prompt_chars,
        );
        self.refuse(
            request,
            StartChatError::Guardrail {
                verdict: verdict.to_owned(),
                score: decision.score as f64,
                reasons: decision
                    .reasons
                    .iter()
                    .map(|r| crate::web_chat::GuardrailReason {
                        code: r.code.clone(),
                        message: r.message.clone(),
                    })
                    .collect(),
            },
        )
    }

    /// Chat-native approval: when the thread has a parked approval and the
    /// message is a yes/no reply, route it to the gate instead of starting a
    /// turn.
    fn approval_reply(&self, request: &TurnRequest<QueuedPayload>) -> IngressDecision {
        let Some(gate) = crate::security::approval::ApprovalGate::try_global() else {
            return IngressDecision::Continue;
        };
        let Some(approval_id) = gate.pending_for_thread(&request.thread_id) else {
            return IngressDecision::Continue;
        };
        let Some(decision) = crate::security::approval::parse_approval_reply(&request.text) else {
            return IngressDecision::Continue;
        };
        match gate.decide(&approval_id, decision) {
            Ok(Some(_)) => {
                log::info!(
                    "{LOG} routed chat reply to approval gate thread_id={} request_id={approval_id} decision={}",
                    request.thread_id,
                    decision.as_str()
                );
                self.shared
                    .admit(&request.request_id, Admission::Consumed(approval_id));
                IngressDecision::Consumed
            }
            Ok(None) => {
                log::warn!(
                    "{LOG} approval reply targeted a non-pending/already-decided request thread_id={} request_id={approval_id} decision={} — dispatching as fresh turn",
                    request.thread_id,
                    decision.as_str()
                );
                IngressDecision::Continue
            }
            Err(err) => {
                log::warn!(
                    "{LOG} failed to route chat reply to approval gate thread_id={} request_id={approval_id} decision={} err={err}",
                    request.thread_id,
                    decision.as_str()
                );
                IngressDecision::Continue
            }
        }
    }

    /// Configured `beforeSubmitPrompt` hooks. After the approval routing on
    /// purpose: a bare "yes" answering a parked approval is not a prompt the
    /// user submits to the model, and a hook that blocks short messages would
    /// strand a turn waiting for an approval it can no longer receive. The
    /// text is post-attachment-processing, so a hook sees extracted text and
    /// placeholders, not a multi-megabyte data URI on stdin.
    async fn before_submit(
        &self,
        ctx: &DispatchCtx,
        request: &mut TurnRequest<QueuedPayload>,
    ) -> IngressDecision {
        match crate::hooks::ops::prompt_submitted(
            tinyagents_runtime::command_hooks::context::TurnIdentity {
                conversation_id: Some(request.thread_id.clone()),
                ..Default::default()
            },
            &request.text,
            Vec::new(),
        )
        .await
        {
            crate::hooks::PromptVerdict::Submit { additional_context } => {
                if let Some(context) = additional_context {
                    log::debug!(
                        "{LOG} beforeSubmitPrompt hook added {} chars of context thread_id={}",
                        context.chars().count(),
                        request.thread_id
                    );
                    request.text = format!("{}\n\n{context}", request.text);
                }
            }
            crate::hooks::PromptVerdict::Block(reason) => {
                log::info!(
                    "{LOG} prompt blocked by a configured hook thread_id={}: {reason}",
                    request.thread_id
                );
                return self.refuse(request, StartChatError::Other(reason));
            }
        }
        self.accepted(ctx, request).await;
        IngressDecision::Continue
    }

    /// The message is accepted. A fresh accepted user request is the explicit
    /// boundary after Stop, and an interrupting one supersedes the turn in
    /// flight; a message that cannot be queued is admitted as running now.
    async fn accepted(&self, ctx: &DispatchCtx, request: &TurnRequest<QueuedPayload>) {
        crate::agent::orchestration::background_completions::resume_stopped_thread(
            ctx,
            &request.thread_id,
        );
        let in_flight = match self.runner.get().and_then(Weak::upgrade) {
            Some(runner) => {
                runner
                    .in_flight_request_id(&scope_of(ctx), &request.thread_id)
                    .await
            }
            None => None,
        };
        if request.disposition == QueueDisposition::Interrupt {
            if let Some(cancelled) = in_flight.clone() {
                log::info!(
                    "{LOG} interrupting in-flight turn thread_id={} cancelled_request_id={cancelled}",
                    request.thread_id
                );
                self.shared.note_cancel(
                    &cancelled,
                    CancelNote {
                        // The interrupter's client hears how it ended.
                        client_id: Some(request.payload.client_id.clone()),
                        superseded_by: Some(request.request_id.clone()),
                        ..CancelNote::default()
                    },
                );
                crate::core::bus::BUS.publish(DomainEvent::RunQueueInterrupted {
                    thread_id: request.thread_id.clone(),
                    cancelled_request_id: cancelled,
                    item_id: Some(request.request_id.clone()),
                    text_preview: Some(crate::agent::queued_turn::text_preview(&request.text)),
                });
            }
        }
        let can_queue = matches!(
            request.disposition,
            QueueDisposition::Steer | QueueDisposition::Followup | QueueDisposition::Collect
        );
        if !(can_queue && in_flight.is_some()) {
            self.shared.admit(&request.request_id, Admission::Started);
        }
    }
}

#[async_trait]
impl IngressPolicy<QueuedPayload> for WebIngress {
    async fn apply(
        &self,
        stage: IngressStage,
        request: &mut TurnRequest<QueuedPayload>,
    ) -> Result<IngressDecision, String> {
        let Some(ctx) = request.payload.ctx.clone() else {
            return Err("web chat request carries no core context".to_owned());
        };
        Ok(match stage {
            IngressStage::StageAttachments => self.stage_attachments(&ctx, request).await,
            IngressStage::Guardrail => self.guardrail(request),
            IngressStage::ApprovalInterception => self.approval_reply(request),
            IngressStage::BeforeSubmit => self.before_submit(&ctx, request).await,
        })
    }
}

#[cfg(test)]
#[path = "ingress_tests.rs"]
mod tests;
