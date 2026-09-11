//! Event bus handlers for the channels domain.
//!
//! The [`ChannelInboundSubscriber`] handles inbound channel messages published
//! by the socket transport layer. It runs the agent inference loop via the web
//! channel provider and sends the reply back through the REST API.

#[cfg(test)]
#[path = "bus_inbound_thread_id_tests_tests.rs"]
mod inbound_thread_id_tests;

#[cfg(test)]
#[path = "bus_tests.rs"]
mod tests;

#[cfg(any(test, debug_assertions))]
#[path = "bus_test_support_tests.rs"]
pub mod test_support;
include!("bus_part_01.rs");
include!("bus_part_02.rs");
