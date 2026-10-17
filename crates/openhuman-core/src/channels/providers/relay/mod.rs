//! Hosted-channel relay: messages from a chat platform the core does not
//! listen to itself, relayed in by a gateway as the user they belong to.
//!
//! In SaaS the cloud gateway owns the hosted Telegram, iMessage and Discord
//! webhooks and the account links (`openhuman_tinyhumans::hosted::channel_link`).
//! For each message it calls `channel_relay_inbound` under that user's scope.
//! The core:
//!
//! 1. validates the message ([`params`]) and asks TinyChannels' confidential
//!    `PrepareChannelIdentity` operation for its canonical thread id,
//!    `channel:<channel>/<sender>/<chat>`;
//! 2. records the message on that thread in the caller's workspace
//!    ([`store`]), refusing a duplicate `message_id`;
//! 3. runs the turn through the channel dispatch pipeline
//!    (`channels::runtime`), under the channel's `ExternalChannel` origin, with
//!    a runtime context built from the caller's config ([`ops`]);
//! 4. publishes each reply as a `channel_outbound` event on the caller's
//!    `/events` stream ([`RelayChannel`]) and records it on the thread.
//!
//! It works in a single-user core too (an embedder's or a local relay), where
//! the caller's config is the process's.

mod channel;
mod ops;
mod params;
mod schemas;
mod store;

pub use channel::{RelayChannel, CHANNEL_OUTBOUND_EVENT};
pub use ops::channel_relay_inbound;
pub use params::{
    validate_channel, validate_id, RelayInboundParams, DEFAULT_RELAY_CLIENT_ID, MAX_CHANNEL_LEN,
    MAX_ID_LEN, MAX_SENDER_NAME_CHARS, MAX_TEXT_BYTES, RESERVED_CHANNELS,
};
pub use schemas::{all_relay_controller_schemas, all_relay_registered_controllers};
