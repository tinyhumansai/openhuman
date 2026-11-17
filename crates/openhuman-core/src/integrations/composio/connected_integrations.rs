//! Connected-integrations discovery: the process-wide [`cache`] fronting a
//! mode switch [`fetch`]. The backend/direct walk itself is the module's
//! `ListConnectedIntegrations` member.

mod cache;
mod fetch;

pub(crate) use cache::sync_cache_with_connections;
pub use cache::{
    cached_active_integrations, connected_set_hash, invalidate_connected_integrations_cache,
};
#[cfg(test)]
pub(crate) use cache::{composio_cache_test_lock, composio_cache_test_lock_async};
pub use fetch::{
    fetch_connected_integrations, fetch_connected_integrations_status,
    FetchConnectedIntegrationsStatus,
};

// Test-only access to the cache internals for the sibling cache tests.
#[cfg(test)]
pub(crate) use cache::{
    cache_key, read_cached_integrations_from, CachedIntegrations, INTEGRATIONS_CACHE,
};
