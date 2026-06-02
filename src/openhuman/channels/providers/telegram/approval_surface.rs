//! Bridge `ApprovalRequested` domain events to Telegram chat messages.
//!
//! Background — sub-issue 2 of #3098: prior to this subscriber, channel
//! turns initiated from Telegram (Discord, Slack, iMessage, …) carried
//! no [`ApprovalChatContext`], so the [`ApprovalGate`]'s "no chat
//! context → allow straight through" branch (`approval/gate.rs:225-231`)
//! silently bypassed every `Prompt`-class tool call. A user on
//! `level=supervised` got the same unprompted behavior as `level=full`
//! over Telegram, which voids the entire supervised approval model.
//!
//! This subscriber, paired with the [`APPROVAL_CHAT_CONTEXT`] scope set
//! in `channels/runtime/dispatch.rs` for Telegram turns, makes the
//! approval gate actually fire over Telegram:
//!
//! 1. The dispatch loop scopes the agent turn in an [`ApprovalChatContext`]
//!    whose `thread_id` is the conversation history key and `client_id`
//!    is `"telegram"`.
//! 2. When a tool call gets parked, the gate publishes
//!    [`DomainEvent::ApprovalRequested`] with those identifiers.
//! 3. This subscriber sees the event, looks up the original
//!    `(reply_target, thread_ts)` by `thread_id` (populated from the
//!    parallel [`DomainEvent::ChannelMessageReceived`] stream), and sends
//!    a Telegram message asking the user to reply `yes`/`no`.
//! 4. The user replies in Telegram; the dispatch loop intercepts the
//!    reply, parses it via [`parse_approval_reply`], and routes it to
//!    [`ApprovalGate::decide`] — resuming the parked tool call.
//!
//! Discord, Slack, iMessage, and Mattermost are still in the silent
//! bypass state (no per-channel surface subscriber, no
//! `ApprovalChatContext` scoping). Each will get its own follow-up PR.
//!
//! [`APPROVAL_CHAT_CONTEXT`]: crate::openhuman::approval::APPROVAL_CHAT_CONTEXT
//! [`ApprovalChatContext`]: crate::openhuman::approval::ApprovalChatContext
//! [`ApprovalGate`]: crate::openhuman::approval::ApprovalGate
//! [`ApprovalGate::decide`]: crate::openhuman::approval::ApprovalGate::decide
//! [`parse_approval_reply`]: crate::openhuman::approval::parse_approval_reply

use crate::core::event_bus::{DomainEvent, EventHandler};
use crate::openhuman::channels::traits::SendMessage;
use crate::openhuman::channels::Channel;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

const LOG_PREFIX: &str = "[telegram-approval]";

/// Identifier the dispatch loop sets as `ApprovalChatContext.client_id`
/// for Telegram-originated turns. Used by this subscriber to filter
/// `ApprovalRequested` events down to the ones it should surface.
pub const TELEGRAM_APPROVAL_CLIENT_ID: &str = "telegram";

/// Reply context captured from a Telegram inbound message so a later
/// `ApprovalRequested` event can be sent back to the right Telegram chat
/// (and reply thread, when one is in use).
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ReplyContext {
    pub(crate) reply_target: String,
    pub(crate) thread_ts: Option<String>,
}

/// Same shape as `channels::context::conversation_history_key` but
/// reconstructable from individual `ChannelMessageReceived` event fields
/// (the helper takes a `&ChannelMessage`). Keep in sync with that
/// helper's Telegram branch — Telegram drops `thread_ts` from the key so
/// reply threads stay glued to the same history.
fn telegram_history_key(sender: &str, reply_target: &str) -> String {
    format!("telegram_{sender}_{reply_target}")
}

/// Subscriber that turns `ApprovalRequested` events for Telegram-originated
/// turns into Telegram messages. Holds a small in-memory map keyed by the
/// conversation history key so the inbound message context (reply_target,
/// thread_ts) is available when an approval surfaces later in the same
/// turn.
pub struct TelegramApprovalSurfaceSubscriber {
    channels_by_name: Arc<HashMap<String, Arc<dyn Channel>>>,
    reply_index: Arc<Mutex<HashMap<String, ReplyContext>>>,
}

