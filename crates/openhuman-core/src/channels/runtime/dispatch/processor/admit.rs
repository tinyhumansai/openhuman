//! Admission of one inbound message: module preparation, the
//! `ChannelMessageReceived` event, and the runtime-command and approval-reply
//! intercepts that answer a message without an agent turn. Admission runs
//! before any debounce, so a command or an approval reply is never held behind
//! the quiet window of a running conversation.

use crate::channels::context::ChannelRuntimeContext;
use crate::channels::routes::handle_runtime_command_with_reset;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::util::truncate_with_ellipsis;
use std::sync::Arc;
use tinychannels_bus::{ChannelInboundEnvelope, ChannelMessage};

use super::approval::{channel_has_approval_surface, try_route_approval_reply};
use super::progressive::NativeProgressiveTarget;
use super::RuntimeChannelMessage;

/// A prepared inbound message that still needs an agent turn: not a runtime
/// command and not an approval reply.
pub(crate) struct AdmittedInbound {
    pub(crate) msg: ChannelMessage,
    pub(crate) envelope: ChannelInboundEnvelope,
    pub(crate) history_key: String,
    pub(crate) canonical_session_key: String,
    pub(crate) session_key: String,
    pub(crate) tenant: crate::core::runtime::Tenant,
    pub(crate) native_target: Option<Arc<NativeProgressiveTarget>>,
    pub(crate) typed_envelope_input: bool,
    pub(crate) imported_history: Vec<tinyagents_session::transcript::TranscriptMessage>,
    pub(crate) legacy_history: Option<crate::channels::providers::relay::LegacyHistoryImport>,
}

pub(super) fn channel_inbound_preparation_request(
    source: tinychannels_bus::module_inbound::ChannelInboundSource,
    preparation_mode: tinychannels_bus::module_inbound::ChannelInboundPreparationMode,
) -> tinychannels_bus::module_inbound::PrepareChannelInboundRequest {
    tinychannels_bus::module_inbound::PrepareChannelInboundRequest {
        source,
        namespace: "main".into(),
        session_policy: Default::default(),
        preparation_mode,
    }
}

pub(super) fn inbound_console_preview(envelope: &ChannelInboundEnvelope, text: &str) -> String {
    let contains_audio = envelope.parts.iter().any(|part| {
        matches!(
            part,
            tinychannels_bus::ContentPart::VoiceNote { .. }
                | tinychannels_bus::ContentPart::Audio { .. }
        )
    });
    if contains_audio {
        "[voice message]".to_owned()
    } else {
        truncate_with_ellipsis(text, 80)
    }
}

