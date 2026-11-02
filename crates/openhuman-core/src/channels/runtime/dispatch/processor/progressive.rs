//! Progress projection onto the compiled TinyChannels progressive engine.

use crate::channels::bus::NativeProgressiveReply;
use std::sync::Arc;
use tinyagents_harness::progress::AgentProgress;
use tinychannels_bus::module_progressive::ChannelOutputEvent;
use tokio::sync::mpsc;

/// Captured configured module target for a single inbound conversation.
/// Opening the progressive handle is deferred until TurnRunner checkout, so a
/// queued follow-up gets a fresh module handle after it wins admission.
#[derive(Clone)]
pub(crate) struct NativeProgressiveTarget {
    kind: ProgressiveTargetKind,
    channel: tinychannels_bus::channel::ChannelRef,
    conversation: tinychannels_bus::channel::ConversationRef,
}

#[derive(Clone)]
enum ProgressiveTargetKind {
    Configured(tinybus::module::ModuleObjectClient),
    HostedRelay {
        client: tinybus::module::ModuleClient,
        channel: std::sync::Arc<crate::channels::providers::relay::RelayChannel>,
        ctx: crate::core::runtime::DispatchCtx,
        config: crate::config::Config,
        client_id: String,
        thread_id: String,
        request_id: String,
    },
}

impl NativeProgressiveTarget {
    pub(crate) async fn open(
        &self,
        turn_id: &str,
    ) -> anyhow::Result<std::sync::Arc<NativeProgressiveReply>> {
        tracing::debug!(
            provider_matches_account = self.channel.account_id.as_deref().is_some_and(|account| {
                tinychannels_bus::provider_id(account) == self.channel.id
            }),
            account_present = self
                .channel
                .account_id
                .as_deref()
                .is_some_and(|account| !account.trim().is_empty()),
            conversation_present = !self.conversation.id.trim().is_empty(),
            turn_present = !turn_id.trim().is_empty(),
            "opening native channel progressive turn"
        );
        let lifetime = std::time::Duration::from_secs(3_660);
        let reply = match &self.kind {
            ProgressiveTargetKind::Configured(object) => {
                NativeProgressiveReply::open(
                    object.clone(),
                    self.channel.clone(),
                    self.conversation.clone(),
                    turn_id.to_owned(),
                    lifetime,
                )
                .await?
            }
            ProgressiveTargetKind::HostedRelay {
                client,
                channel,
                ctx,
                config,
                client_id,
                thread_id,
                request_id,
            } => {
                let projection = Arc::new(
                    crate::web_chat::Surface::new(
                        ctx,
                        crate::web_chat::TurnIds {
                            client_id: client_id.clone(),
                            thread_id: thread_id.clone(),
                            request_id: request_id.clone(),
                        },
                        config.clone(),
                    )
                    .await,
                );
                let callback =
                    crate::channels::providers::relay::progressive::RelayProgressiveTransport::new(
                        ctx.clone(),
                        Arc::clone(channel),
                        self.channel.clone(),
                        self.conversation.clone(),
                        turn_id.to_owned(),
                        projection,
                    );
                let runtime = crate::modules::channel_runtime::ChannelRuntimeLease::open(
                    client.clone(),
                    callback,
                )
                .await?;
                NativeProgressiveReply::open_hosted(
                    runtime,
                    self.channel.clone(),
                    self.conversation.clone(),
                    turn_id.to_owned(),
                    lifetime,
                )
                .await?
            }
        };
        Ok(std::sync::Arc::new(reply))
    }
}

pub(crate) fn hosted_relay_target(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &crate::config::Config,
    channel: std::sync::Arc<crate::channels::providers::relay::RelayChannel>,
    channel_ref: tinychannels_bus::channel::ChannelRef,
    conversation: tinychannels_bus::channel::ConversationRef,
    client_id: String,
    thread_id: String,
    request_id: String,
) -> NativeProgressiveTarget {
    NativeProgressiveTarget {
        kind: ProgressiveTargetKind::HostedRelay {
            client: crate::modules::client(config.clone()),
            channel,
            ctx: ctx.clone(),
            config: config.clone(),
            client_id,
            thread_id,
            request_id,
        },
        channel: channel_ref,
        conversation,
    }
}

