//! Legacy channel orchestration seam during the provider cutover.
//!
//! Serialized messages come from the pure bus contract. The remaining
//! in-process trait is an explicit `tinychannels-runtime` migration exception;
//! it will be replaced when provider runtime callers switch to TinyBus.
pub use tinychannels_bus::{ChannelMessage, SendMessage};
pub use tinychannels_runtime::{Channel, ChannelSendExt};

#[cfg(test)]
#[path = "traits_tests.rs"]
mod tests;
