//! Channel startup wiring.

mod callback;
mod chat_workload;
mod credentials;
#[cfg(feature = "legacy-import")]
mod delivery_import;
#[cfg(not(feature = "legacy-import"))]
mod delivery_import {
    use std::path::Path;

    pub(super) async fn import_legacy_delivery(
        _ctx: &crate::core::runtime::DispatchCtx,
        _workspace: &Path,
    ) -> anyhow::Result<()> {
        anyhow::bail!(
            "legacy delivery import is unavailable without `legacy-import`; legacy data was not inspected or migrated"
        )
    }
}
mod module_providers;
mod progressive_transport;
mod prompt;
mod start_channels;
mod turn_parts;

use anyhow::Result;
use tinybus::module::{ModuleCallError, ModuleClient};

#[cfg(test)]
type BeginTypingRefresh = std::sync::Arc<
    dyn Fn(String, u64) -> futures::future::BoxFuture<'static, anyhow::Result<Option<String>>>
        + Send
        + Sync,
>;
#[cfg(test)]
type EndTypingRefresh = std::sync::Arc<
    dyn Fn(String) -> futures::future::BoxFuture<'static, anyhow::Result<()>> + Send + Sync,
>;

/// Host-owned handle to a configured channel instance. Production operations
/// are dispatched through the loaded TinyChannels module's bus object.
#[derive(Clone)]
pub(crate) enum ChannelHandle {
    Module(std::sync::Arc<module_providers::ModuleChannel>),
    Relay(std::sync::Arc<crate::channels::providers::relay::RelayChannel>),
    /// Test-only sink for narrow policy tests that do not need the linked module.
    #[cfg(test)]
    Test(std::sync::Arc<TestChannel>),
}

#[cfg(test)]
pub(crate) struct TestChannel {
    pub(crate) name: String,
    pub(crate) proactive_target: Option<String>,
    pub(crate) supports_reactions: bool,
    pub(crate) sent: std::sync::Arc<std::sync::Mutex<Vec<tinychannels_bus::SendMessage>>>,
    pub(crate) sent_messages: tokio::sync::Mutex<Vec<String>>,
    pub(crate) send_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) outbound_intent_count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) start_typing_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) stop_typing_calls: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    pub(crate) send_error: Option<String>,
    pub(crate) begin_typing_refresh: Option<BeginTypingRefresh>,
    pub(crate) end_typing_refresh: Option<EndTypingRefresh>,
}

impl ChannelHandle {
    #[cfg(test)]
    pub(crate) fn test(channel: TestChannel) -> Self {
        Self::Test(std::sync::Arc::new(channel))
    }

    pub(crate) fn name(&self) -> &str {
        match self {
            Self::Module(channel) => &channel.name,
            Self::Relay(channel) => channel.name(),
            #[cfg(test)]
            Self::Test(channel) => &channel.name,
        }
    }

    pub(crate) fn proactive_target(&self) -> Option<String> {
        match self {
            Self::Module(channel) => channel.proactive_target(),
            Self::Relay(_) => None,
            #[cfg(test)]
            Self::Test(channel) => channel.proactive_target.clone(),
        }
    }

    pub(crate) fn capabilities(&self) -> tinychannels_bus::ChannelCapabilities {
        match self {
            Self::Module(channel) => channel.capabilities(),
            Self::Relay(_) => tinychannels_bus::ChannelCapabilities::unknown(),
            #[cfg(test)]
            Self::Test(channel) => {
                // A test channel stands in for a chat provider: the module only
                // parses remote commands for channels that advertise them.
                let mut capabilities = tinychannels_bus::ChannelCapabilities::unknown();
                capabilities.remote_control = true;
                capabilities.chat_approvals = true;
                capabilities.history_key_ignores_thread =
                    tinychannels_bus::provider_id(&channel.name) == "telegram";
                capabilities.reactions = channel.supports_reactions;
                capabilities
            }
        }
    }

    pub(crate) fn supports_reactions(&self) -> bool {
        match self {
            Self::Module(channel) => channel.interaction.supports_reactions,
            Self::Relay(_) => false,
            #[cfg(test)]
            Self::Test(channel) => channel.supports_reactions,
        }
    }

