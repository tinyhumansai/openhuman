//! Resolve which search providers are usable right now, and which provider
//! serves each capability role.
//!
//! This is the host's policy view of `[search]`: it combines the user's
//! selection and routes with what the process can actually reach — a backend
//! credential for managed routes, a stored key (or SearXNG base URL) for direct
//! ones, nothing for Keenable's keyless direct route. The settings RPC renders it; `modules::search::module_config` turns it
//! into the TinySearch module configuration. Provider execution is not here.

use serde::Serialize;
use tinysearch_bus::{default_role_providers, provider_roles, Role};

use crate::config::{Config, SearchRoute, MANAGED_SEARCH_PROVIDERS, SEARCH_PROVIDERS};

/// One provider's resolved state.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ResolvedProvider {
    /// Stable provider id (`exa`, `gemini`, ...).
    pub id: &'static str,
    /// Selected by the user.
    pub enabled: bool,
    /// Chosen route (always `direct` for providers that cannot be managed).
    pub route: SearchRoute,
    /// Whether the provider can be reached through the managed backend.
    pub managed_capable: bool,
    /// Whether a backend credential exists (session JWT or API key).
    pub managed_available: bool,
    /// Whether the direct route has what it needs (a key, or a SearXNG URL).
    pub key_configured: bool,
    /// Whether the direct route also works without a key (see [`key_optional()`]).
    pub key_optional: bool,
    /// Enabled, search is on, and the chosen route is reachable.
    pub usable: bool,
    /// Roles this provider can serve.
    pub roles: Vec<Role>,
}

/// Why a provider is not usable, for the settings UI badge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProviderStatus {
    Ready,
    Disabled,
    NeedsKey,
    SignInRequired,
    SearchOff,
}

impl ResolvedProvider {
    pub fn status(&self, search_enabled: bool) -> ProviderStatus {
        if !search_enabled {
            ProviderStatus::SearchOff
        } else if !self.enabled {
            ProviderStatus::Disabled
        } else if self.usable {
            ProviderStatus::Ready
        } else if self.route == SearchRoute::Managed {
            ProviderStatus::SignInRequired
        } else {
            ProviderStatus::NeedsKey
        }
    }
}

/// Whether the direct route of `provider` is configured.
pub fn direct_configured(config: &Config, provider: &str) -> bool {
    match provider {
        "seltz" => config
            .seltz
            .api_key
            .as_deref()
            .is_some_and(|key| !key.trim().is_empty()),
        "searxng" => !config.searxng.base_url.trim().is_empty(),
        other => config
            .search
            .credentials(other)
            .is_some_and(|credentials| credentials.has_key()),
    }
}

/// Whether the direct route of `provider` works without a key. Keenable serves
/// keyless public endpoints rate-limited per IP, so like SearXNG the user's
/// enabled entry is the only opt-in; a stored key moves it to the keyed
/// endpoints. TinySearch applies the same rule when it lists provider tools.
pub fn key_optional(provider: &str) -> bool {
    provider == "keenable"
}

/// Whether the process holds a credential the managed backend accepts.
pub fn backend_credential_available(config: &Config) -> bool {
    crate::security::credentials::session_support::resolve_backend_credential(config).is_ok()
}

/// Resolve every known provider, in catalog order.
pub fn resolve(config: &Config) -> Vec<ResolvedProvider> {
    resolve_with(config, backend_credential_available(config))
}

/// [`resolve`] with the credential check supplied, for tests and for callers
/// that already resolved the credential.
pub fn resolve_with(config: &Config, managed_available: bool) -> Vec<ResolvedProvider> {
    let search_enabled = config.search.is_enabled();
    SEARCH_PROVIDERS
        .iter()
        .map(|&id| {
            let settings = config.search.providers.get(id).copied();
            let enabled = settings.is_some_and(|s| s.enabled);
            let route = config.search.route(id);
            let managed_capable = MANAGED_SEARCH_PROVIDERS.contains(&id);
            let key_configured = direct_configured(config, id);
            let key_optional = key_optional(id);
            let reachable = match route {
                SearchRoute::Managed => managed_capable && managed_available,
                SearchRoute::Direct => key_configured || key_optional,
            };
            ResolvedProvider {
                id,
                enabled,
                route,
                managed_capable,
                managed_available,
                key_configured,
                key_optional,
                usable: search_enabled && enabled && reachable,
                roles: provider_roles(id).to_vec(),
            }
        })
        .collect()
}

/// The configured (or default) provider order for `role`, keeping only
/// providers that can serve it.
pub fn role_order(config: &Config, role: Role) -> Vec<String> {
    let configured = config
        .search
        .roles
        .get(role_key(role))
        .filter(|order| !order.is_empty());
    let order: Vec<String> = match configured {
        Some(order) => order.clone(),
        None => default_role_providers(role)
            .iter()
            .map(|p| (*p).to_string())
            .collect(),
    };
    order
        .into_iter()
        .filter(|provider| provider_roles(provider).contains(&role))
        .collect()
}

/// Usable providers for `role`, in serving order. The first one answers; the
/// rest are fallbacks.
pub fn effective_role_providers(
    resolved: &[ResolvedProvider],
    config: &Config,
    role: Role,
) -> Vec<String> {
    role_order(config, role)
        .into_iter()
        .filter(|provider| {
            resolved
                .iter()
                .any(|p| p.id == provider.as_str() && p.usable)
        })
        .collect()
}

/// All roles, in display order.
pub const ROLES: [Role; 3] = [Role::Search, Role::Answer, Role::Contents];

/// The `[search.roles]` key for a role — its serde name.
pub fn role_key(role: Role) -> &'static str {
    match role {
        Role::Search => crate::config::SEARCH_ROLE_SEARCH,
        Role::Answer => crate::config::SEARCH_ROLE_ANSWER,
        Role::Contents => crate::config::SEARCH_ROLE_CONTENTS,
    }
}

/// Parse a `[search.roles]` key.
pub fn parse_role(key: &str) -> Option<Role> {
    ROLES.into_iter().find(|role| role_key(*role) == key.trim())
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
