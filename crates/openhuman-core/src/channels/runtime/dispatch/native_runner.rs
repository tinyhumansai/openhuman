//! Native channel adapter for TinyAgents' shared turn lifecycle.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
};

use async_trait::async_trait;
use tinyagents_harness::{
    run_queue::RunQueue,
    turn_runner::{
        EventSink, FailureClassification, FailureClassifier, IngressDecision, IngressPolicy,
        IngressStage, QueueDisposition, ReplyStore, ScopeKey, Summarizer, TerminalEvent, TurnHost,
        TurnRequest, TurnResult, TurnRunner, TurnRunnerConfig, TurnSession, TurnSignal,
    },
    CancellationToken,
};
use tokio::sync::mpsc;

use crate::{
    agent::{queued_turn::QueuedPayload, session_lease::SessionLease},
    channels::bus::NativeProgressiveReply,
    core::runtime::DispatchCtx,
};

type Runner = TurnRunner<
    NativeHost,
    NativeIngress,
    NativeReplies,
    NativeEvents,
    NativeFailures,
    NativeSummarizer,
    QueuedPayload,
>;

#[derive(Default)]
struct NativeSlot {
    lease: Option<SessionLease>,
    profile_turn: Option<crate::profiles::turn_gate::TurnGuard>,
    busy: Option<crate::agent::orchestration::busy_guard::TurnBusy>,
    native_reply: Option<Arc<NativeProgressiveReply>>,
    reply: Option<String>,
    progress_failed: bool,
    waiting_for_delivery: bool,
    delivery_succeeded: Option<bool>,
    #[cfg(test)]
    failure_message: Option<String>,
}

#[derive(Default)]
struct NativeShared {
    slots: Mutex<HashMap<String, NativeSlot>>,
}

impl NativeShared {
    fn with<R>(&self, request_id: &str, f: impl FnOnce(&mut NativeSlot) -> R) -> R {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        f(slots.entry(request_id.to_owned()).or_default())
    }

    fn begin_delivery_wait(&self, request_id: &str) {
        self.with(request_id, |slot| slot.waiting_for_delivery = true);
    }

    fn record_delivery(&self, request_id: &str, delivered: bool) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(slot) = slots.get_mut(request_id) {
            slot.delivery_succeeded = Some(delivered);
            // The lease spans model execution and terminal provider delivery.
            slot.lease = None;
            slot.profile_turn = None;
            slot.busy = None;
            if !slot.waiting_for_delivery {
                slots.remove(request_id);
            }
        }
    }

    fn abandon_delivery_wait(&self, request_id: &str) {
        let mut slots = self
            .slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(slot) = slots.get_mut(request_id) {
            slot.waiting_for_delivery = false;
            if slot.delivery_succeeded.is_some() {
                slots.remove(request_id);
            }
        }
    }

    fn take_delivery(&self, request_id: &str) -> Option<bool> {
        self.slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .remove(request_id)
            .and_then(|slot| slot.delivery_succeeded)
    }
}

struct NativeHost {
    shared: Arc<NativeShared>,
}

#[async_trait]
impl TurnHost<QueuedPayload> for NativeHost {
    async fn checkout(&self, request: &TurnRequest<QueuedPayload>) -> Result<TurnSession, String> {
        self.checkout_session(request, false).await
    }

    async fn checkout_fork(
        &self,
        request: &TurnRequest<QueuedPayload>,
    ) -> Result<TurnSession, String> {
        self.checkout_session(request, true).await
    }

    fn run_origin(&self, request: &TurnRequest<QueuedPayload>) -> serde_json::Value {
        let Some(trusted) = request.payload.trusted_turn.as_ref() else {
            return serde_json::Value::Null;
        };
        match &trusted.origin {
            crate::security::turn_origin::AgentTurnOrigin::ExternalChannel {
                channel,
                sender,
                reply_target,
                message_id,
                ..
            } => serde_json::json!({
                "kind": "external_channel",
                "channel": channel,
                "sender": sender,
                "reply_target": reply_target,
                "message_id": message_id,
            }),
            _ => serde_json::json!({"kind": "trusted_host_turn"}),
        }
    }

