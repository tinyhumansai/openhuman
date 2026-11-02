//! Channel runtime entry points.

#[cfg(feature = "channels")]
pub(crate) mod dispatch;
#[cfg(feature = "channels")]
pub(crate) mod session;
#[cfg(feature = "channels")]
mod startup;
pub(crate) mod turn_driver;

#[cfg(all(test, feature = "channels"))]
pub(crate) use startup::prepare_module_runtime_for_test_in_context;
#[cfg(feature = "channels")]
pub(crate) use startup::prepare_runtime_limits;
#[cfg(feature = "channels")]
pub use startup::start_channels;
#[cfg(feature = "channels")]
pub(crate) use startup::start_channels_with_session;
#[cfg(feature = "channels")]
pub(crate) use startup::ChannelHandle;
#[cfg(all(test, feature = "channels"))]
pub(crate) use startup::TestChannel;
#[cfg(all(test, feature = "channels"))]
pub(crate) use startup::{prepare_module_runtime_for_test, ModuleRuntimeTestGuard};
// The hosted-channel relay (`channels::providers::relay`) runs one message
// through the same pipeline, with a context built from the caller's config.
#[cfg(all(test, feature = "channels"))]
pub(crate) use dispatch::process_channel_message;
#[cfg(feature = "channels")]
pub(crate) use dispatch::{process_channel_runtime_message, RuntimeChannelMessage};
#[cfg(feature = "channels")]
pub(crate) use startup::{build_channel_turn_parts, runtime_context, PromptToolDescs};

#[cfg(all(test, feature = "channels"))]
pub mod test_support;

// Re-exported for `channels::tests` only; omit in normal lib builds to avoid unused-import warnings.
#[cfg(all(test, feature = "channels"))]
pub(crate) use dispatch::run_message_dispatch_loop;
