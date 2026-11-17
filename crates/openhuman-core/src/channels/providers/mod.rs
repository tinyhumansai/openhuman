//! Hosted relay entry.
//!
//! Provider transports, and the provider-independent remote control and
//! in-chat approvals that used to be Telegram glue, belong to `tinychannels`.
//! [`relay`] is the exception: not a transport, but the host-side entry for
//! messages a gateway relays in from a hosted platform.

pub mod relay;