    async fn run(
        &self,
        request: &TurnRequest<QueuedPayload>,
        session: &TurnSession,
        _origin: &serde_json::Value,
        cancellation: CancellationToken,
        queue: Arc<RunQueue<TurnRequest<QueuedPayload>>>,
        progress: mpsc::Sender<TurnSignal>,
    ) -> Result<TurnResult, String> {
        if let Some(error) = request.payload.native_preparation_error.as_ref() {
            return Err(error.clone());
        }
        let trusted = request
            .payload
            .trusted_turn
            .as_ref()
            .ok_or_else(|| "channel turn has no trusted host context".to_owned())?;
        let (agent_progress, mut progress_rx) = mpsc::channel(64);
        let progress_forwarder = request.payload.ctx.as_ref().map(|dispatch| {
            tokio_util::task::AbortOnDropHandle::new(dispatch.clone().spawn(async move {
                while let Some(event) = progress_rx.recv().await {
                    if progress
                        .send(TurnSignal::Progress(Box::new(event)))
                        .await
                        .is_err()
                    {
                        break;
                    }
                }
            }))
        });
        let response = crate::channels::runtime::dispatch::host_agent::run_host_agent_turn(
            trusted.host_agent.clone(),
            crate::channels::runtime::dispatch::host_agent::HostChannelTurn {
                relay_thread: trusted.relay_thread.clone(),
                channel: trusted.channel.clone(),
                session_key: session.session_id.clone(),
                event_session_id: trusted.event_session_id.clone(),
                message: request.text.clone(),
                message_input: request.payload.native_message_input.clone(),
                staged_parts: request.payload.native_staged_parts.clone(),
                staged_lease: request.payload.native_staged_lease.clone(),
                staged_message_id: request.payload.native_message_id.clone(),
                origin: trusted.origin.clone(),
                progress: Some(agent_progress),
                run_queue: Some(queue),
                approval: trusted.approval.clone(),
                cancellation,
                turn_deadline: None,
                imported_history: trusted.imported_history.clone(),
                legacy_history: trusted.legacy_history.clone(),
            },
        )
        .await
        .map_err(|error| error.to_string());
        if let Some(mut progress_forwarder) = progress_forwarder {
            if tokio::time::timeout(std::time::Duration::from_secs(5), &mut progress_forwarder)
                .await
                .is_err()
            {
                progress_forwarder.abort();
            }
        }
        response.map(|response| TurnResult {
            reply: response.text,
            data: serde_json::json!({
                "provider": response.resolved_provider,
                "model": response.resolved_model,
            }),
        })
    }

    async fn checkin(&self, _request: &TurnRequest<QueuedPayload>) {}

    async fn cancel_children(&self, _request: &TurnRequest<QueuedPayload>) {
        // Channel turns preserve the web host's scoped-stop policy: the
        // per-turn cancellation token reaches the parent session, while
        // detached sub-agents remain independently owned and cancellable.
    }
}

