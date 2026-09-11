//! Domain types for the Composio integration.
//!
//! The definitions live in **`tinyconnectors_bus`**, the connector module's wire
//! contract, and are re-exported here so every existing
//! `integrations::composio::types::…` path keeps resolving and keeps naming the
//! same types.
//!
//! They were previously reached through `tinymemory_api::host::composio`, which
//! held them only because two crates needed to name them and there was nowhere
//! else both could. That is no longer true: the crate that *produces* these
//! shapes owns them, and memory takes them — or, after the sync migration,
//! stops needing them at all.
//!
//! **Their serde form is a wire contract**, not an implementation detail: it
//! mirrors the backend's response envelopes under
//! `/agent-integrations/composio/*`, and a field renamed on one side fails at
//! runtime with a decode error rather than at compile time. The contract crate
//! pins every one of them in a test for that reason.

pub use tinyconnectors_bus::composio::*;

/// Re-encode a value across the two parallel Composio type sets.
///
/// `tinymemory-api` still carries its own copy of these shapes, because
/// `tinymemory_core::composio_host::ComposioHost` — which this crate implements
/// — is typed against them. The two sets are field-for-field identical: the
/// memory copy was written from the same backend envelopes the contract crate's
/// was, which is exactly why holding them in two places was worth ending.
///
/// So this is a re-encode, not a translation, and it is **temporary**. Phase 4
/// of the connector extraction deletes `ComposioHost` along with the memory-side
/// copy, and every call to this function goes with it. Until then the
/// conversion is explicit and in one place rather than spread across the call
/// sites, so removing it later is a matter of deleting this function and
/// following the compiler.
///
/// # Errors
///
/// Returns an error only if the two shapes have drifted apart — which is the
/// failure this whole migration exists to make impossible, and is worth hearing
/// about loudly rather than papering over.
pub fn reencode<From, To>(value: &From) -> Result<To, String>
where
    From: serde::Serialize,
    To: serde::de::DeserializeOwned,
{
    let json = serde_json::to_value(value)
        .map_err(|error| format!("composio type re-encode failed to serialize: {error}"))?;
    serde_json::from_value(json).map_err(|error| {
        format!("composio type re-encode failed: the two copies have drifted — {error}")
    })
}