pub(crate) fn native_target(
    ctx: &crate::channels::context::ChannelRuntimeContext,
    envelope: &tinychannels_bus::channel::ChannelInboundEnvelope,
) -> anyhow::Result<Option<NativeProgressiveTarget>> {
    #[cfg(test)]
    if ctx
        .channels_by_name
        .get(&envelope.channel.id)
        .is_some_and(|handle| {
            matches!(
                handle.as_ref(),
                crate::channels::runtime::ChannelHandle::Test(_)
            )
        })
    {
        return Ok(None);
    }
    #[cfg(test)]
    if ctx.config.is_none() && ctx.module_objects.is_empty() {
        return Ok(None);
    }
    let account = envelope
        .channel
        .account_id
        .as_deref()
        .filter(|account| tinychannels_bus::provider_id(account) == envelope.channel.id)
        .ok_or_else(|| anyhow::anyhow!("native channel has no configured account identity"))?;
    let object = ctx
        .module_objects
        .get(account)
        .cloned()
        .ok_or_else(|| anyhow::anyhow!("native channel account is not configured"))?;
    tracing::debug!(
        provider_matches_account = tinychannels_bus::provider_id(account) == envelope.channel.id,
        account_present = !account.trim().is_empty(),
        conversation_present = !envelope.conversation.id.trim().is_empty(),
        message_present = !envelope.message_id.trim().is_empty(),
        "native channel progressive target resolved"
    );
    Ok(Some(NativeProgressiveTarget {
        kind: ProgressiveTargetKind::Configured(object),
        channel: envelope.channel.clone(),
        conversation: envelope.conversation.clone(),
    }))
}

pub(crate) async fn finish_native_reply(
    reply: &Option<std::sync::Arc<NativeProgressiveReply>>,
    text: &str,
) -> bool {
    let Some(reply) = reply else {
        return false;
    };
    let delivered = match reply.finish(text).await {
        Ok(result) if delivery_succeeded(&result) => true,
        Ok(result) => {
            tracing::error!(
                outcomes = delivery_outcome_count(&result),
                unsupported_parts = delivery_unsupported_count(&result),
                "[dispatch] native progressive final delivery was incomplete"
            );
            false
        }
        Err(error) => {
            tracing::error!("[dispatch] native progressive final delivery failed: {error}");
            if let Err(close_error) = reply.close().await {
                tracing::warn!("[dispatch] native progressive close failed: {close_error}");
            }
            false
        }
    };
    delivered
}

pub(super) async fn close_native_reply(reply: &Option<std::sync::Arc<NativeProgressiveReply>>) {
    if let Some(reply) = reply {
        if let Err(error) = reply.close().await {
            tracing::warn!("[dispatch] native progressive close failed: {error}");
        }
    }
}

pub(super) fn spawn_native_progress(
    mut events: mpsc::Receiver<AgentProgress>,
    reply: std::sync::Arc<NativeProgressiveReply>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let mut failed = false;
        while let Some(progress) = events.recv().await {
            let Some(event) = output_event(progress) else {
                continue;
            };
            match reply.feed(vec![event]).await {
                Ok(result) if delivery_succeeded(&result) => {}
                Ok(result) => {
                    tracing::warn!(
                        outcomes = delivery_outcome_count(&result),
                        unsupported_parts = delivery_unsupported_count(&result),
                        "[dispatch] progressive channel update was incomplete"
                    );
                    failed = true;
                    break;
                }
                Err(error) => {
                    tracing::warn!("[dispatch] progressive channel update failed: {error}");
                    failed = true;
                    break;
                }
            }
        }
        if failed {
            if let Err(error) = reply.close().await {
                tracing::warn!("[dispatch] native progressive close failed: {error}");
            }
        }
    })
}

pub(crate) fn delivery_succeeded(
    result: &tinychannels_bus::module_progressive::ProgressiveResult,
) -> bool {
    use tinychannels_bus::module_progressive::ProgressiveResult;
    match result {
        ProgressiveResult::Updated => true,
        ProgressiveResult::DeliveryOutcome { outcome } => outcome.error.is_none(),
        ProgressiveResult::DeliveryOutcomes { outcomes } => {
            outcomes.iter().all(|outcome| outcome.error.is_none())
        }
        ProgressiveResult::DeliveryBatch {
            outcomes,
            unsupported,
            failure,
            ..
        } => {
            outcomes.iter().all(|outcome| outcome.error.is_none())
                && unsupported.is_empty()
                && failure.is_none()
        }
        ProgressiveResult::DeliveryUnsupported { parts } => parts.is_empty(),
        _ => false,
    }
}

fn delivery_outcome_count(
    result: &tinychannels_bus::module_progressive::ProgressiveResult,
) -> usize {
    use tinychannels_bus::module_progressive::ProgressiveResult;
    match result {
        ProgressiveResult::DeliveryOutcome { .. } => 1,
        ProgressiveResult::DeliveryOutcomes { outcomes }
        | ProgressiveResult::DeliveryBatch { outcomes, .. } => outcomes.len(),
        _ => 0,
    }
}

fn delivery_unsupported_count(
    result: &tinychannels_bus::module_progressive::ProgressiveResult,
) -> usize {
    use tinychannels_bus::module_progressive::ProgressiveResult;
    match result {
        ProgressiveResult::DeliveryUnsupported { parts }
        | ProgressiveResult::DeliveryBatch {
            unsupported: parts, ..
        } => parts.len(),
        _ => 0,
    }
}

pub(crate) fn output_event(progress: AgentProgress) -> Option<ChannelOutputEvent> {
    crate::agent::turn_progress::turn_event(progress)
}

#[cfg(test)]
#[path = "progressive_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "progressive_flow_tests.rs"]
mod flow_tests;