    pub(crate) async fn send(&self, message: &tinychannels_bus::SendMessage) -> anyhow::Result<()> {
        match self {
            Self::Module(channel) => channel.send_prepared(message).await,
            Self::Relay(channel) => channel.send(message).await,
            #[cfg(test)]
            Self::Test(channel) => channel.send(message).await,
        }
    }

    pub(crate) async fn send_with_outbound_intent(
        &self,
        message: &tinychannels_bus::SendMessage,
    ) -> anyhow::Result<()> {
        match self {
            Self::Module(channel) => channel.send_prepared(message).await,
            Self::Relay(channel) => channel.send(message).await,
            #[cfg(test)]
            Self::Test(channel) => {
                channel
                    .outbound_intent_count
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                channel.send(message).await
            }
        }
    }

    pub(crate) async fn start_typing(&self, recipient: &str) -> anyhow::Result<()> {
        match self {
            // Factory-built adapters serve `StartTyping` through their typing
            // port, so the host drives the indicator again.
            Self::Module(module) => module.start_typing(recipient).await,
            Self::Relay(_) => Ok(()),
            #[cfg(test)]
            Self::Test(channel) => {
                let _ = recipient;
                channel
                    .start_typing_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
    }

    pub(crate) async fn stop_typing(&self, recipient: &str) -> anyhow::Result<()> {
        match self {
            Self::Module(module) => module.stop_typing(recipient).await,
            Self::Relay(_) => Ok(()),
            #[cfg(test)]
            Self::Test(channel) => {
                let _ = recipient;
                channel
                    .stop_typing_calls
                    .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                Ok(())
            }
        }
    }

    pub(crate) async fn begin_typing_refresh(
        &self,
        recipient: &str,
        interval_secs: u64,
    ) -> anyhow::Result<Option<String>> {
        match self {
            Self::Module(module) => module.begin_typing_refresh(recipient, interval_secs).await,
            Self::Relay(_) => Ok(None),
            #[cfg(test)]
            Self::Test(channel) => match &channel.begin_typing_refresh {
                Some(begin) => begin(recipient.to_owned(), interval_secs).await,
                None => Ok(None),
            },
        }
    }

    pub(crate) async fn end_typing_refresh(&self, handle: &str) -> anyhow::Result<()> {
        match self {
            Self::Module(module) => module.end_typing_refresh(handle).await,
            Self::Relay(_) => Ok(()),
            #[cfg(test)]
            Self::Test(channel) => match &channel.end_typing_refresh {
                Some(end) => end(handle.to_owned()).await,
                None => Ok(()),
            },
        }
    }
}

#[cfg(test)]
impl TestChannel {
    pub(crate) fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            proactive_target: None,
            supports_reactions: false,
            sent: std::sync::Arc::new(std::sync::Mutex::new(Vec::new())),
            sent_messages: tokio::sync::Mutex::new(Vec::new()),
            send_count: Default::default(),
            outbound_intent_count: Default::default(),
            start_typing_calls: Default::default(),
            stop_typing_calls: Default::default(),
            send_error: None,
            begin_typing_refresh: None,
            end_typing_refresh: None,
        }
    }

    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    async fn send(&self, message: &tinychannels_bus::SendMessage) -> anyhow::Result<()> {
        self.send_count
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Some(error) = &self.send_error {
            anyhow::bail!(error.clone());
        }
        self.sent
            .lock()
            .expect("test send lock")
            .push(message.clone());
        self.sent_messages
            .lock()
            .await
            .push(format!("{}:{}", message.recipient, message.content));
        Ok(())
    }
}

#[cfg(test)]
impl Default for TestChannel {
    fn default() -> Self {
        Self::new("test-channel")
    }
}