impl NativeHost {
    async fn checkout_session(
        &self,
        request: &TurnRequest<QueuedPayload>,
        fork: bool,
    ) -> Result<TurnSession, String> {
        let trusted = request
            .payload
            .trusted_turn
            .as_ref()
            .ok_or_else(|| "channel turn has no trusted host context".to_owned())?;
        let dispatch = request
            .payload
            .ctx
            .as_ref()
            .ok_or_else(|| "channel turn has no core context".to_owned())?;
        let tenant = dispatch.tenant().map_err(|error| error.to_string())?;
        let tenant_scope = crate::core::runtime::tenant::tenant_key(&tenant, "");
        let session_id = if fork {
            format!("{}:parallel:{}", trusted.session_key, request.request_id)
        } else {
            trusted.session_key.clone()
        };
        let lease = SessionLease::acquire(
            crate::agent::session_lease::SessionLeaseKey::new(
                tenant_scope,
                trusted.host_agent.definition.id.clone(),
                session_id.clone(),
            ),
            None,
        )
        .await
        .map_err(|()| "channel session lease was cancelled".to_owned())?;
        // Match WebHost's lock order exactly: session lease before the
        // profile-wide chat/cron gate. Both acquisitions happen in checkout,
        // after TurnRunner admission, and cancellation drops either future.
        let profile_turn = crate::profiles::turn_gate::acquire(dispatch).await;
        // Relay keeps its thread busy through the small post-run durable
        // reply-recording tail in `run_relay_turn`; other native turns can
        // release their busy guard with progressive terminal delivery.
        let busy = if trusted.relay_thread.is_none() {
            Some(crate::agent::orchestration::busy_guard::TurnBusy::start(
                dispatch,
                &session_id,
            ))
        } else {
            None
        };
        let native_reply = match trusted.native_target.as_ref() {
            Some(target) => Some(
                target
                    .open(&request.request_id)
                    .await
                    .map_err(|error| format!("native progressive output unavailable: {error}"))?,
            ),
            None => None,
        };
        self.shared.with(&request.request_id, |slot| {
            slot.lease = Some(lease);
            slot.profile_turn = profile_turn;
            slot.busy = busy;
            slot.native_reply = native_reply;
        });
        Ok(TurnSession {
            session_id,
            fingerprint: None,
        })
    }
}

struct NativeIngress;

#[async_trait]
impl IngressPolicy<QueuedPayload> for NativeIngress {
    async fn apply(
        &self,
        _stage: IngressStage,
        _request: &mut TurnRequest<QueuedPayload>,
    ) -> Result<IngressDecision, String> {
        Ok(IngressDecision::Continue)
    }
}

struct NativeReplies {
    shared: Arc<NativeShared>,
}

#[async_trait]
impl ReplyStore<QueuedPayload> for NativeReplies {
    async fn persist(
        &self,
        request: &TurnRequest<QueuedPayload>,
        _reply_id: &str,
        result: &TurnResult,
    ) -> Result<(), String> {
        self.shared.with(&request.request_id, |slot| {
            slot.reply = Some(result.reply.clone())
        });
        Ok(())
    }
}

struct NativeEvents {
    shared: Arc<NativeShared>,
}

fn render_classified_native_failure(
    config: Option<&crate::config::Config>,
    fallback: &str,
    classification: &FailureClassification,
) -> String {
    config.map_or_else(
        || fallback.to_owned(),
        |config| {
            config.render_native_channel_failure(
                fallback,
                crate::web_chat::failure_details::copy_key(classification).as_deref(),
                crate::web_chat::failure_details::copy_params(classification).as_ref(),
            )
        },
    )
}

#[async_trait]
impl EventSink<QueuedPayload> for NativeEvents {
    async fn progress(
        &self,
        request: &TurnRequest<QueuedPayload>,
        event: tinyagents_harness::progress::AgentProgress,
    ) {
        let Some(event) = super::processor::progressive::output_event(event) else {
            return;
        };
        let reply = self
            .shared
            .with(&request.request_id, |slot| slot.native_reply.clone());
        let Some(reply) = reply else {
            return;
        };
        if self
            .shared
            .with(&request.request_id, |slot| slot.progress_failed)
        {
            return;
        }
        match reply.feed(vec![event]).await {
            Ok(result) if super::processor::progressive::delivery_succeeded(&result) => {}
            Ok(_) | Err(_) => {
                self.shared
                    .with(&request.request_id, |slot| slot.progress_failed = true);
                if let Err(error) = reply.close().await {
                    tracing::warn!("[dispatch] native progressive close failed: {error}");
                }
            }
        }
    }

    async fn terminal(&self, request: &TurnRequest<QueuedPayload>, event: TerminalEvent) {
        let reply = self
            .shared
            .with(&request.request_id, |slot| slot.native_reply.clone());
        let (text, cancelled) = match event {
            TerminalEvent::Completed { .. } => (
                self.shared
                    .with(&request.request_id, |slot| slot.reply.clone()),
                false,
            ),
            TerminalEvent::Failed { message, .. } => (Some(message), false),
            TerminalEvent::Cancelled { .. } => (None, true),
        };
        let progress_failed = self
            .shared
            .with(&request.request_id, |slot| slot.progress_failed);
        let delivered = if cancelled || progress_failed {
            if let Some(reply) = reply.as_ref() {
                if let Err(error) = reply.close().await {
                    tracing::warn!("[dispatch] native progressive close failed: {error}");
                }
            }
            false
        } else if let (Some(reply), Some(text)) = (reply.as_ref(), text.as_deref()) {
            super::processor::progressive::finish_native_reply(&Some(Arc::clone(reply)), text).await
        } else {
            false
        };
        self.shared.record_delivery(&request.request_id, delivered);
    }

