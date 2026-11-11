//! OpenHuman host callbacks and event subscribers for the TinyChannels module.
//!
//! Production startup serves the typed callback surface from `runtime::startup`.

mod adapters;
mod channel_events;
pub(crate) mod remote_control;
pub(crate) mod voice;

pub(crate) use adapters::{ConfigAllowlistStore, OpenHumanEventSink};
pub(crate) use channel_events::ChannelApprovalSurfaceSubscriber;
pub use channel_events::ChannelTurnStateSubscriber;

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
