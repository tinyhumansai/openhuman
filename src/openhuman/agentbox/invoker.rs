//! Bridges AgentBox `/run` invocations to OpenHuman's agent runtime.
//!
//! Each `invoke` call drives one user turn through the full agent runtime
//! (skills, tools, memory) by submitting it through the same web-channel
//! pipeline that the desktop UI uses, then waiting for the matching
//! `chat_done` / `chat_error` event on the in-process broadcast bus.

use async_trait::async_trait;
use std::sync::Arc;
use tokio::sync::broadcast::error::RecvError;

use crate::openhuman::channels::providers::web::{
    start_chat, subscribe_web_channel_events, ChatRequestMetadata,
};
use crate::openhuman::memory::rpc_models::CreateConversationThreadRequest;
use crate::openhuman::threads::ops::thread_create_new;

/// Bridges AgentBox `/run` invocations to OpenHuman's agent runtime.
///
/// Implementations resolve (or create) a thread, drive a single user turn
/// through the full agent runtime (skills, tools, memory), and return the
/// final assistant text + the thread id used.
#[async_trait]
pub trait AgentInvoker: Send + Sync + 'static {
    async fn invoke(
        &self,
        thread_id: Option<&str>,
        message: &str,
    ) -> Result<InvocationOutput, String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InvocationOutput {
    pub assistant_message: String,
    pub thread_id: String,
}

/// Production impl — submits the user turn through the web-channel pipeline
/// (`start_chat`) and awaits the matching `chat_done` event on the in-process
/// broadcast bus. The web pipeline is the same code path the desktop UI
/// drives, so this exercises skills/tools/memory exactly the same way.
#[derive(Default)]
pub struct CoreAgentInvoker;

/// Stable client-id prefix for AgentBox-initiated chats. Keeps audit logs,
/// per-session caches, and analytics able to distinguish marketplace traffic
/// from live UI traffic. Each invocation appends a per-call UUID to avoid
/// collisions when two parallel jobs target the same thread.
const AGENTBOX_CLIENT_PREFIX: &str = "agentbox";

#[async_trait]
impl AgentInvoker for CoreAgentInvoker {
    async fn invoke(
        &self,
        thread_id: Option<&str>,
        message: &str,
    ) -> Result<InvocationOutput, String> {
        // 1. Resolve thread — caller-supplied id wins; otherwise create a
        //    fresh thread via the same op the UI uses so the conversation is
        //    discoverable from the desktop client.
        let resolved_thread_id = match thread_id {
            Some(id) if !id.trim().is_empty() => id.trim().to_string(),
            _ => {
                let outcome = thread_create_new(CreateConversationThreadRequest {
                    labels: None,
                    personality_id: None,
                })
                .await
                .map_err(|err| format!("agentbox: thread_create_new failed: {err}"))?;
                outcome
                    .value
                    .data
                    .ok_or_else(|| {
                        "agentbox: thread_create_new returned no data envelope".to_string()
                    })?
                    .id
            }
        };

        // 2. Subscribe BEFORE submitting so the race window between
        //    `start_chat` returning and the agent emitting `chat_done` cannot
        //    drop our event.
        let mut events = subscribe_web_channel_events();

        // 3. Submit via the web-channel pipeline. Per-job UUID client_id keeps
        //    parallel AgentBox jobs from masquerading as the same client in
        //    audit/analytics surfaces; the broadcast filter below matches on
        //    `request_id`, so this label is identity-only.
        let client_id = format!("{AGENTBOX_CLIENT_PREFIX}:{}", uuid::Uuid::new_v4());
        let request_id = start_chat(
            &client_id,
            &resolved_thread_id,
            message,
            None,
            None,
            None,
            None,
            None,
            ChatRequestMetadata::agentbox(),
        )
        .await
        .map_err(|err| format!("agentbox: start_chat failed: {err}"))?;

        log::info!(
            "[agentbox] submitted chat client_id={} thread_id={} request_id={} message_chars={}",
            client_id,
            resolved_thread_id,
            request_id,
            message.chars().count()
        );

        // 4. Wait for the matching completion event. Per-job timeout is
        //    enforced by the caller (`run_job` wraps this future in
        //    `tokio::time::timeout`); dropping our broadcast receiver here on
        //    cancellation cleans up correctly (RAII).
        //
        //    Match on `request_id` (request-scoped) so we don't accidentally
        //    pick up an unrelated turn that happens to share the same
        //    thread/client.
        loop {
            let event = match events.recv().await {
                Ok(ev) => ev,
                Err(RecvError::Lagged(n)) => {
                    log::warn!("[agentbox] event bus lagged, skipped {n} events");
                    continue;
                }
                Err(RecvError::Closed) => {
                    return Err("agentbox: event stream closed".into());
                }
            };

            if event.request_id != request_id {
                continue;
            }

            match event.event.as_str() {
                "chat_done" | "chat:done" => {
                    let reply = event.full_response.unwrap_or_default();
                    if reply.is_empty() {
                        log::warn!(
                            "[agentbox] chat_done with empty full_response request_id={request_id}"
                        );
                    }
                    log::info!(
                        "[agentbox] chat completed thread_id={} request_id={} reply_chars={}",
                        resolved_thread_id,
                        request_id,
                        reply.chars().count()
                    );
                    return Ok(InvocationOutput {
                        assistant_message: reply,
                        thread_id: resolved_thread_id,
                    });
                }
                "chat_error" | "chat:error" => {
                    let detail = event
                        .message
                        .clone()
                        .unwrap_or_else(|| "unknown chat error".to_string());
                    let kind = event.error_type.as_deref().unwrap_or("error");
                    log::warn!(
                        "[agentbox] chat failed thread_id={} request_id={} kind={} message={}",
                        resolved_thread_id,
                        request_id,
                        kind,
                        detail
                    );
                    return Err(format!("agentbox: chat_error ({kind}): {detail}"));
                }
                _ => {
                    // Streaming progress / tool deltas — keep waiting.
                    continue;
                }
            }
        }
    }
}

/// Convenience alias used by the rest of the module.
pub type SharedInvoker = Arc<dyn AgentInvoker>;