impl TelegramApprovalSurfaceSubscriber {
    pub fn new(channels_by_name: Arc<HashMap<String, Arc<dyn Channel>>>) -> Self {
        Self {
            channels_by_name,
            reply_index: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// For tests: snapshot a reply context by history key. Returns `None`
    /// if the subscriber hasn't seen a Telegram message on that thread.
    #[cfg(test)]
    pub(crate) fn reply_context(&self, history_key: &str) -> Option<ReplyContext> {
        self.reply_index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(history_key)
            .cloned()
    }

    /// For tests: directly seed a reply context, simulating an inbound
    /// `ChannelMessageReceived` for a Telegram message. Lets tests cover
    /// the `ApprovalRequested` branch without spinning up the runtime
    /// dispatch loop.
    #[cfg(test)]
    pub(crate) fn record_reply_context_for_test(&self, history_key: &str, ctx: ReplyContext) {
        self.reply_index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(history_key.to_string(), ctx);
    }

    fn record_reply_context(&self, sender: &str, reply_target: &str, thread_ts: Option<&str>) {
        let key = telegram_history_key(sender, reply_target);
        let ctx = ReplyContext {
            reply_target: reply_target.to_string(),
            thread_ts: thread_ts.map(str::to_string),
        };
        self.reply_index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(key, ctx);
    }

    async fn send_approval_prompt(
        &self,
        request_id: &str,
        tool_name: &str,
        action_summary: &str,
        thread_id: &str,
    ) {
        let reply_ctx = match self
            .reply_index
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(thread_id)
            .cloned()
        {
            Some(ctx) => ctx,
            None => {
                tracing::warn!(
                    "{LOG_PREFIX} no reply context recorded for thread_id={thread_id} \
                     (approval request_id={request_id} tool={tool_name}); cannot surface \
                     prompt — the parked turn will TTL-deny"
                );
                return;
            }
        };

        let channel = match self.channels_by_name.get(TELEGRAM_APPROVAL_CLIENT_ID) {
            Some(c) => Arc::clone(c),
            None => {
                tracing::warn!(
                    "{LOG_PREFIX} telegram channel not registered in runtime; \
                     dropping approval prompt for request_id={request_id}"
                );
                return;
            }
        };

        let body = format_approval_prompt(tool_name, action_summary);
        let send = SendMessage::new(body, &reply_ctx.reply_target).in_thread(reply_ctx.thread_ts);

        tracing::info!(
            "{LOG_PREFIX} surfacing approval prompt request_id={request_id} tool={tool_name} \
             thread_id={thread_id} reply_target={}",
            reply_ctx.reply_target
        );

        if let Err(err) = channel.send(&send).await {
            tracing::warn!(
                "{LOG_PREFIX} failed to send approval prompt request_id={request_id} \
                 tool={tool_name}: {err}"
            );
        }
    }
}

/// Render an approval request as a Telegram message body. Kept as a
/// free function so tests can pin the exact wording without going
/// through a real channel.
pub(crate) fn format_approval_prompt(tool_name: &str, action_summary: &str) -> String {
    format!(
        "🔐 Approval needed\nTool: `{tool_name}`\nAction: {action_summary}\n\nReply `yes` to approve or `no` to deny."
    )
}

#[async_trait]
impl EventHandler for TelegramApprovalSurfaceSubscriber {
    fn name(&self) -> &str {
        "telegram::approval_surface"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["channel", "approval"])
    }

    async fn handle(&self, event: &DomainEvent) {
        match event {
            DomainEvent::ChannelMessageReceived {
                channel,
                sender,
                reply_target,
                thread_ts,
                ..
            } if channel == TELEGRAM_APPROVAL_CLIENT_ID => {
                self.record_reply_context(sender, reply_target, thread_ts.as_deref());
            }
            DomainEvent::ApprovalRequested {
                request_id,
                tool_name,
                action_summary,
                thread_id,
                client_id,
                ..
            } => {
                let Some(client) = client_id.as_deref() else {
                    return;
                };
                if client != TELEGRAM_APPROVAL_CLIENT_ID {
                    return;
                }
                let Some(thread_id) = thread_id.as_deref() else {
                    tracing::warn!(
                        "{LOG_PREFIX} approval request_id={request_id} tool={tool_name} \
                         has client_id=telegram but no thread_id — cannot route"
                    );
                    return;
                };
                self.send_approval_prompt(request_id, tool_name, action_summary, thread_id)
                    .await;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
#[path = "approval_surface_tests.rs"]
mod tests;
