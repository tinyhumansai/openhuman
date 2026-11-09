//! Host facts supplied to TinyAgents' generic workload router.

use super::*;
use tinyagents_registry::router::{
    DefaultModelRoute, FallbackDiagnostics, RouteSource, WorkloadRouter,
};

static WORKLOAD_ROUTER: std::sync::LazyLock<WorkloadRouter> =
    std::sync::LazyLock::new(|| WorkloadRouter::from_workload_routes("hint:vision"));

/// Config-backed facts that the generic router needs from the host.
pub(crate) struct ConfigRouteSource<'a> {
    config: &'a Config,
    provider_id: &'a str,
    extensions: Option<Option<&'a crate::core::slots::Extensions>>,
    primary_cloud_route: String,
    managed_default_model: Option<String>,
}

impl<'a> ConfigRouteSource<'a> {
    pub(crate) fn new(config: &'a Config, provider_id: &'a str) -> Self {
        Self::with_extension_selection(config, provider_id, None)
    }

    pub(crate) fn with_extension_selection(
        config: &'a Config,
        provider_id: &'a str,
        extensions: Option<Option<&'a crate::core::slots::Extensions>>,
    ) -> Self {
        Self {
            config,
            provider_id,
            extensions,
            primary_cloud_route: crate::config::ops::resolve_primary_cloud_provider_string(config),
            managed_default_model: None,
        }
    }

    pub(crate) fn with_managed_default_model(mut self, model: Option<String>) -> Self {
        self.managed_default_model = model;
        self
    }
}

impl RouteSource for ConfigRouteSource<'_> {
    fn configured_route_for_role(&self, role: &str) -> Option<&str> {
        configured_route_for_role(role, self.config)
    }

    fn primary_cloud_route(&self) -> &str {
        &self.primary_cloud_route
    }

    fn default_model(&self) -> Option<&str> {
        self.config.default_model.as_deref()
    }

    fn classify_default_model_route(&self, model: &str) -> DefaultModelRoute {
        if is_explicit_provider_default(model, self.config) {
            DefaultModelRoute::ExplicitProvider
        } else if model.trim().starts_with("openrouter/") {
            DefaultModelRoute::ManagedCatalog
        } else {
            DefaultModelRoute::Other
        }
    }

    fn is_managed_route(&self, route: &str) -> bool {
        let route = route.trim();
        route.is_empty()
            || route == "cloud"
            || route == PROVIDER_OPENHUMAN
            || route == BYOK_INCOMPLETE_SENTINEL
            || route == self.provider_id
    }

    fn managed_default_model(&self) -> Option<&str> {
        self.managed_default_model.as_deref()
    }

    fn legacy_tier_role(&self, tier: &str) -> Option<&str> {
        let role = match self.extensions {
            None => crate::inference::registry::provider_model_role(self.provider_id, tier),
            Some(extensions) => crate::inference::registry::provider_model_role_for(
                extensions,
                self.provider_id,
                tier,
            ),
        };
        role.and_then(|role| {
            crate::config::WORKLOAD_ROLES
                .iter()
                .copied()
                .find(|known| *known == role)
        })
    }
}

/// Explicit route configured for a workload role, before fallback resolution.
pub(super) fn configured_route_for_role<'a>(role: &str, config: &'a Config) -> Option<&'a str> {
    match role {
        "chat" => config.chat_provider.as_deref(),
        "reasoning" => config.reasoning_provider.as_deref(),
        "agentic" | "burst" => config.agentic_provider.as_deref(),
        "coding" => config.coding_provider.as_deref(),
        "vision" => config.vision_provider.as_deref(),
        "memory" | "summarization" => config.memory_provider.as_deref(),
        "embeddings" => config.embeddings_provider.as_deref(),
        _ => None,
    }
}

/// Shared workload routing policy, with product-specific facts supplied here.
pub(crate) fn workload_router() -> &'static WorkloadRouter {
    &WORKLOAD_ROUTER
}

/// Route a role through the native workload router.
pub fn provider_for_role(role: &str, config: &Config) -> String {
    let source = ConfigRouteSource::new(config, PROVIDER_OPENHUMAN);
    let resolved = WORKLOAD_ROUTER.provider_for_role(role, &source);
    if let Some(diagnostic) = WORKLOAD_ROUTER.fallback_diagnostics(role, &source) {
        if let Some(chat) = config
            .chat_provider
            .as_deref()
            .filter(|chat| tinyinference_local::profile::is_local_provider_string(chat))
        {
            log::info!(
                "[providers][local-fallback] role={} {}",
                role,
                crate::inference::provider::fallback_diagnostics::cloud_fallback_notice(
                    &diagnostic,
                    chat,
                )
            );
        }
    }
    resolved
}

