//! Event bus handlers for the channels domain.
//!
//! The [`ChannelInboundSubscriber`] handles inbound channel messages published
//! by the socket transport layer. It runs the agent inference loop via the web
//! channel provider and sends the reply back through the REST API.

mod delivery;
mod subscriber;

pub(crate) use delivery::NativeProgressiveReply;
pub use subscriber::ChannelInboundSubscriber;

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;
