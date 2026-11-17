//! Host-owned agent turn orchestration with module-owned progressive delivery.
use super::delivery::ModuleProgressiveReply;
use crate::core::events::DomainEvent;
use async_trait::async_trait;
use tinybus::EventHandler;
use tinychannels_bus::module_progressive::ProgressiveEvent;

/// Subscribes to `ChannelInboundMessage` events and runs the agent loop,
/// sending replies back to the originating channel via the backend REST API.
pub struct ChannelInboundSubscriber;

impl Default for ChannelInboundSubscriber {
    fn default() -> Self {
        Self::new()
    }
}

impl ChannelInboundSubscriber {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl EventHandler<DomainEvent> for ChannelInboundSubscriber {
    fn name(&self) -> &str {
        "channel::inbound_handler"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel"])
    }

    async fn handle(&self, event: &DomainEvent) {
        let DomainEvent::ChannelInboundMessage {
            event_name: _,
            channel,
            message,
            sender,
            reply_target,
            thread_ts,
            raw_data: _,
        } = event
        else {
            return;
        };

        tracing::info!(
            "[channel-inbound] received message from channel='{}' sender={} len={}",
            channel,
            sender.as_deref().unwrap_or("<unknown>"),
            message.len()
        );

        // Discord's backend supplies the author's id for every forwarded DM.
        // If that contract is broken, the legacy channel-only key would let
        // different users share one chat session and its prepared wallet quote.
        if channel.split(':').next() == Some("discord")
            && sender
                .as_deref()
                .is_none_or(|value| value.trim().is_empty())
        {
            tracing::warn!("[channel-inbound] dropping Discord message without sender id");
            return;
        }

        let session = crate::channels::runtime::session::channel_session();
        // An inbound channel event carries no context of its own.
        let ctx = crate::core::runtime::DispatchCtx::host_default();
        let config = match ctx.load_config().await {
            Ok(config) => config,
            Err(error) => {
                tracing::error!("[channel-inbound] failed to load configuration: {error}");
                return;
            }
        };
        let identity = match crate::modules::channel_controller::prepare_identity(
            &crate::modules::client(config.clone()),
            tinychannels_bus::module_inbound::PrepareChannelIdentityRequest {
                channel: channel.clone(),
                sender: sender.clone(),
                reply_target: reply_target.clone(),
                thread_ts: thread_ts.clone(),
            },
        )
        .await
        {
            Ok(identity) => identity,
            Err(error) => {
                tracing::error!("[channel-inbound] identity preparation failed: {error}");
                return;
            }
        };
        let thread_id = identity.thread_id;
        let client_id = identity.client_id;

        let event_rx = crate::web_chat::subscribe_web_channel_events();
        let reply =
            match ModuleProgressiveReply::open(config, channel.clone(), session.clone()).await {
                Ok(reply) => reply,
                Err(error) => {
                    tracing::error!("[channel-inbound] progressive runtime unavailable: {error}");
                    return;
                }
            };

        let request_id = match crate::web_chat::start_chat(
            &ctx,
            &client_id,
            &thread_id,
            message,
            None,
            None,
            None,
            None,
            crate::web_chat::ChatRequestMetadata {
                // Tag inbound provider messages so traces classify as
                // run:channel_inbound instead of interactive chat.
                source: Some("channel_inbound".to_string()),
                ..Default::default()
            },
        )
        .await
        {
            Ok(rid) => {
                tracing::debug!(
                    "[channel-inbound] agent started request_id={} thread={}",
                    rid,
                    thread_id
                );
                rid
            }
            Err(err) => {
                tracing::error!("[channel-inbound] start_chat failed: {}", err);
                let _ = reply
                    .finalize(&format!("Sorry, I couldn't process your message: {err}"))
                    .await;
                return;
            }
        };

        run_channel_reply_loop(event_rx, request_id, channel.clone(), session, &reply).await;
    }
}

#[async_trait::async_trait]
trait ProgressiveReplySink: Send + Sync {
    async fn feed(&self, events: Vec<ProgressiveEvent>) -> Result<(), String>;
    async fn finalize(&self, text: &str) -> Result<(), String>;
}

#[async_trait::async_trait]
impl ProgressiveReplySink for ModuleProgressiveReply {
    async fn feed(&self, events: Vec<ProgressiveEvent>) -> Result<(), String> {
        ModuleProgressiveReply::feed(self, events)
            .await
            .map_err(|error| error.to_string())
    }

