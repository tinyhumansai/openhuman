//! Search keys an embedder pins on one agent's in-memory config.
//!
//! A process that runs several agents (an embedder's hive, a test harness)
//! cannot hand each its own search key through `config.toml` or the
//! environment: both are shared by every agent in the process. A pinned
//! [`SearchHostCredential`] lives only on that agent's [`SearchConfig`]: it is
//! never serialized, it redacts itself in `Debug`, and while any is pinned the
//! search proxy runs that agent's calls under the agent's own config instead
//! of reloading `config_path` (see `modules::search::proxy`).

use super::{SearchConfig, SearchProviderSettings};

/// A per-agent search-provider credential supplied by an embedder.
///
/// Never persisted and never printed: `Debug` shows only whether a base URL
/// override is set.
#[derive(Clone, PartialEq, Eq)]
pub struct SearchHostCredential {
    api_key: String,
    base_url: Option<String>,
}

impl SearchHostCredential {
    /// A credential for `api_key` against the provider's public endpoint.
    pub fn new(api_key: impl Into<String>) -> Self {
        Self {
            api_key: api_key.into().trim().to_string(),
            base_url: None,
        }
    }

    /// Send this provider's calls to `base_url` instead of its public
    /// endpoint (a proxy, or a local stand-in under test).
    #[must_use]
    pub fn with_base_url(mut self, base_url: impl Into<String>) -> Self {
        self.base_url = Some(base_url.into().trim().to_string());
        self
    }

    /// The key, or `None` when it is empty after trimming.
    pub fn api_key(&self) -> Option<&str> {
        (!self.api_key.is_empty()).then_some(self.api_key.as_str())
    }

    /// The endpoint override, if any.
    pub fn base_url(&self) -> Option<&str> {
        self.base_url.as_deref().filter(|url| !url.is_empty())
    }
}

impl std::fmt::Debug for SearchHostCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SearchHostCredential")
            .field("api_key", &"<redacted>")
            .field("base_url", &self.base_url.is_some())
            .finish()
    }
}

/// Why [`SearchConfig::pin_host_credential`] refused a provider.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("search provider '{0}' does not take an API key")]
pub struct UnknownKeyedSearchProvider(pub String);

impl SearchConfig {
    /// Pin `credential` as this agent's key for `provider` (`brave`, `exa`,
    /// `gemini`, `parallel`, `querit`, `tavily` or `tinyfish`).
    ///
    /// Turns search on and puts `provider` on its direct route, the route a
    /// key authenticates. The key wins over any stored `[search.<provider>]`
    /// key for this config, and from then on this agent's search calls run
    /// under this config rather than a reload of `config_path`.
    ///
    /// # Errors
    ///
    /// [`UnknownKeyedSearchProvider`] when `provider` takes no single key.
    pub fn pin_host_credential(
        &mut self,
        provider: &str,
        credential: SearchHostCredential,
    ) -> Result<(), UnknownKeyedSearchProvider> {
        let provider = provider.trim();
        if self.credentials(provider).is_none() || provider == "gemini_deep_research" {
            return Err(UnknownKeyedSearchProvider(provider.to_string()));
        }
        self.enabled = Some(true);
        self.providers
            .insert(provider.to_string(), SearchProviderSettings::direct());
        self.host_credentials
            .insert(provider.to_string(), credential);
        Ok(())
    }

    /// Whether an embedder pinned search credentials on this config, making
    /// it authoritative for this agent's search calls.
    pub fn is_host_pinned(&self) -> bool {
        !self.host_credentials.is_empty()
    }

    /// The pinned credential for `provider`. Deep research shares Gemini's.
    pub fn host_credential(&self, provider: &str) -> Option<&SearchHostCredential> {
        let provider = match provider {
            "gemini_deep_research" => "gemini",
            other => other,
        };
        self.host_credentials.get(provider)
    }

    /// The key a direct call to `provider` authenticates with: the pinned one
    /// first, then the stored `[search.<provider>]` key.
    pub fn direct_key(&self, provider: &str) -> Option<&str> {
        self.host_credential(provider)
            .and_then(SearchHostCredential::api_key)
            .or_else(|| self.credentials(provider).and_then(|c| c.key()))
    }
}

#[cfg(test)]
#[path = "search_host_tests.rs"]
mod tests;
