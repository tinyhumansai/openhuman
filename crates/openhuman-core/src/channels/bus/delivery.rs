//! Transport for module-owned progressive replies: delivery goes through the
//! registered channel callback (`channels::callback_slot`).
use crate::modules::channel_runtime::ChannelRuntimeLease;
use tinybus::module::ModuleCallError;
use tinychannels_bus::module_progressive::{
    ChannelOutputEvent, ProgressiveCommand, ProgressiveEvent, ProgressiveResult,
    ProgressiveTransportRequest, ProgressiveTransportResult,
};
use tinychannels_bus::{methods, ChannelBackendRequest, ChannelBackendResult};

fn request_channel(request: &ProgressiveTransportRequest) -> &str {
    match request {
        ProgressiveTransportRequest::Send { channel, .. }
        | ProgressiveTransportRequest::Edit { channel, .. }
        | ProgressiveTransportRequest::Delete { channel, .. }
        | ProgressiveTransportRequest::Typing { channel, .. } => channel,
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
            return Ok(ChannelBackendResult::ProgressiveTransport(Box::new(
                ProgressiveTransportResult::Failed,
            )));
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
                Box::new(ProgressiveTransportResult::Unavailable),
            )),
            result = call => match result {
                Ok(result) => Ok(result),
                Err(error) => {
                    tracing::debug!(%error, "[channels][delivery] registered callback failed");
                    Ok(ChannelBackendResult::ProgressiveTransport(Box::new(
                        ProgressiveTransportResult::Failed,
                    )))
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

/// Native listener output handle on the already-running compiled module
/// object. Its callback is the captured configured provider adapter, so native
/// output never falls back to the hosted REST transport.
pub(crate) struct NativeProgressiveReply {
    object: tinybus::module::ModuleObjectClient,
    handle: String,
    // Hosted relay handles own their callback runtime. Keep that lease alive
    // until Close is acknowledged or the final reply owner is dropped.
    runtime: Option<ChannelRuntimeLease>,
}

impl NativeProgressiveReply {
    pub(crate) async fn open(
        object: tinybus::module::ModuleObjectClient,
        channel: tinychannels_bus::channel::ChannelRef,
        conversation: tinychannels_bus::channel::ConversationRef,
        turn_id: String,
        lifetime: std::time::Duration,
    ) -> anyhow::Result<Self> {
        let result: ProgressiveResult = object
            .call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::OpenOutput {
                    channel,
                    conversation,
                    turn_id,
                    lifetime_ms: lifetime.as_millis().min(u64::MAX as u128) as u64,
                },),
            )
            .await?;
        match result {
            ProgressiveResult::Opened { handle } if !handle.is_empty() && handle.len() <= 128 => {
                Ok(Self {
                    object,
                    handle,
                    runtime: None,
                })
            }
            _ => {
                crate::modules::report::malformed_reply("tinychannels");
                Err(ModuleCallError::ModuleFault.into())
            }
        }
    }

    pub(crate) async fn open_hosted(
        runtime: ChannelRuntimeLease,
        channel: tinychannels_bus::channel::ChannelRef,
        conversation: tinychannels_bus::channel::ConversationRef,
        turn_id: String,
        lifetime: std::time::Duration,
    ) -> anyhow::Result<Self> {
        let object = runtime.object().clone();
        let result: ProgressiveResult = object
            .call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::OpenHostedOutput {
                    channel,
                    conversation,
                    turn_id,
                    lifetime_ms: lifetime.as_millis().min(u64::MAX as u128) as u64,
                },),
            )
            .await?;
        match result {
            ProgressiveResult::Opened { handle } if !handle.is_empty() && handle.len() <= 128 => {
                Ok(Self {
                    object,
                    handle,
                    runtime: Some(runtime),
                })
            }
            _ => {
                crate::modules::report::malformed_reply("tinychannels");
                Err(ModuleCallError::ModuleFault.into())
            }
        }
    }

    pub(crate) async fn feed(
        &self,
        events: Vec<ChannelOutputEvent>,
    ) -> Result<ProgressiveResult, ModuleCallError> {
        let result: ProgressiveResult = self
            .object
            .call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::FeedOutput {
                    handle: self.handle.clone(),
                    turn_id: None,
                    channel_ref: None,
                    conversation: None,
                    sequence: None,
                    events,
                },),
            )
            .await?;
        if !matches!(
            result,
            ProgressiveResult::Updated
                | ProgressiveResult::DeliveryOutcome { .. }
                | ProgressiveResult::DeliveryOutcomes { .. }
                | ProgressiveResult::DeliveryUnsupported { .. }
                | ProgressiveResult::DeliveryBatch { .. }
        ) {
            crate::modules::report::malformed_reply("tinychannels");
            return Err(ModuleCallError::ModuleFault);
        }
        Ok(result)
    }

    /// Deliver the canonical final event and return every receipt/failure fact.
    pub(crate) async fn finish(&self, text: &str) -> Result<ProgressiveResult, ModuleCallError> {
        let result: ProgressiveResult = self
            .object
            .call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::FeedOutput {
                    handle: self.handle.clone(),
                    turn_id: None,
                    channel_ref: None,
                    conversation: None,
                    sequence: None,
                    events: vec![ChannelOutputEvent::FinalMessage {
                        text: text.to_owned(),
                        parts: Vec::new(),
                        receipt: None,
                    }],
                },),
            )
            .await?;
        if !matches!(
            result,
            ProgressiveResult::DeliveryOutcome { .. }
                | ProgressiveResult::DeliveryOutcomes { .. }
                | ProgressiveResult::DeliveryUnsupported { .. }
                | ProgressiveResult::DeliveryBatch { .. }
        ) {
            crate::modules::report::malformed_reply("tinychannels");
            return Err(ModuleCallError::ModuleFault);
        }
        Ok(result)
    }

    /// Abort an unfinished reply after cancellation or a failed progress call.
    pub(crate) async fn close(&self) -> Result<(), ModuleCallError> {
        let result: ProgressiveResult = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            self.object.call_confidential(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::Close {
                    handle: self.handle.clone(),
                },),
            ),
        )
        .await
        .map_err(|_| ModuleCallError::ModuleFault)??;
        if result != ProgressiveResult::Closed {
            crate::modules::report::malformed_reply("tinychannels");
            return Err(ModuleCallError::ModuleFault);
        }
        if let Some(runtime) = &self.runtime {
            runtime.close().await?;
        }
        Ok(())
    }
}

impl Drop for NativeProgressiveReply {
    fn drop(&mut self) {
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let object = self.object.clone();
        let handle = self.handle.clone();
        runtime.spawn(async move {
            let close = object.call_confidential::<ProgressiveResult>(
                methods::EXECUTE_PROGRESSIVE,
                (ProgressiveCommand::Close { handle },),
            );
            match tokio::time::timeout(std::time::Duration::from_secs(5), close).await {
                Ok(Ok(ProgressiveResult::Closed)) => {}
                Ok(Ok(_)) => crate::modules::report::malformed_reply("tinychannels"),
                Ok(Err(_)) | Err(_) => {
                    tracing::debug!("[channels] native progressive cleanup did not complete")
                }
            }
        });
    }
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

#[cfg(test)]
#[path = "delivery_tests.rs"]
mod tests;
