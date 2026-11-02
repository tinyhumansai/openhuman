//! Channel contracts and OpenHuman runtime orchestration.
//!
//! Provider orchestration is gated by `channels` and runs through the
//! TinyChannels TinyBus module. Serialized configuration, messages, and
//! schemas come from `tinychannels-bus`; OpenHuman retains host policy and
//! lifecycle orchestration.
// Always-compiled carve-outs (see module docs).
pub(crate) mod capabilities;
pub mod contract_schema;
#[cfg(any(feature = "voice", feature = "channels"))]
pub(crate) mod credentials;

pub use tinychannels_bus::SendMessage;

#[cfg(feature = "channels")]
pub mod bus;
#[cfg(feature = "channels")]
pub mod callback_slot;
#[cfg(feature = "channels")]
mod commands;
#[cfg(feature = "channels")]
pub(crate) use commands::{configured_providers, has_listener_configuration};
#[cfg(feature = "channels")]
pub(crate) mod context;
#[cfg(feature = "channels")]
pub mod controllers;
#[cfg(feature = "channels")]
pub(crate) mod failure_copy;
#[cfg(feature = "channels")]
pub mod host;
#[cfg(feature = "channels")]
pub mod proactive;
pub(crate) mod prompt;
#[cfg(feature = "channels")]
pub mod providers;
#[cfg(feature = "channels")]
mod routes;
#[cfg(all(test, feature = "channels"))]
pub(crate) use routes::register_hosted_route_test_policy;
// Ungated: the agent domain's `agent.run_turn` handler drives every turn
// through `runtime::turn_driver`, with or without the `channels` feature.
// Everything else in `runtime` carries its own `channels` gate.
pub(crate) mod runtime;
#[cfg(feature = "channels")]
pub(crate) mod system_prompt;

#[cfg(feature = "channels")]
pub(crate) use system_prompt::ChannelSystemPrompt;

#[cfg(all(feature = "channels", test))]
mod tests;

#[cfg(all(feature = "channels", any(test, debug_assertions)))]
pub mod test_support;

#[cfg(feature = "channels")]
// Channel system-prompt assembly lives in
// `crate::channels::prompt` alongside the rest of
// the prompt-building code. Re-exported here for callers that used the
// old `channels::build_system_prompt` path.
#[cfg(feature = "channels")]
pub use prompt::build_system_prompt;
#[cfg(feature = "channels")]
pub use runtime::start_channels;
pub(crate) use runtime::turn_driver::run_channel_turn;
#[cfg(feature = "channels")]
pub(crate) use runtime::{session, start_channels_with_session};

#[cfg(all(test, feature = "channels", feature = "modules"))]
#[path = "module_fixture_tests.rs"]
pub(crate) mod module_fixture;