    async fn finalize(&self, text: &str) -> Result<(), String> {
        ModuleProgressiveReply::finalize(self, text)
            .await
            .map_err(|error| error.to_string())
    }
}

/// Consume the backend socket path's events from the shared runner and project
/// progress and terminal output onto its module-owned progressive reply.
async fn run_channel_reply_loop(
    mut event_rx: tokio::sync::broadcast::Receiver<crate::web_chat::ScopedWebEvent>,
    request_id: String,
    channel: String,
    session: tokio_util::sync::CancellationToken,
    reply: &dyn ProgressiveReplySink,
) {
    let mut pending = Vec::with_capacity(32);
    let mut batch_timer = tokio::time::interval(tokio::time::Duration::from_millis(20));
    batch_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        tokio::select! {
            biased;
            _ = session.cancelled() => return,
            _ = batch_timer.tick(), if !pending.is_empty() => {
                if let Err(error) = flush_events(reply, &mut pending).await {
                    tracing::error!("[channel-inbound] delivery failed: {error}");
                    return;
                }
            }
            event = event_rx.recv() => {
                match event {
                    Ok(ev) if ev.context().request_id == request_id => {
                        match ev.wire_event() {
                            crate::web_chat::WebChannelEvent::TextDelta { delta, .. } => {
                                if let Some(delta) = delta.as_ref() {
                                    pending.push(ProgressiveEvent::TextDelta(delta.clone()));
                                }
                            }
                            crate::web_chat::WebChannelEvent::ToolCall {
                                tool_name: Some(name), ..
                            } => pending.push(ProgressiveEvent::ToolCall(name.clone())),
                            crate::web_chat::WebChannelEvent::ToolCall { .. } => {}
                            crate::web_chat::WebChannelEvent::ToolResult {
                                tool_name: Some(name), success, ..
                            } => pending.push(ProgressiveEvent::ToolResult {
                                tool_name: name.clone(),
                                success: success.unwrap_or(true),
                            }),
                            crate::web_chat::WebChannelEvent::ToolResult { .. } => {}
                            crate::web_chat::WebChannelEvent::ThinkingDelta { delta, .. } => {
                                if let Some(delta) = delta.as_ref() {
                                    pending.push(ProgressiveEvent::ThinkingDelta(delta.clone()));
                                }
                            }
                            crate::web_chat::WebChannelEvent::ChatDone { full_response, .. } => {
                                if let Err(error) = flush_events(reply, &mut pending).await {
                                    tracing::error!("[channel-inbound] delivery failed: {error}");
                                    return;
                                }
                                let full_response = full_response.as_deref().unwrap_or_default();
                                let reply_text = if full_response.trim().is_empty() {
                                    tracing::warn!(
                                        "[channel-inbound] agent returned empty response — finalizing draft with fallback",
                                    );
                                    "(No response from agent.)"
                                } else {
                                    full_response
                                };
                                tracing::info!(
                                    "[channel-inbound] agent done, replying to channel='{}' len={}",
                                    channel,
                                    reply_text.len(),
                                );
                                if let Err(error) = reply.finalize(reply_text).await {
                                    tracing::error!("[channel-inbound] final delivery failed: {error}");
                                }
                                return;
                            }
                            crate::web_chat::WebChannelEvent::ChatError { message, .. } => {
                                if let Err(error) = flush_events(reply, &mut pending).await {
                                    tracing::error!("[channel-inbound] delivery failed: {error}");
                                    return;
                                }
                                let err_msg = message.as_deref().unwrap_or("unknown error");
                                tracing::error!("[channel-inbound] agent error: {}", err_msg);
                                let _ = reply.finalize(&format!("Sorry, I encountered an error: {err_msg}")).await;
                                return;
                            }
                            // The runner may emit ChatCancelled on paths without
                            // the compatibility ChatError event; finalize once.
                            crate::web_chat::WebChannelEvent::ChatCancelled { cancel_reason, .. } => {
                                if let Err(error) = flush_events(reply, &mut pending).await {
                                    tracing::error!("[channel-inbound] delivery failed: {error}");
                                    return;
                                }
                                tracing::info!("[channel-inbound] turn cancelled reason={:?}", cancel_reason);
                                let _ = reply.finalize("Cancelled.").await;
                                return;
                            }
                            _ => {}
                        }
                    }
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        tracing::warn!("[channel-inbound] event bus lagged, skipped {} events", n);
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        tracing::error!("[channel-inbound] event bus closed unexpectedly");
                        return;
                    }
                }
                if pending.len() >= 32 {
                    if let Err(error) = flush_events(reply, &mut pending).await {
                        tracing::error!("[channel-inbound] delivery failed: {error}");
                        return;
                    }
                }
            }
        }
    }
}

async fn flush_events(
    reply: &dyn ProgressiveReplySink,
    pending: &mut Vec<ProgressiveEvent>,
) -> Result<(), String> {
    if pending.is_empty() {
        return Ok(());
    }
    reply.feed(std::mem::take(pending)).await
}

#[cfg(test)]
#[path = "subscriber_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "subscriber_loop_tests.rs"]
mod loop_tests;
