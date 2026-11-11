//! Provider-neutral helpers for resolving model ids against registered model
//! vocabularies.

use crate::config::Config;
use tinyagents_registry::router::RouteSource;

/// The workload role represented by a provider-specific model alias, defaulting
/// to the generic `chat` role for concrete and unknown ids.
pub fn role_for_model(provider_id: &str, model: &str) -> String {
    crate::inference::registry::provider_model_role(provider_id, model)
        .unwrap_or_else(|| "chat".to_string())
}

/// Generic workload role for a provider-owned model alias.
pub fn role_for_provider_model(provider_id: &str, model: &str) -> &'static str {
    role_for_provider_model_with(provider_id, model, None)
}

pub(crate) fn role_for_provider_model_for(
    extensions: Option<&crate::core::slots::Extensions>,
    provider_id: &str,
    model: &str,
) -> &'static str {
    role_for_provider_model_with(provider_id, model, Some(extensions))
}

fn role_for_provider_model_with(
    provider_id: &str,
    model: &str,
    extensions: Option<Option<&crate::core::slots::Extensions>>,
) -> &'static str {
    let config = Config::default();
    let source = crate::inference::provider::factory::ConfigRouteSource::with_extension_selection(
        &config,
        provider_id,
        extensions,
    );
    let role =
        crate::inference::provider::factory::workload_router().role_for_model_tier(model, &source);
    crate::config::WORKLOAD_ROLES
        .iter()
        .copied()
        .find(|known| *known == role)
        .unwrap_or("chat")
}

/// Whether `model` is a provider-owned alias or a concrete model id.
pub fn is_raw_model(provider_id: &str, model: &str) -> bool {
    crate::inference::registry::provider_model_is_raw(provider_id, model)
}

/// `hint:*` is an abstract routing marker even if no installed provider
/// recognizes the suffix; never pass it to a concrete BYOK endpoint.
pub fn is_abstract_model(provider_id: &str, model: &str) -> bool {
    model.trim().starts_with("hint:")
        || crate::inference::registry::provider_model_is_alias(provider_id, model)
}

/// Context-selected counterpart to [`is_abstract_model`]. Generic `hint:*`
/// markers remain abstract even when no host provider is registered.
pub(crate) fn is_abstract_model_for(
    extensions: Option<&crate::core::slots::Extensions>,
    provider_id: &str,
    model: &str,
) -> bool {
    model.trim().starts_with("hint:")
        || crate::inference::registry::provider_model_is_alias_for(extensions, provider_id, model)
}

/// Resolve a model hint according to the configured provider route and the
/// route's registered provider model policy.
pub fn resolve_provider_model_hint(provider_id: &str, hint: &str, config: &Config) -> String {
    resolve_provider_model_hint_with(provider_id, hint, config, None)
}

pub(crate) fn resolve_provider_model_hint_for(
    extensions: Option<&crate::core::slots::Extensions>,
    provider_id: &str,
    hint: &str,
    config: &Config,
) -> String {
    resolve_provider_model_hint_with(provider_id, hint, config, Some(extensions))
}

fn resolve_provider_model_hint_with(
    provider_id: &str,
    hint: &str,
    config: &Config,
    extensions: Option<Option<&crate::core::slots::Extensions>>,
) -> String {
    let trimmed = hint.trim();
    if trimmed.is_empty() {
        return trimmed.to_string();
    }
    let router = crate::inference::provider::factory::workload_router();
    let base_source =
        crate::inference::provider::factory::ConfigRouteSource::with_extension_selection(
            config,
            provider_id,
            extensions,
        );
    let role = router.role_for_model_tier(trimmed, &base_source);
    let route = router.provider_for_role(&role, &base_source);
    let managed_default_model = if base_source.is_managed_route(&route) {
        match extensions {
            Some(extensions) => crate::inference::registry::resolve_provider_model_for(
                extensions,
                provider_id,
                &role,
                config,
                Some(trimmed),
            ),
            None => crate::inference::registry::resolve_provider_model(
                provider_id,
                &role,
                config,
                Some(trimmed),
            ),
        }
    } else {
        None
    };
    let source = base_source.with_managed_default_model(managed_default_model);
    router
        .resolve_model_for_hint(trimmed, &source)
        .unwrap_or_else(|| trimmed.to_string())
}

#[cfg(test)]
#[path = "provider_model_tests.rs"]
mod tests;
