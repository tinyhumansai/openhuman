//! Immutable provider bindings for authenticated profile contexts.
//!
//! The binding is skipped by Config serialization. Only private module
//! initialization receives search credentials; public profile configs hold no
//! operator keys. Role aliases are a host policy boundary, never a module tool
//! router that can point at arbitrary native capabilities.

use crate::config::{
    Config, PresentationMode, ProviderRoute, SearchConfig, SearchProviderSettings,
};
use crate::core::runtime::direct_profile_provider::{validate_endpoint, DirectProfileProvider};
use crate::core::runtime::SaasConfig;
use std::collections::{BTreeMap, HashSet};
use std::sync::Arc;
use tinysearch_bus::Role;
/// Search capability vocabulary for operator presentation settings.
pub use tinysearch_bus::Role as SearchRole;

/// Host-only settings carried by a profile's immutable config/context.
#[derive(Debug, Clone)]
pub struct ProfileProviderSettings {
    chat: Option<DirectProfileProvider>,
    search: SearchConfig,
    role_names: BTreeMap<Role, String>,
}

impl ProfileProviderSettings {
    /// Binding of the profile `ctx` runs for, never a process fallback.
    pub fn in_ctx(ctx: &crate::core::runtime::DispatchCtx) -> Option<Arc<Self>> {
        let context = ctx.core()?;
        context.profile()?;
        context.embedder_config()?.profile_providers.clone()
    }