/// Ask the owning module to prepare effective execution limits for this host
/// runtime. A malformed response fails closed and reports only the module ID.
pub(crate) async fn prepare_runtime_limits(
    client: &ModuleClient,
    timeout_secs: u64,
    listener_count: usize,
) -> Result<(u64, usize)> {
    use tinychannels_bus::module_control::{ChannelControlRequest, ChannelControlResult};

    let result: ChannelControlResult = client
        .call_confidential(
            "tinychannels",
            tinychannels_bus::methods::PREPARE_CHANNEL_CONTROL,
            (ChannelControlRequest::RuntimeLimits {
                timeout_secs,
                listener_count,
            },),
        )
        .await?;
    match result {
        ChannelControlResult::RuntimeLimits {
            timeout_secs,
            max_in_flight,
        } if timeout_secs >= tinychannels_bus::module_runtime::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
            && max_in_flight > 0 =>
        {
            Ok((timeout_secs, max_in_flight))
        }
        _ => {
            crate::modules::report::malformed_reply("tinychannels");
            Err(anyhow::Error::new(ModuleCallError::ModuleFault)
                .context("channel runtime returned invalid runtime limits"))
        }
    }
}

pub use start_channels::start_channels;
pub(crate) use start_channels::start_channels_with_session;
pub(crate) use turn_parts::{build_channel_turn_parts, runtime_context, PromptToolDescs};

/// Owns a real module-backed channel runtime for channel policy tests.
///
/// The module remains leased until the guard is dropped, so calls made through
/// its channel handles exercise the same TinyBus adapter used by production.
#[cfg(test)]
pub(crate) struct ModuleRuntimeTestGuard(module_providers::ModuleChannelRuntime);

#[cfg(test)]
impl ModuleRuntimeTestGuard {
    pub(crate) fn channels(
        &self,
    ) -> std::sync::Arc<std::collections::HashMap<String, std::sync::Arc<ChannelHandle>>> {
        self.0.channels()
    }

    pub(crate) fn module_objects(
        &self,
    ) -> std::sync::Arc<std::collections::HashMap<String, tinybus::module::ModuleObjectClient>>
    {
        self.0.module_objects()
    }

    pub(crate) async fn start(&mut self, config: &Config) -> Result<()> {
        self.0.start(config).await
    }

    pub(crate) async fn close(&mut self) -> Result<()> {
        self.0.close().await
    }
}

/// Prepare a linked TinyChannels runtime for a host policy test.
#[cfg(test)]
pub(crate) async fn prepare_module_runtime_for_test(
    config: std::sync::Arc<Config>,
    dispatch: tokio::sync::mpsc::Sender<crate::channels::runtime::dispatch::RuntimeChannelMessage>,
) -> Result<ModuleRuntimeTestGuard> {
    prepare_module_runtime_for_test_in_context(
        config,
        std::sync::Arc::new(std::sync::OnceLock::new()),
        dispatch,
    )
    .await
}

/// Prepare a linked TinyChannels runtime whose host callbacks resolve a live
/// runtime context after the caller installs it.
#[cfg(test)]
pub(crate) async fn prepare_module_runtime_for_test_in_context(
    config: std::sync::Arc<Config>,
    context: std::sync::Arc<
        std::sync::OnceLock<std::sync::Weak<crate::channels::context::ChannelRuntimeContext>>,
    >,
    dispatch: tokio::sync::mpsc::Sender<crate::channels::runtime::dispatch::RuntimeChannelMessage>,
) -> Result<ModuleRuntimeTestGuard> {
    let runtime = module_providers::ModuleChannelRuntime::prepare(
        &crate::core::runtime::DispatchCtx::host_default(),
        config,
        context,
        dispatch,
    )
    .await?;
    Ok(ModuleRuntimeTestGuard(runtime))
}

// Re-exported at module scope (rather than left as private `pub(super)`
// items reached only through their owning submodule) so the `#[path]` test
// modules below — which share this module's scope via `super::*` — can see
// them exactly as they could when this was one unsplit file.
#[cfg(test)]
use crate::config::Config;
#[cfg(test)]
use chat_workload::{resolve_chat_workload, ChatWorkloadResolution};
#[cfg(any(test, debug_assertions))]
use credentials::resolve_yuanbao_app_secret;
#[cfg(any(test, debug_assertions))]
#[path = "startup_test_support_tests.rs"]
pub mod test_support;

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "startup_yuanbao_secret_tests_tests.rs"]
mod yuanbao_secret_tests;

#[cfg(test)]
#[path = "startup_email_secret_tests_tests.rs"]
mod email_secret_tests;

#[cfg(all(test, feature = "modules"))]
#[path = "startup/runtime_limits_tests.rs"]
mod runtime_limits_tests;