    async fn classified_failure(
        &self,
        request: &TurnRequest<QueuedPayload>,
        event: TerminalEvent,
        classification: FailureClassification,
    ) {
        let event = match event {
            TerminalEvent::Failed {
                request_id,
                message,
                class,
            } => {
                let fallback = if classification.message.is_empty() {
                    message
                } else {
                    classification.message.clone()
                };
                let config = if let Some(ctx) = request.payload.ctx.as_ref() {
                    ctx.load_config().await.ok()
                } else {
                    None
                };
                let localized =
                    render_classified_native_failure(config.as_ref(), &fallback, &classification);
                #[cfg(test)]
                self.shared.with(&request.request_id, |slot| {
                    slot.failure_message = Some(localized.clone());
                });
                TerminalEvent::Failed {
                    request_id,
                    message: localized,
                    class,
                }
            }
            other => other,
        };
        self.terminal(request, event).await;
    }
}

struct NativeFailures;

impl FailureClassifier for NativeFailures {
    fn classify(&self, error: &str) -> (String, String) {
        if error == crate::channels::failure_copy::LEGACY_HISTORY_UNAVAILABLE {
            return (
                "channel_history_unavailable".to_owned(),
                "Something went wrong. Please try again.".to_owned(),
            );
        }
        let classified = crate::web_chat::classify_inference_error(error);
        (classified.error_type.to_owned(), classified.message)
    }

    fn classify_failure(&self, error: &str) -> FailureClassification {
        if error == crate::channels::failure_copy::LEGACY_HISTORY_UNAVAILABLE {
            return FailureClassification {
                class: "channel_history_unavailable".to_owned(),
                message: "Something went wrong. Please try again.".to_owned(),
                details: Some(crate::web_chat::failure_details::encode(
                    Some("chat_error.inference".to_owned()),
                    None,
                    error,
                )),
            };
        }
        let classified = crate::web_chat::classify_inference_error(error);
        FailureClassification {
            class: classified.error_type.to_owned(),
            message: classified.message,
            details: Some(crate::web_chat::failure_details::encode(
                Some(classified.copy_key.to_owned()),
                classified.copy_params,
                error,
            )),
        }
    }
}

struct NativeSummarizer;

#[async_trait]
impl Summarizer<QueuedPayload> for NativeSummarizer {
    async fn grounded_close(
        &self,
        request: &TurnRequest<QueuedPayload>,
        reason: &str,
    ) -> Option<TurnResult> {
        let error = if reason.contains("idle") {
            let idle = request
                .idle_timeout
                .unwrap_or_else(crate::agent::turn_policy::configured_idle_window);
            crate::web_chat::turn_idle_error_message(idle.as_secs())
        } else {
            let deadline = request.deadline.unwrap_or_default();
            crate::web_chat::turn_timeout_error_message(deadline.as_secs())
        };
        let classified = NativeFailures.classify_failure(&error);
        let config = if let Some(ctx) = request.payload.ctx.as_ref() {
            ctx.load_config().await.ok()
        } else {
            None
        };
        let message =
            render_classified_native_failure(config.as_ref(), &classified.message, &classified);
        Some(TurnResult {
            reply: message,
            data: serde_json::json!({"failure_type": classified.class}),
        })
    }
}

struct NativeRuntime {
    shared: Arc<NativeShared>,
    runner: Arc<Runner>,
}

