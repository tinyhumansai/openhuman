//! Transport for module-owned progressive replies: delivery goes through the
//! registered channel callback (`channels::callback_slot`).
use crate::modules::channel_runtime::ChannelRuntimeLease;
#[cfg(test)]
use serde_json::Value;
use tinybus::module::ModuleCallError;
use tinychannels_bus::module_progressive::{
    ProgressiveCommand, ProgressiveEvent, ProgressiveResult, ProgressiveTransportRequest,
    ProgressiveTransportResult,
};
use tinychannels_bus::{methods, ChannelBackendRequest, ChannelBackendResult};

fn request_channel(request: &ProgressiveTransportRequest) -> &str {
    match request {
        ProgressiveTransportRequest::Send { channel, .. }
        | ProgressiveTransportRequest::Edit { channel, .. }
        | ProgressiveTransportRequest::Delete { channel, .. }
        | ProgressiveTransportRequest::Typing { channel } => channel,
    }
}

/// The transport progressive delivery runs through: a registered channel
/// callback ([`crate::channels::callback_slot`]). Transport refusals are typed
/// facts the callback maps itself; recovery and retry budgets belong to the
/// module.
#[derive(Clone)]
struct SlotTransport {
    callback: crate::channels::callback_slot::ChannelCallback,
    config: crate::config::Config,
    channel: String,
    session: tokio_util::sync::CancellationToken,
}

#[tinybus::interface(name = "ai.tinyhumans.tinychannels.ChannelsHost")]
impl SlotTransport {
    async fn dispatch_controller_backend(
        &self,
        request: ChannelBackendRequest,
    ) -> tinybus::Result<ChannelBackendResult> {
        let ChannelBackendRequest::ProgressiveTransport(request) = request else {
            return Err(tinybus::Error::failed("callback capability unavailable"));
        };
        if request_channel(&request) != self.channel {
            return Ok(ChannelBackendResult::ProgressiveTransport(
                ProgressiveTransportResult::Failed,
            ));
        }
        let ctx = crate::channels::callback_slot::ChannelCallbackCtx {
            config: self.config.clone(),
            channel: Some(self.channel.clone()),
            session: self.session.clone(),
        };
        let call = (self.callback)(ctx, ChannelBackendRequest::ProgressiveTransport(request));
        tokio::select! {
            biased;
            _ = self.session.cancelled() => Ok(ChannelBackendResult::ProgressiveTransport(
                ProgressiveTransportResult::Unavailable,
            )),
            result = call => match result {
                Ok(result) => Ok(result),
                Err(error) => {
                    tracing::debug!(%error, "[channels][delivery] registered callback failed");
                    Ok(ChannelBackendResult::ProgressiveTransport(
                        ProgressiveTransportResult::Failed,
                    ))
                }
            },
        }
    }
}

/// The host projects ordered turn facts; the module owns timers and delivery state.
pub(super) struct ModuleProgressiveReply {
    runtime: ChannelRuntimeLease,
    handle: String,
}

impl ModuleProgressiveReply {
    pub(super) async fn open(
        config: crate::config::Config,
        channel: String,
        session: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Self> {
        let Some(callback) = crate::channels::callback_slot::current() else {
            tracing::debug!("[channels][delivery] no channel callback registered");
            anyhow::bail!(
                "{} no channel transport registered",
                crate::core::observability::BACKEND_UNAVAILABLE_PREFIX
            );
        };
        tracing::debug!("[channels][delivery] progressive delivery via the registered callback");
        anyhow::ensure!(!session.is_cancelled(), "Channel session closed");
        let transport = SlotTransport {
            callback,
            config: config.clone(),
            channel: channel.clone(),
            session,
        };
        Self::open_with(config, channel, transport).await
    }

    async fn open_with(
        config: crate::config::Config,
        channel: String,
        callback: impl tinybus::service::Interface + 'static,
    ) -> anyhow::Result<Self> {
        let runtime = ChannelRuntimeLease::open(crate::modules::client(config), callback).await?;
        let result: ProgressiveResult = runtime
            .object()
            .call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::Open { channel },),
            )
            .await?;
        match result {
            ProgressiveResult::Opened { handle } if !handle.is_empty() && handle.len() <= 128 => {
                Ok(Self { runtime, handle })
            }
            _ => {
                crate::modules::report::malformed_reply("tinychannels");
                Err(ModuleCallError::ModuleFault.into())
            }
        }
    }

    pub(super) async fn feed(&self, events: Vec<ProgressiveEvent>) -> Result<(), ModuleCallError> {
        let result: ProgressiveResult = self
            .runtime
            .object()
            .call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::Feed {
                    handle: self.handle.clone(),
                    events,
                },),
            )
            .await?;
        if result != ProgressiveResult::Updated {
            crate::modules::report::malformed_reply("tinychannels");
            return Err(ModuleCallError::ModuleFault);
        }
        Ok(())
    }

    pub(super) async fn finalize(&self, text: &str) -> Result<(), ModuleCallError> {
        let result: ProgressiveResult = self
            .runtime
            .object()
            .call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::Finish {
                    handle: self.handle.clone(),
                    text: text.into(),
                },),
            )
            .await?;
        if result != ProgressiveResult::Closed {
            crate::modules::report::malformed_reply("tinychannels");
            return Err(ModuleCallError::ModuleFault);
        }
        self.runtime.close().await
    }
}

// Compatibility fixtures exercise the owning library only in test builds.
#[cfg(test)]
pub(super) fn channel_message_body_with_idempotency(channel: &str, body: Value) -> Value {
    let intent = tinychannels::outbound_intent_from_legacy_message(channel, body);
    tinychannels::legacy_message_value_from_outbound_intent(&intent)
}

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
