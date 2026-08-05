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
/// The provider that should reshape an action response, or `None`.
///
/// Two conditions, and the second is the one that is easy to forget at a call
/// site: the slug must map to a registered provider, **and** the response must
/// have succeeded. [`ComposioProvider::post_process_action_result`] and
/// [`ComposioProvider::reshape_supersedes_markdown`] are both written against
/// the success shape. A failure carries the provider's diagnostics in `data`
/// instead, so a reshaper run on it rewrites them into an empty or wrong-shaped
/// record — and `reshape_supersedes_markdown` then clears the backend's error
/// rendering on behalf of a reshape that found nothing, leaving the model with
/// neither the error nor the diagnostics.
///
/// Named rather than inlined so both execute paths (`ComposioExecuteTool` and
/// `ComposioActionTool`) state the same rule, and so the rule is testable
/// without a live client.
pub fn provider_for_reshape(slug: &str, successful: bool) -> Option<ProviderArc> {
    if !successful {
        return None;
    }
    super::toolkit_from_slug(slug).and_then(|toolkit| get_provider(&toolkit))
}

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
/// from `start_channels` / `bootstrap_core_runtime` startup paths.
///
/// Idempotent: re-running just re-registers (no-op in practice).
pub fn init_default_providers() {
    register_provider(Arc::new(super::clickup::ClickUpProvider::new()));
    register_provider(Arc::new(super::github::GitHubProvider::new()));
    register_provider(Arc::new(super::gmail::GmailProvider::new()));
    register_provider(Arc::new(super::linear::LinearProvider::new()));
    register_provider(Arc::new(super::notion::NotionProvider::new()));
    register_provider(Arc::new(super::slack::SlackProvider::new()));
    tracing::info!(
        count = all_providers().len(),
        "[composio:registry] default providers initialised"
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::sync::composio::providers::{
        ProviderContext, ProviderUserProfile,
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

    /// The regression for the failure path. A `GMAIL_LIST_THREADS` that failed
    /// carries the provider's diagnostics in `data`; reshaping it rewrote them
    /// into the success shape and then cleared the backend's error rendering on
    /// behalf of a reshape that had found nothing, so the model got neither.
    #[test]
    fn a_failed_response_selects_no_provider_to_reshape_with() {
        // A toolkit of this test's own, not `gmail`: the registry is process-
        // global, so registering a `DummyProvider` under a real slug would hand
        // it to any parallel test that looks Gmail up. `toolkit_from_slug`
        // splits on the first `_`, so the slug below resolves to this toolkit.
        register_provider(Arc::new(DummyProvider {
            slug: "reshapeprobe",
        }));

        assert!(
            provider_for_reshape("RESHAPEPROBE_LIST_THINGS", true).is_some(),
            "the success case must still reshape, or this test proves nothing"
        );
        assert!(provider_for_reshape("RESHAPEPROBE_LIST_THINGS", false).is_none());
    }

    /// An unregistered toolkit has nothing to reshape with either way — the
    /// success flag does not manufacture a provider.
    #[test]
    fn an_unknown_slug_selects_no_provider_even_on_success() {
        assert!(provider_for_reshape("NOT_A_REAL_ACTION", true).is_none());
        assert!(provider_for_reshape("", true).is_none());
    }
}
