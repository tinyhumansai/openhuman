//! Connected-integrations discovery: the process-wide [`cache`] fronting a
//! mode-aware backend/direct [`fetch`] (whose uncached backend walk lives in
//! [`fetch_uncached`] since it is one large, sequential routine).

mod backend_tools;
mod cache;
mod fetch;
mod fetch_uncached;

#[cfg(test)]
#[path = "connected_integrations_connectable_slug_tests_tests.rs"]
mod connectable_slug_tests;

#[cfg(test)]
#[path = "connected_integrations_catalog_description_tests_tests.rs"]
mod catalog_description_tests;

pub(crate) use cache::sync_cache_with_connections;
pub use cache::{
    cached_active_integrations, cached_active_integrations_including_expired, connected_set_hash,
    invalidate_connected_integrations_cache,
};
#[cfg(test)]
pub(crate) use cache::{composio_cache_test_lock, composio_cache_test_lock_async};
pub use fetch::{
    fetch_connected_integrations, fetch_connected_integrations_status,
    FetchConnectedIntegrationsStatus,
};

// Brought into this module's own namespace (private `use`, not `pub use`)
// so `connected_integrations_connectable_slug_tests_tests.rs` /
// `connected_integrations_catalog_description_tests_tests.rs` — declared as
// direct child modules of `connected_integrations` above — can still reach
// these via a plain `use super::<name>;`, exactly as when this was one
// un-split file. See each item's `pub(super)` in its owning submodule.
#[cfg(test)]
pub(crate) use cache::{
    cache_key, read_cached_integrations_from, CachedIntegrations, INTEGRATIONS_CACHE,
};
#[cfg(test)]
pub(crate) use fetch::{connectable_toolkit_slugs, resolve_toolkit_description};
