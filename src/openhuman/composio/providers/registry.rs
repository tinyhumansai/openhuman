//! Process-global registry of [`ComposioProvider`] implementations.
//!
//! There is exactly one provider per toolkit slug — the trait is not
//! a fan-out fan-in dispatch, it is a 1:1 mapping. This keeps trigger
//! routing simple (`HashMap::get(toolkit)` → call) and avoids the
//! "which subscriber wins" ambiguity that would come with multiple
//! providers per toolkit.
//!
//! The registry is initialised once at startup via
//! [`init_default_providers`] and is intentionally write-rare: tests
//! can register additional providers ad-hoc, but the production path
//! only writes during the startup hook.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock, RwLock};

use super::ComposioProvider;

/// Reference-counted handle to a registered provider.
pub type ProviderArc = Arc<dyn ComposioProvider>;

/// Backing storage for the global registry.
///
/// `RwLock<HashMap<…>>` is fine here — registration happens at
/// startup and lookups are very fast (no contention in steady state).
type Registry = RwLock<HashMap<String, ProviderArc>>;

static REGISTRY: OnceLock<Registry> = OnceLock::new();

fn registry() -> &'static Registry {
    REGISTRY.get_or_init(|| RwLock::new(HashMap::new()))
}

/// Register or replace a provider for its toolkit slug.
///
/// Idempotent — re-registering the same toolkit overwrites the
/// previous entry, which is what tests rely on for setup/teardown.
pub fn register_provider(provider: ProviderArc) {
    let slug = provider.toolkit_slug().to_string();
    if slug.is_empty() {
        tracing::warn!("[composio:registry] refusing to register provider with empty slug");
        return;
    }
    let mut guard = registry()
        .write()
        .expect("composio provider registry poisoned");
    let was_present = guard.insert(slug.clone(), provider).is_some();
    if was_present {
        tracing::debug!(toolkit = %slug, "[composio:registry] replaced existing provider");
    } else {
        tracing::info!(toolkit = %slug, "[composio:registry] provider registered");
    }
}

/// Look up the provider for a toolkit slug, if one is registered.
pub fn get_provider(toolkit: &str) -> Option<ProviderArc> {
    let key = toolkit.trim();
    if key.is_empty() {
        return None;
    }
    let guard = registry()
        .read()
        .expect("composio provider registry poisoned");
    guard.get(key).cloned()
}

/// Snapshot of every registered provider, in unspecified order. Used
/// by the periodic sync scheduler to walk every toolkit.
pub fn all_providers() -> Vec<ProviderArc> {
    let guard = registry()
        .read()
        .expect("composio provider registry poisoned");
    guard.values().cloned().collect()
}

/// Register the built-in providers shipped with the core. Called once
/// from `start_channels` / `bootstrap_skill_runtime` startup paths.
///
/// Idempotent: re-running just re-registers (no-op in practice).
pub fn init_default_providers() {
    register_provider(Arc::new(super::gmail::GmailProvider::new()));
    register_provider(Arc::new(super::notion::NotionProvider::new()));
    tracing::info!(
        count = all_providers().len(),
        "[composio:registry] default providers initialised"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::composio::providers::{
        ProviderContext, ProviderUserProfile, SyncOutcome, SyncReason,
    };
    use async_trait::async_trait;

    struct DummyProvider {
        slug: &'static str,
    }

    #[async_trait]
    impl ComposioProvider for DummyProvider {
        fn toolkit_slug(&self) -> &'static str {
            self.slug
        }
        async fn fetch_user_profile(
            &self,
            _ctx: &ProviderContext,
        ) -> Result<ProviderUserProfile, String> {
            Ok(ProviderUserProfile::default())
        }
        async fn sync(
            &self,
            _ctx: &ProviderContext,
            _reason: SyncReason,
        ) -> Result<SyncOutcome, String> {
            Ok(SyncOutcome::default())
        }
    }

    #[test]
    fn register_and_lookup_roundtrip() {
        register_provider(Arc::new(DummyProvider {
            slug: "test_dummy_a",
        }));
        let p = get_provider("test_dummy_a").expect("provider should be registered");
        assert_eq!(p.toolkit_slug(), "test_dummy_a");
    }

    #[test]
    fn lookup_unknown_returns_none() {
        assert!(get_provider("__definitely_not_a_real_toolkit__").is_none());
    }

    #[test]
    fn register_replaces_existing() {
        register_provider(Arc::new(DummyProvider {
            slug: "test_dummy_b",
        }));
        register_provider(Arc::new(DummyProvider {
            slug: "test_dummy_b",
        }));
        // Still exactly one entry under that slug.
        let count_with_b = all_providers()
            .iter()
            .filter(|p| p.toolkit_slug() == "test_dummy_b")
            .count();
        assert_eq!(count_with_b, 1);
    }

    #[test]
    fn empty_slug_is_rejected() {
        register_provider(Arc::new(DummyProvider { slug: "" }));
        assert!(get_provider("").is_none());
    }
}
