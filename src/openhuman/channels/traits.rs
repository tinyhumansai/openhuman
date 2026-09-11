//! The channel seam, re-exported from the contract crate.
//!
//! This carve-out is deliberately UNGATED: the always-on agent-harness
//! interactive loop and `cron::bus` both name these, so they must resolve in a
//! `channels`-less build. They come from `tinychannels-bus` rather than
//! `tinychannels` for exactly that reason — the implementation crate is
//! optional now, the vocabulary is not.
pub use tinychannels_bus::{Channel, ChannelMessage, ChannelSendExt, SendMessage};

#[cfg(test)]
#[path = "traits_tests.rs"]
mod tests;
