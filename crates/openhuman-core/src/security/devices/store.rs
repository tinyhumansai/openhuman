//! Paired devices.
//!
//! Every function here is served from the document port
//! (`store_documents.rs`), in the storage backend and the acting scope. The
//! `devices/devices.db` older installs wrote is read once, read-only, by the
//! storage importer (`store_import.rs`).

use anyhow::Result;

use crate::config::Config;
use crate::security::devices::types::PairedDevice;

// ---------------------------------------------------------------------------
// Public store API
// ---------------------------------------------------------------------------

/// Persist a newly-paired device.
pub fn insert_device(
    config: &Config,
    channel_id: &str,
    label: &str,
    device_pubkey: &str,
    core_session_token_hash: &str,
) -> Result<PairedDevice> {
    super::store_documents::current(config)?.insert_device(
        channel_id,
        label,
        device_pubkey,
        core_session_token_hash,
    )
}

/// Update `last_seen_at` for a device (called on `tunnel:peer-status` online events).
pub fn touch_device(config: &Config, channel_id: &str) -> Result<()> {
    super::store_documents::current(config)?.touch_device(channel_id)
}

/// Mark a device as revoked (soft delete).
pub fn revoke_device(config: &Config, channel_id: &str) -> Result<bool> {
    super::store_documents::current(config)?.revoke_device(channel_id)
}

/// Load a single paired device by channel_id (returns None if not found).
pub fn get_device(config: &Config, channel_id: &str) -> Result<Option<PairedDevice>> {
    super::store_documents::current(config)?.get_device(channel_id)
}

/// List all non-revoked paired devices ordered by creation time.
pub fn list_devices(config: &Config) -> Result<Vec<PairedDevice>> {
    super::store_documents::current(config)?.list_devices()
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