impl Default for NativeRuntime {
    fn default() -> Self {
        let shared = Arc::new(NativeShared::default());
        let runner = Arc::new(TurnRunner::new(
            Arc::new(NativeHost {
                shared: shared.clone(),
            }),
            Arc::new(NativeIngress),
            Arc::new(NativeReplies {
                shared: shared.clone(),
            }),
            Arc::new(NativeEvents {
                shared: shared.clone(),
            }),
            Arc::new(NativeFailures),
            Arc::new(NativeSummarizer),
            TurnRunnerConfig::default(),
        ));
        Self { shared, runner }
    }
}

fn channel_turn_deadline(
    message_timeout_secs: u64,
    web_backstop: Option<std::time::Duration>,
) -> std::time::Duration {
    let channel_deadline = std::time::Duration::from_secs(
        message_timeout_secs
            .max(tinychannels_bus::module_runtime::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS),
    );
    web_backstop.map_or(channel_deadline, |backstop| backstop.min(channel_deadline))
}

pub(crate) struct NativeTurnExecution {
    pub(crate) result: Result<crate::agent::bus::AgentTurnResponse, String>,
    pub(crate) delivery_succeeded: Option<bool>,
    pub(crate) queued: bool,
}

pub(crate) async fn run(
    dispatch: &DispatchCtx,
    thread_id: String,
    request_id: String,
    text: String,
    message_timeout_secs: u64,
    message_input: Option<tinyinference_llm::message::Message>,
    staged_parts: Vec<tinychannels_bus::ContentPart>,
    staged_lease: Option<crate::threads::attachments::NativeStagedLease>,
    staged_message_id: Option<String>,
    preparation_error: Option<String>,
    trusted: Arc<crate::agent::turn_host::TrustedAgentTurn>,
) -> NativeTurnExecution {
    let runtime = crate::core::runtime::slot_for::<NativeRuntime>(dispatch);
    let tenant = match dispatch.tenant() {
        Ok(tenant) => tenant,
        Err(error) => {
            return NativeTurnExecution {
                result: Err(error.to_string()),
                delivery_succeeded: None,
                queued: false,
            };
        }
    };
    runtime.shared.begin_delivery_wait(&request_id);
    let deadline = channel_turn_deadline(
        message_timeout_secs,
        crate::web_chat::turn_guards::backstop::configured_backstop(),
    );
    let request = TurnRequest {
        scope: ScopeKey {
            agent_id: trusted.host_agent.definition.id.clone(),
            profile_id: tenant.profile,
        },
        thread_id,
        request_id: request_id.clone(),
        text,
        payload: QueuedPayload {
            ctx: Some(dispatch.clone()),
            trusted_turn: Some(trusted),
            native_message_input: message_input,
            native_staged_parts: staged_parts,
            native_staged_lease: staged_lease.clone(),
            native_message_id: staged_message_id,
            native_preparation_error: preparation_error,
            ..Default::default()
        },
        host_authored: false,
        disposition: QueueDisposition::Interrupt,
        deadline: Some(deadline),
        idle_timeout: Some(crate::agent::turn_policy::configured_idle_window()),
    };
    let runner_result = runtime.runner.run(request).await;
    let queued = matches!(
        &runner_result,
        Err(tinyagents_harness::turn_runner::TurnRunnerError::Queued)
    );
    let result = runner_result.map_err(|error| error.to_string());
    if !queued && result.is_err() {
        if let Some(staged_lease) = staged_lease.as_ref() {
            if staged_lease.discard().await.is_err() {
                tracing::warn!(
                    "[channels::dispatch] failed to clean rejected native media staging"
                );
            }
        }
    }
    if queued {
        runtime.shared.abandon_delivery_wait(&request_id);
    }
    let result = result.map(|result| {
        let provider = result.data["provider"].as_str().map(str::to_owned);
        let model = result.data["model"].as_str().map(str::to_owned);
        crate::agent::bus::AgentTurnResponse {
            text: result.reply,
            resolved_provider: provider,
            resolved_model: model,
        }
    });
    NativeTurnExecution {
        result,
        delivery_succeeded: if queued {
            None
        } else {
            runtime.shared.take_delivery(&request_id)
        },
        queued,
    }
}

#[cfg(test)]
#[path = "native_runner_tests.rs"]
mod tests;