/// Prepare an inbound message, publish it, and answer it directly when it is
/// a runtime command or an approval reply. Returns the message when it still
/// needs an agent turn.
pub(crate) async fn admit_runtime_message(
    ctx: Arc<ChannelRuntimeContext>,
    runtime_msg: RuntimeChannelMessage,
) -> Option<AdmittedInbound> {
    use tinychannels_bus::module_inbound::ChannelInboundSource;
    let RuntimeChannelMessage {
        message,
        inbound_envelope,
        authenticated_relay,
        prepared_inbound,
        imported_history,
        legacy_history,
        progressive_target,
    } = runtime_msg;
    let typed_envelope_input = message.channel.is_empty()
        && inbound_envelope
            .as_ref()
            .is_some_and(super::native_media::requires_typed_preparation);
    let prepared = match prepared_inbound {
        Some(prepared) => prepared,
        None => {
            let source = if message.channel.is_empty() {
                let envelope = inbound_envelope?;
                ChannelInboundSource::Envelope {
                    envelope,
                    timestamp: 0,
                }
            } else {
                ChannelInboundSource::Legacy {
                    message,
                    envelope: inbound_envelope,
                }
            };
            match crate::modules::channel_controller::prepare_inbound(
                &crate::channels::context::module_client(&ctx).await,
                channel_inbound_preparation_request(
                    source,
                    if typed_envelope_input {
                        tinychannels_bus::module_inbound::ChannelInboundPreparationMode::Typed
                    } else {
                        tinychannels_bus::module_inbound::ChannelInboundPreparationMode::LegacyText
                    },
                ),
            )
            .await
            {
                Ok(prepared) => prepared,
                Err(error) => {
                    tracing::warn!("[dispatch] channel preparation failed: {error}");
                    return None;
                }
            }
        }
    };
    let msg = prepared.message;
    let history_key = prepared.history_key;
    let canonical_session_key = prepared.session_key.clone();
    let mut envelope = prepared.envelope;
    tracing::debug!(
        provider_matches_account = envelope
            .channel
            .account_id
            .as_deref()
            .is_some_and(|account| tinychannels_bus::provider_id(account) == envelope.channel.id),
        account_present = envelope
            .channel
            .account_id
            .as_deref()
            .is_some_and(|account| !account.trim().is_empty()),
        conversation_present = !envelope.conversation.id.trim().is_empty(),
        message_present = !envelope.message_id.trim().is_empty(),
        "prepared inbound scope for native channel turn"
    );
    envelope.access.delivered_via_upstream_relay = authenticated_relay;
    let remote_reset_scope = tinychannels_bus::module_remote::RemoteSessionScope {
        channel: envelope.channel.clone(),
        conversation: envelope.conversation.clone(),
        session_key: canonical_session_key.clone(),
    };
    let native_target = if let Some(target) = progressive_target {
        Some(target)
    } else {
        match super::progressive::native_target(&ctx, &envelope) {
            Ok(target) => target.map(Arc::new),
            Err(error) => {
                tracing::error!("[dispatch] native channel target is invalid: {error}");
                return None;
            }
        }
    };

    let tenant = match ctx.dispatch_ctx().tenant() {
        Ok(tenant) => tenant,
        Err(error) => {
            tracing::warn!("[dispatch] channel turn lost its tenant scope: {error}");
            return None;
        }
    };
    let session_key = crate::core::runtime::tenant::tenant_key(&tenant, &canonical_session_key);

    println!(
        "  💬 [{}] from {}: {}",
        msg.channel,
        msg.sender,
        inbound_console_preview(&envelope, &msg.content)
    );

    BUS.publish(DomainEvent::ChannelMessageReceived {
        channel: msg.channel.clone(),
        message_id: msg.id.clone(),
        sender: msg.sender.clone(),
        reply_target: msg.reply_target.clone(),
        content: msg.content.clone(),
        thread_ts: msg.thread_ts.clone(),
        inbound_envelope: Some(envelope.clone()),
        workspace_dir: ctx.workspace_dir.as_ref().clone(),
    });

    let target_channel = ctx.channels_by_name.get(&msg.channel).cloned();
    let reset_context = Arc::clone(&ctx);
    let reset_message = msg.clone();
    let reset_session_key = session_key.clone();
    let reset_history_key = history_key.clone();
    if handle_runtime_command_with_reset(
        ctx.as_ref(),
        &msg,
        target_channel.as_ref(),
        &history_key,
        Some(remote_reset_scope),
        Some(session_key.clone()),
        &msg,
        || async move {
            super::reset::reset_command(
                reset_context,
                reset_message,
                reset_session_key,
                reset_history_key,
            )
            .await
        },
    )
    .await
    {
        return None;
    }

    // Sub-issue 2 of #3098: if this channel has an approval surface and the
    // inbound message is a yes/no reply for a parked approval on this same
    // history key, route it to `ApprovalGate::decide` and return — running
    // a fresh agent turn would cancel the parked tool call. Any other text
    // falls through to the normal dispatch (the user is redirecting). Mirrors
    // the same intercept in `web_chat/`.
    if channel_has_approval_surface(&msg.channel)
        && try_route_approval_reply(&msg, &history_key).await
    {
        return None;
    }

    Some(AdmittedInbound {
        msg,
        envelope,
        history_key,
        canonical_session_key,
        session_key,
        tenant,
        native_target,
        typed_envelope_input,
        imported_history,
        legacy_history,
    })
}
