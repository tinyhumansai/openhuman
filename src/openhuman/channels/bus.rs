//! Event bus handlers for the channels domain.
//!
//! The [`ChannelInboundSubscriber`] handles inbound channel messages published
//! by the socket transport layer. It runs the agent inference loop via the web
//! channel provider and sends the reply back through the REST API.

use crate::core::event_bus::{DomainEvent, EventHandler};
use async_trait::async_trait;
use serde_json::json;

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
impl EventHandler for ChannelInboundSubscriber {
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
            raw_data: _,
        } = event
        else {
            return;
        };

        tracing::info!(
            "[channel-inbound] received message from channel='{}' len={}",
            channel,
            message.len()
        );

        let thread_id = format!("channel:{}", channel);
        let client_id = "inbound".to_string();

        let mut event_rx =
            crate::openhuman::channels::providers::web::subscribe_web_channel_events();

        let request_id = match crate::openhuman::channels::providers::web::start_chat(
            &client_id, &thread_id, message, None, None,
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
                send_channel_reply(
                    channel,
                    &format!("Sorry, I couldn't process your message: {err}"),
                )
                .await;
                return;
            }
        };

        let timeout = tokio::time::Duration::from_secs(180);
        let deadline = tokio::time::Instant::now() + timeout;

        loop {
            tokio::select! {
                event = event_rx.recv() => {
                    match event {
                        Ok(ev) if ev.request_id == request_id => {
                            if ev.event == "chat_done" || ev.event == "chat:done" {
                                let reply = ev.full_response.unwrap_or_default();
                                if reply.trim().is_empty() {
                                    tracing::warn!("[channel-inbound] agent returned empty response");
                                    return;
                                }
                                tracing::info!(
                                    "[channel-inbound] agent done, replying to channel='{}' len={}",
                                    channel,
                                    reply.len()
                                );
                                send_channel_reply(channel, &reply).await;
                                return;
                            }
                            if ev.event == "chat_error" || ev.event == "chat:error" {
                                let err_msg = ev.message.unwrap_or_else(|| "unknown error".to_string());
                                tracing::error!("[channel-inbound] agent error: {}", err_msg);
                                send_channel_reply(
                                    channel,
                                    &format!("Sorry, I encountered an error: {err_msg}"),
                                )
                                .await;
                                return;
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
                }
                _ = tokio::time::sleep_until(deadline) => {
                    tracing::error!("[channel-inbound] agent timed out after {}s", timeout.as_secs());
                    send_channel_reply(channel, "Sorry, the request timed out.").await;
                    return;
                }
            }
        }
    }
}

/// Send a text reply back to a channel via the backend REST API.
async fn send_channel_reply(channel: &str, text: &str) {
    let config = match crate::openhuman::config::rpc::load_config_with_timeout().await {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to load config: {}", e);
            return;
        }
    };

    let api_url = crate::api::config::effective_api_url(&config.api_url);
    let jwt = match crate::api::jwt::get_session_token(&config) {
        Ok(Some(t)) => t,
        Ok(None) => {
            tracing::error!("[channel-inbound] no session JWT — cannot reply");
            return;
        }
        Err(e) => {
            tracing::error!("[channel-inbound] failed to get session token: {}", e);
            return;
        }
    };

    let client = match crate::api::rest::BackendOAuthClient::new(&api_url) {
        Ok(c) => c,
        Err(e) => {
            tracing::error!("[channel-inbound] failed to create API client: {}", e);
            return;
        }
    };

    let body = json!({ "text": text });
    match client.send_channel_message(channel, &jwt, body).await {
        Ok(resp) => {
            tracing::info!(
                "[channel-inbound] reply sent to channel='{}' response={:?}",
                channel,
                resp
            );
        }
        Err(e) => {
            tracing::error!(
                "[channel-inbound] failed to send reply to channel='{}': {}",
                channel,
                e
            );
        }
    }
}
