//! Captured managed transport for module-owned progressive replies.
use crate::backend::{BackendApiError, BackendClient};
use crate::modules::channel_runtime::ChannelRuntimeLease;
use crate::security::credentials::session_support::{
    get_session_token, is_local_session_token, BackendCredential,
};
use serde_json::json;
#[cfg(test)]
use serde_json::Value;
use tinybus::module::ModuleCallError;
use tinychannels_bus::module_progressive::{
    ProgressiveCommand, ProgressiveEvent, ProgressiveResult, ProgressiveTransportRequest,
    ProgressiveTransportResult,
};
use tinychannels_bus::{methods, ChannelBackendRequest, ChannelBackendResult};

/// Transport refusals are typed facts; recovery and retry budgets belong to the module.
pub(super) fn map_backend_error(error: anyhow::Error) -> ProgressiveTransportResult {
    match error.downcast_ref::<BackendApiError>() {
        Some(BackendApiError::ChannelEditUnsupported { .. }) => {
            ProgressiveTransportResult::EditUnsupported
        }
        Some(BackendApiError::MessageNotFound { .. }) => ProgressiveTransportResult::MessageGone,
        _ => ProgressiveTransportResult::Failed,
    }
}

#[derive(Clone)]
struct ManagedTransport {
    client: BackendClient,
    credential: BackendCredential,
    channel: String,
    session: tokio_util::sync::CancellationToken,
}

impl ManagedTransport {
    fn capture(
        config: &crate::config::Config,
        channel: String,
        session: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(!session.is_cancelled(), "Channel session closed");
        let credential = match get_session_token(config)
            .map_err(|_| anyhow::anyhow!("Hosted channel credential unavailable"))?
        {
            Some(token) if !is_local_session_token(&token) => BackendCredential::Session(token),
            _ => anyhow::bail!("Hosted channel credential unavailable"),
        };
        Ok(Self {
            client: BackendClient::from_config(config)?,
            credential,
            channel,
            session,
        })
    }

    async fn execute(&self, request: ProgressiveTransportRequest) -> ProgressiveTransportResult {
        if request_channel(&request) != self.channel {
            return ProgressiveTransportResult::Failed;
        }
        tokio::select! {
            biased;
            _ = self.session.cancelled() => ProgressiveTransportResult::Unavailable,
            result = self.execute_transport(request) => result,
        }
    }

    async fn execute_transport(
        &self,
        request: ProgressiveTransportRequest,
    ) -> ProgressiveTransportResult {
        let result = match request {
            ProgressiveTransportRequest::Send { channel, message } => {
                self.client
                    .send_channel_message(&channel, &self.credential, message)
                    .await
            }
            ProgressiveTransportRequest::Edit {
                channel,
                message_id,
                text,
            } => {
                self.client
                    .send_channel_edit(
                        &channel,
                        &message_id,
                        &self.credential,
                        json!({"text":text}),
                    )
                    .await
            }
            ProgressiveTransportRequest::Delete {
                channel,
                message_id,
            } => {
                self.client
                    .send_channel_delete(&channel, &message_id, &self.credential)
                    .await
            }
            ProgressiveTransportRequest::Typing { channel } => {
                self.client
                    .send_channel_typing(&channel, &self.credential)
                    .await
            }
        };
        match result {
            Ok(value) => ProgressiveTransportResult::Success(value),
            Err(error) => map_backend_error(error),
        }
    }
}

fn request_channel(request: &ProgressiveTransportRequest) -> &str {
    match request {
        ProgressiveTransportRequest::Send { channel, .. }
        | ProgressiveTransportRequest::Edit { channel, .. }
        | ProgressiveTransportRequest::Delete { channel, .. }
        | ProgressiveTransportRequest::Typing { channel } => channel,
    }
}

#[tinybus::interface(name = "ai.tinyhumans.tinychannels.ChannelsHost")]
impl ManagedTransport {
    async fn dispatch_controller_backend(
        &self,
        request: ChannelBackendRequest,
    ) -> tinybus::Result<ChannelBackendResult> {
        let ChannelBackendRequest::ProgressiveTransport(request) = request else {
            return Err(tinybus::Error::failed("callback capability unavailable"));
        };
        Ok(ChannelBackendResult::ProgressiveTransport(
            self.execute(request).await,
        ))
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
        let callback = ManagedTransport::capture(&config, channel.clone(), session)?;
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
