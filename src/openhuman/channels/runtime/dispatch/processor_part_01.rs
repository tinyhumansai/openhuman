use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::messages::ChatMessage;
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::channels::context::{
    build_memory_context, compact_sender_history, conversation_history_key,
    conversation_memory_key, is_context_window_overflow_error, ChannelRuntimeContext,
};
use crate::openhuman::channels::providers::telegram::TELEGRAM_APPROVAL_CLIENT_ID;
use crate::openhuman::channels::routes::{
    get_or_create_turn_model_source, get_route_selection, handle_runtime_command_if_needed,
};
use crate::openhuman::channels::traits;
use crate::openhuman::channels::{ChannelSendExt, SendMessage};
use crate::openhuman::inference::provider;
use crate::openhuman::util::truncate_with_ellipsis;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tinybus::NativeRequestError;
use tinymemory_api::provider::MemoryCore as _;
use tokio_util::sync::CancellationToken;

use super::helpers::{
    build_channel_context_block, log_worker_join_result, select_acknowledgment_reaction,
    spawn_scoped_typing_task, REPLY_LOG_TRUNCATE_CHARS,
};
use super::routing::resolve_target_agent;

#[derive(Debug, Clone)]
pub(crate) struct RuntimeChannelMessage {
    pub(crate) message: traits::ChannelMessage,
    pub(crate) inbound_envelope: Option<tinychannels::ChannelInboundEnvelope>,
}

impl RuntimeChannelMessage {
    pub(crate) fn with_inbound_envelope(
        message: traits::ChannelMessage,
        inbound_envelope: tinychannels::ChannelInboundEnvelope,
    ) -> Self {
        Self {
            message,
            inbound_envelope: Some(inbound_envelope),
        }
    }
}

impl From<traits::ChannelMessage> for RuntimeChannelMessage {
    fn from(message: traits::ChannelMessage) -> Self {
        Self {
            message,
            inbound_envelope: None,
        }
    }
}

/// Whether a channel currently has a registered approval surface — i.e.
/// a subscriber that turns `ApprovalRequested` events into chat messages
/// and a way for the user's reply to flow back into the
/// [`ApprovalGate`]. When `true`, the dispatch loop scopes the agent
/// turn in an [`ApprovalChatContext`] so the gate actually fires for
/// `Prompt`-class tools and intercepts yes/no replies for parked
/// approvals.
///
/// Only Telegram has a surface today (sub-issue 2 of #3098). Discord /
/// Slack / iMessage / Mattermost remain in the legacy "no chat context
/// → silently allow" state until each gets a per-channel surface in a
/// follow-up PR; surfacing approvals there without a subscriber would
/// just TTL-deny every parked call, which is worse than the status quo.
///
/// [`ApprovalChatContext`]: crate::openhuman::security::approval::ApprovalChatContext
/// [`ApprovalGate`]: crate::openhuman::security::approval::ApprovalGate
pub(crate) fn channel_has_approval_surface(channel: &str) -> bool {
    channel == TELEGRAM_APPROVAL_CLIENT_ID
}

/// If the inbound message is a yes/no reply for a parked approval on
/// this thread, route it to [`ApprovalGate::decide`] and return `true`.
/// Otherwise return `false` so the caller can dispatch the message as a
/// fresh turn (which intentionally cancels any parked approval — the
/// user is redirecting). Mirrors the web channel intercept at
/// `web_chat/`.
///
/// [`ApprovalGate::decide`]: crate::openhuman::security::approval::ApprovalGate::decide
async fn try_route_approval_reply(msg: &traits::ChannelMessage) -> bool {
    let Some(gate) = crate::openhuman::security::approval::ApprovalGate::try_global() else {
        return false;
    };
    let thread_id = conversation_history_key(msg);
    let Some(request_id) = gate.pending_for_thread(&thread_id) else {
        return false;
    };
    let Some(decision) = crate::openhuman::security::approval::parse_approval_reply(&msg.content)
    else {
        return false;
    };
    match gate.decide(&request_id, decision) {
        Ok(Some(_)) => {
            tracing::info!(
                "[dispatch] routed chat reply to approval gate channel={} thread_id={} request_id={} decision={}",
                msg.channel,
                thread_id,
                request_id,
                decision.as_str()
            );
            true
        }
        Ok(None) => {
            // The request was already decided / cleared between our
            // `pending_for_thread` check and `decide`. Don't claim the
            // intercept; fall through so the reply lands as a normal turn.
            tracing::warn!(
                "[dispatch] approval reply targeted a non-pending request channel={} thread_id={} request_id={} — dispatching as fresh turn",
                msg.channel,
                thread_id,
                request_id
            );
            false
        }
        Err(err) => {
            tracing::warn!(
                "[dispatch] approval gate decide failed channel={} thread_id={} request_id={}: {err}",
                msg.channel,
                thread_id,
                request_id
            );
            false
        }
    }
}

pub(crate) async fn process_channel_message(
    ctx: Arc<ChannelRuntimeContext>,
    msg: traits::ChannelMessage,
) {
    process_channel_runtime_message(ctx, RuntimeChannelMessage::from(msg)).await;
}