    /// Canonical capability for a registered name. A renamed canonical role is
    /// refused rather than being allowed to bypass its advertised scope.
    pub(crate) fn policy_name<'a>(&self, name: &'a str) -> Option<std::borrow::Cow<'a, str>> {
        if let Some(canonical) = self.canonical_role_name(name) {
            return Some(std::borrow::Cow::Borrowed(canonical));
        }
        tinysearch_bus::role_for_tool(name)
            .is_none()
            .then_some(std::borrow::Cow::Borrowed(name))
    }

    /// Whether the operator configured a direct chat provider, so profile
    /// turns need no TinyHumans backend credential.
    pub(crate) fn has_direct_chat(&self) -> bool {
        self.chat.is_some()
    }

    /// Validate operator-owned direct routes and their presentation.
    ///
    /// # Errors
    /// Rejects managed routes, extra providers, non-role presentation, invalid
    /// endpoints, unsafe aliases, duplicate names, and native tool collisions.
    pub fn from_saas(saas: &SaasConfig) -> anyhow::Result<Self> {
        let mut search = saas.search.clone().unwrap_or_else(|| SearchConfig {
            enabled: Some(false),
            providers: BTreeMap::new(),
            ..SearchConfig::default()
        });
        if search.presentation != PresentationMode::Roles {
            anyhow::bail!("profile search requires role presentation");
        }
        if search.providers.iter().any(|(provider, setting)| {
            setting.enabled
                && (!matches!(provider.as_str(), "exa" | "gemini")
                    || setting.route != ProviderRoute::Direct)
        }) {
            anyhow::bail!("profile search permits only direct Exa and Gemini providers");
        }
        for provider in ["exa", "gemini"] {
            if let Some(endpoint) = search
                .credentials(provider)
                .and_then(|credential| credential.base_url.as_deref())
            {
                validate_endpoint(endpoint)?;
            }
        }
        // The deployment has exactly these roles and fallback order. Arguments
        // and stored user settings cannot add another provider or capability.
        let usable: HashSet<_> = ["exa", "gemini"]
            .into_iter()
            .filter(|provider| {
                search
                    .providers
                    .get(*provider)
                    .is_some_and(|setting| setting.enabled)
                    && search
                        .credentials(provider)
                        .is_some_and(|credential| credential.has_key())
            })
            .collect();
        search.providers = usable
            .iter()
            .map(|provider| ((*provider).to_owned(), SearchProviderSettings::direct()))
            .collect();
        search.enabled = Some(!usable.is_empty());
        search.presentation_provider = None;
        search.roles = BTreeMap::from([
            (
                "search".into(),
                ["exa", "gemini"]
                    .into_iter()
                    .filter(|name| usable.contains(name))
                    .map(str::to_owned)
                    .collect(),
            ),
            (
                "answer".into(),
                ["gemini"]
                    .into_iter()
                    .filter(|name| usable.contains(name))
                    .map(str::to_owned)
                    .collect(),
            ),
            (
                "contents".into(),
                ["exa"]
                    .into_iter()
                    .filter(|name| usable.contains(name))
                    .map(str::to_owned)
                    .collect(),
            ),
        ]);
        let role_names = saas.search_role_names.clone();
        let mut used = HashSet::new();
        for role in tinysearch_bus::ROLES {
            let canonical = tinysearch_bus::names::role_tool_name(role);
            let name = role_names
                .get(&role)
                .map(String::as_str)
                .unwrap_or(canonical);
            if !valid_identifier(name) || !used.insert(name.to_owned()) {
                anyhow::bail!("profile search role names require unique safe tool identifiers");
            }
            if name != canonical && collides(name) {
                anyhow::bail!("profile search role name collides with a native tool");
            }
        }
        Ok(Self {
            chat: saas.chat_provider.clone(),
            search,
            role_names,
        })
    }

    /// Attach a serialization-skipped binding to a profile config. The visible
    /// search policy is safe to persist; credentials and endpoint overrides are absent.
    pub fn bind(&self, config: &mut Config) {
        let mut public_search = self.search.clone();
        for provider in crate::config::SEARCH_PROVIDERS {
            if let Some(credential) = public_search.credentials_mut(provider) {
                credential.api_key = None;
                credential.base_url = None;
            }
        }
        config.search = public_search;
        config.profile_providers = Some(Arc::new(self.clone()));
        self.apply_chat(config);
    }

    /// Model fixed by the operator, when direct chat was configured.
    pub(crate) fn chat_model(&self) -> Option<&str> {
        self.chat.as_ref().map(DirectProfileProvider::model)
    }

    pub(crate) fn apply_chat(&self, config: &mut Config) {
        if let Some(chat) = &self.chat {
            chat.apply(config);
        }
    }

    /// Private search configuration for module initialization only.
    pub(crate) fn effective_search(&self, config: &Config) -> Config {
        let mut effective = config.clone();
        effective.search = self.search.clone();
        // Legacy search integrations cannot widen a direct profile's providers.
        effective.seltz.enabled = false;
        effective.searxng.enabled = false;
        effective
    }

    /// An advertised role maps exactly once to its canonical module operation.
    /// Renamed canonical operations and provider-native names never resolve.
    pub fn canonical_role_name(&self, advertised: &str) -> Option<&'static str> {
        tinysearch_bus::ROLES.into_iter().find_map(|role| {
            (self.advertised_role_name(role) == advertised)
                .then(|| tinysearch_bus::names::role_tool_name(role))
        })
    }

    /// The operator-selected name for a role, or the ordinary canonical name.
    pub fn advertised_role_name(&self, role: Role) -> &str {
        self.role_names
            .get(&role)
            .map(String::as_str)
            .unwrap_or_else(|| tinysearch_bus::names::role_tool_name(role))
    }

    /// Rename only canonical role declarations; no provider extras are exposed.
    pub(crate) fn present(&self, specs: &mut Vec<tinysearch_bus::ToolSpec>) {
        specs.retain_mut(|spec| {
            let Some(role) = tinysearch_bus::role_for_tool(&spec.name) else {
                return false;
            };
            // The module treats an empty role as its default order, which could
            // add a provider the operator left out (Exa answers); keep it off.
            if self
                .search
                .roles
                .get(tinysearch_bus::role_key(role))
                .is_none_or(Vec::is_empty)
            {
                return false;
            }
            spec.name = self.advertised_role_name(role).to_owned();
            // Deep research is a separate capability the deployment never grants.
            if let Some(properties) = spec
                .parameters
                .get_mut("properties")
                .and_then(serde_json::Value::as_object_mut)
            {
                properties.remove("depth");
            }
            true
        });
    }
}

fn valid_identifier(name: &str) -> bool {
    let mut characters = name.chars();
    name.len() <= 64
        && characters
            .next()
            .is_some_and(|first| first.is_ascii_alphabetic() || first == '_')
        && characters.all(|character| character.is_ascii_alphanumeric() || character == '_')
}

fn collides(name: &str) -> bool {
    tinysearch_bus::role_for_tool(name).is_some()
        || tinysearch_bus::provider_tool_specs()
            .values()
            .flatten()
            .any(|tool| tool.name == name)
        || crate::tools::toolpacks::pack_for_tool(name).is_some()
        || super::tools::HARD_DENIED.contains(&name)
        || super::tools::HARD_DENIED_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix))
        || matches!(
            name,
            "shell"
                | "memory"
                | "show_page"
                | "search"
                | "http_request"
                | "spawn_agent"
                | "spawn_async_subagent"
        )
}

#[cfg(test)]
#[path = "provider_settings_tests.rs"]
mod tests;
