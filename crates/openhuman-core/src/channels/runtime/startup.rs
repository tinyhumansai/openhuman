//! Channel startup wiring.

mod chat_workload;
mod credentials;
mod prompt;
mod relay;
mod start_channels;
mod turn_parts;

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