/// Whether the role's selected provider can be funded directly by the user.
pub async fn role_bypasses_managed_credits(role: &str, config: &Config) -> bool {
    let resolved = provider_for_role(role, config);
    let r = resolved.trim();
    let is_managed =
        r.is_empty() || r == "cloud" || r == PROVIDER_OPENHUMAN || r == BYOK_INCOMPLETE_SENTINEL;
    let usable_byo = !is_managed && route_has_usable_credentials(r, config).await;
    log::debug!(
        "[billing] role_bypasses_managed_credits role={role} resolved={resolved} \
         is_managed={is_managed} usable_byo={usable_byo}"
    );
    usable_byo
}

/// Native route metadata drives the branded explanation; this helper only
/// determines whether the host should attach that explanation to an error.
pub(crate) fn fallback_diagnostics_for_role(
    role: &str,
    config: &Config,
) -> Option<FallbackDiagnostics> {
    let source = ConfigRouteSource::new(config, PROVIDER_OPENHUMAN);
    WORKLOAD_ROUTER.fallback_diagnostics(role, &source)
}

/// Kept until v0.66; review after 2027-01-11 before removing legacy
/// `inference_url` route compatibility from the host's route source.
fn is_explicit_provider_default(model: &str, config: &Config) -> bool {
    let model = model.trim();
    if model.is_empty() || model.starts_with("hint:") || model.starts_with("openrouter/") {
        return false;
    }
    if !model.contains(':') {
        return model == CLAUDE_AGENT_SDK_PROVIDER;
    }
    let Some((slug, rest)) = model.split_once(':') else {
        return false;
    };
    let slug = slug.trim();
    !slug.is_empty()
        && slug != PROVIDER_OPENHUMAN
        && !rest.trim().is_empty()
        && (config
            .cloud_providers
            .iter()
            .any(|entry| entry.slug == slug)
            || tinyinference_local::profile::is_local_provider_string(model)
            || model.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)
            || model.starts_with(CLAUDE_AGENT_SDK_PREFIX))
}

/// True when a resolved chat-tier provider string can actually run on the
/// user's own funding: local runtimes / claude-code carry their own creds; a
/// concrete cloud slug requires a non-empty stored key.
pub(super) async fn route_has_usable_credentials(resolved: &str, config: &Config) -> bool {
    let r = resolved.trim();
    if tinyinference_local::profile::is_local_provider_string(r)
        || r.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)
        || r == CLAUDE_AGENT_SDK_PROVIDER
        || r.starts_with(CLAUDE_AGENT_SDK_PREFIX)
    {
        return true;
    }
    if let Some((slug, _)) = r.split_once(':') {
        let slug = slug.trim();
        if !slug.is_empty() {
            match lookup_key_for_slug(slug, config).await {
                Ok(key) => {
                    let usable = !key.trim().is_empty();
                    log::debug!(
                        "[billing] route_has_usable_credentials slug={slug} usable={usable}"
                    );
                    return usable;
                }
                Err(error) => {
                    log::debug!(
                        "[billing] route_has_usable_credentials slug={slug} lookup_error={error}"
                    );
                    return false;
                }
            }
        }
    }
    false
}

/// `provider` with its model replaced by `model`, when one is given: the same
/// provider (and so the same credentials, endpoint and route temperature)
/// serving another model. A provider string with no model of its own (a
/// routing sentinel such as `cloud`, or the managed backend's bare
/// `openhuman`) is returned as is; those routes take the model from the
/// request's pin instead.
pub(super) fn route_with_model(provider: String, model: Option<&str>) -> String {
    use tinyinference_providers::route::{ProviderRoute, RouteKind};
    let Some(model) = model else {
        return provider;
    };
    match ProviderRoute::parse(&provider) {
        Some(route) if route.kind() != RouteKind::Sentinel => match route.temperature {
            Some(temperature) => format!("{}:{model}@{temperature}", route.provider),
            None => format!("{}:{model}", route.provider),
        },
        _ => provider,
    }
}

#[cfg(test)]
#[path = "routing_tests.rs"]
mod routing_tests;
