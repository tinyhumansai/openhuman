//! Channel startup wiring.

mod callback;
mod chat_workload;
mod credentials;
mod delivery_import;
mod module_providers;
mod prompt;
#[cfg(test)]
mod relay;
mod start_channels;
mod turn_parts;

use anyhow::Result;
use tinybus::module::{ModuleCallError, ModuleClient};

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
        } if timeout_secs >= tinychannels_bus::context::MIN_CHANNEL_MESSAGE_TIMEOUT_SECS
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

pub(crate) use credentials::hydrate_channel_credentials;
pub use start_channels::start_channels;
pub(crate) use start_channels::start_channels_with_session;
pub(crate) use turn_parts::{build_channel_turn_parts, runtime_context, PromptToolDescs};

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
#[cfg(test)]
use relay::RelayInboundMessageHandler;

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
