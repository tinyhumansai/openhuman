//! These assertions exist because the failure they guard against is silent.
//!
//! `tinychannels` is `optional = true`; `tinychannels-bus` is not. Every
//! always-on consumer — `cron::bus`, `security::pairing`,
//! `memory::conversations::bus`, `config::schema::channels` and the
//! `DomainEvent` variant — must therefore name the *contract* crate. Point
//! one of them back at `tinychannels` and a `channels`-less build stops
//! resolving, which CI's `cargo check` smoke lane would catch only if it
//! ran that profile.
//!
//! Re-declaring any of these host-side is the worse failure: it compiles,
//! and then a field added on one side is a decode failure on the other.

/// The re-export is the contract crate's item, not a copy of it.
///
/// `TypeId` equality is the strongest statement available here — two
/// structurally identical types from two different packages are *not*
/// equal, which is exactly the duplicate-package mistake this pins.
#[test]
fn the_channel_vocabulary_is_the_contract_crates_own_types() {
    use std::any::TypeId;

    assert_eq!(
        TypeId::of::<super::ChannelMessage>(),
        TypeId::of::<tinychannels_bus::ChannelMessage>(),
    );
    assert_eq!(
        TypeId::of::<super::SendMessage>(),
        TypeId::of::<tinychannels_bus::SendMessage>(),
    );
}

/// The types the always-on surface pins are reachable without the
/// implementation crate.
///
/// Named individually rather than as a blanket import so that dropping one
/// from the contract is a compile error naming *which* one.
#[test]
fn every_always_on_pin_resolves_through_the_contract_crate() {
    use std::any::TypeId;

    // `DomainEvent::…{ inbound_envelope }` — see `core::events`.
    let _ = TypeId::of::<tinychannels_bus::ChannelInboundEnvelope>();
    // `config::schema::channels` re-exports this whole surface.
    let _ = TypeId::of::<tinychannels_bus::ChannelsConfig>();
    // `security::pairing` re-exports the pairing helpers from here.
    let _ = TypeId::of::<tinychannels_bus::channel::SessionKeyPolicy>();
}
