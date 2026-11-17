//! Resolve context windows using TinyInference's route resolver.
//!
//! OpenHuman supplies its configuration override, the selected provider's
//! discovery request, catalog and managed-model facts, and the HTTP client
//! configured with the user's CA and proxy policy. TinyInference owns route
//! normalization, discovery, caching, remembered limits, and static fallback
//! precedence.

use std::sync::OnceLock;

use tinyinference_llm::model::context_window::{
    ContextWindowResolver, ContextWindowRouteInput, ManagedDefaultWindow, ResolvedWindow,
};
use tinyinference_llm::model::discover::model_limits_cache;
use tinyinference_llm::model::discover::DiscoveryRequest;
#[cfg(test)]
use tinyinference_llm::model::discover::{ModelLimitsCache, ModelListingFetcher};
use tinyinference_local::profile::LocalProviderKind;

use crate::config::Config;

const MODEL_LIMITS_SERVICE: &str = "inference.model_limits";

/// Converts the local profile kind to TinyInference's route kind. The profile
/// crate and route crate intentionally own distinct enum types.
fn route_local_kind(
    local_kind: Option<LocalProviderKind>,
) -> Option<tinyinference_providers::route::LocalProviderKind> {
    local_kind.map(|kind| match kind {
        LocalProviderKind::Ollama => tinyinference_providers::route::LocalProviderKind::Ollama,
        LocalProviderKind::LmStudio => tinyinference_providers::route::LocalProviderKind::LmStudio,
        LocalProviderKind::Mlx => tinyinference_providers::route::LocalProviderKind::Mlx,
        LocalProviderKind::Omlx => tinyinference_providers::route::LocalProviderKind::Omlx,
        LocalProviderKind::LocalOpenai => {
            tinyinference_providers::route::LocalProviderKind::LocalOpenai
        }
    })
}

/// The user's explicit window for `model`, from `config.model_registry`.
pub(crate) fn config_override(model: &str, config: &Config) -> Option<u64> {
    let model = model.trim();
    config
        .model_registry
        .iter()
        .find(|entry| entry.id.trim() == model && entry.context_window > 0)
        .map(|entry| u64::from(entry.context_window))
}

/// The process-wide resolver: precedence and route/model memory live in
/// `tinyinference-llm`.
fn resolver() -> &'static ContextWindowResolver {
    static RESOLVER: OnceLock<ContextWindowResolver> = OnceLock::new();
    RESOLVER.get_or_init(ContextWindowResolver::new)
}

/// Resolves the window for `model` on the role's selected provider route.
pub(crate) async fn resolve_context_window(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
) -> Option<u64> {
    resolve_context_window_inner(role, provider, model, config, None)
        .await
        .window
}

/// Resolves a window using provider metadata from an explicitly selected
/// runtime context.
pub(crate) async fn resolve_context_window_in(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
    extensions: Option<&crate::core::slots::Extensions>,
) -> Option<u64> {
    resolve_context_window_inner(role, provider, model, config, Some(extensions))
        .await
        .window
}

async fn resolve_context_window_inner(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
    extensions: Option<Option<&crate::core::slots::Extensions>>,
) -> ResolvedWindow {
    let local_kind = tinyinference_local::profile::kind_from_provider_string(provider);
    let mut route = route_input(role, provider, model, config, extensions, local_kind);
    route.discovery = match extensions {
        Some(Some(extensions)) => {
            model_limits_request_for(role, provider, model, config, Some(extensions)).await
        }
        _ => model_limits_request(role, provider, model, config).await,
    };
    #[cfg(test)]
    {
        resolver()
            .resolve_for_route_with_fetcher(
                provider,
                model,
                route_local_kind(local_kind),
                route,
                model_limits_cache(),
                &OfflineFetcher,
            )
            .await
    }
    #[cfg(not(test))]
    {
        resolver()
            .resolve_for_route(
                provider,
                model,
                route_local_kind(local_kind),
                route,
                model_limits_cache(),
                || model_limits_client(config, provider),
            )
            .await
    }
}

/// Unit-test callers never reach a provider unless they explicitly inject a
/// fetcher through `resolve_context_window_with`.
#[cfg(test)]
struct OfflineFetcher;

#[cfg(test)]
#[async_trait::async_trait]
impl ModelListingFetcher for OfflineFetcher {
    async fn get_json(
        &self,
        url: &str,
        _headers: &[(String, String)],
    ) -> tinyinference_llm::Result<serde_json::Value> {
        Err(tinyinference_llm::Error::Catalog(format!(
            "offline in unit tests: {url}"
        )))
    }
}

/// Test seam that injects an offline listing fetcher while preserving the
/// production route-input construction.
#[cfg(test)]
pub(crate) async fn resolve_context_window_with(
    fetcher: &dyn ModelListingFetcher,
    cache: &ModelLimitsCache,
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
) -> ResolvedWindow {
    let local_kind = tinyinference_local::profile::kind_from_provider_string(provider);
    let mut route = route_input(role, provider, model, config, None, local_kind);
    route.discovery = model_limits_request(role, provider, model, config).await;
    resolver()
        .resolve_for_route_with_fetcher(
            provider,
            model,
            route_local_kind(local_kind),
            route,
            cache,
            fetcher,
        )
        .await
}

/// Host-owned route facts shared by async resolution and synchronous
/// projections. The native resolver owns the fallback order and URL handling.
fn route_input(
    _role: &str,
    _provider: &str,
    model: &str,
    config: &Config,
    extensions: Option<Option<&crate::core::slots::Extensions>>,
    local_kind: Option<LocalProviderKind>,
) -> ContextWindowRouteInput {
    let mut route = static_route_facts(model, extensions);
    route.config_override = config_override(model, config);
    route.local_window = local_kind.and_then(|kind| {
        tinyinference_local::profile::context_window_with_local_fallback(model, None, Some(kind))
    });
    if local_kind == Some(LocalProviderKind::Ollama) {
        route.endpoint = Some(tinyinference_local::ollama::ollama_base_url_from_override(
            config.local_ai.base_url.as_deref(),
        ));
    }
    route
}

/// Resolve the host's selected discovery endpoint and credentials, then ask
/// TinyInference's provider registry to construct its native listing request.
/// The host supplies only policy it owns: managed catalog selection, stored
/// cloud credentials, and hosted-auth routing.
pub(crate) async fn model_limits_request(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
) -> Option<DiscoveryRequest> {
    let extensions = crate::core::runtime::CoreContext::current_extensions();
    model_limits_request_for(role, provider, model, config, extensions.as_ref()).await
}

pub(crate) async fn model_limits_request_for(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
    extensions: Option<&crate::core::slots::Extensions>,
) -> Option<DiscoveryRequest> {
    use crate::config::schema::cloud_providers::AuthStyle;
    use tinyinference_llm::providers::registry::{ProviderBuildOptions, ProviderRegistry};
    use tinyinference_providers::route::{ProviderRoute, RouteKind};

    let provider = provider.trim();
    let route = ProviderRoute::parse(provider);
    if provider.is_empty()
        || provider == "cloud"
        || provider == crate::inference::provider::factory::PROVIDER_OPENHUMAN
        || route
            .as_ref()
            .is_some_and(|route| matches!(route.kind(), RouteKind::Managed))
    {
        let model = route
            .as_ref()
            .map(|route| route.model.as_str())
            .filter(|model| !model.is_empty())
            .unwrap_or(model);
        let context = crate::inference::registry::ProviderContext::new(
            config,
            &crate::security::egress::EgressLedger::none(),
        );
        let limits = crate::inference::registry::catalog_for(
            extensions,
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        )
        .and_then(|entry| entry.limits);
        return match limits {
            Some(limits) => {
                limits(
                    tinyinference_llm::providers::overrides::CatalogLimitsRequest {
                        model: model.to_owned(),
                        context,
                    },
                )
                .await
            }
            None => None,
        };
    }

    let route = route?;
    let RouteKind::Cloud { slug } = route.kind() else {
        return None;
    };
    let entry = config
        .cloud_providers
        .iter()
        .find(|entry| entry.slug == slug)?;
    if crate::config::schema::cloud_providers::is_hosted_auth_style_for(
        extensions,
        entry.auth_style,
    ) {
        let model = if route.model.trim().is_empty() {
            model
        } else {
            route.model.trim()
        };
        let context = crate::inference::registry::ProviderContext::new(
            config,
            &crate::security::egress::EgressLedger::none(),
        );
        let limits = crate::inference::registry::catalog_for(
            extensions,
            crate::inference::provider::factory::PROVIDER_OPENHUMAN,
        )
        .and_then(|entry| entry.limits);
        return match limits {
            Some(limits) => {
                limits(
                    tinyinference_llm::providers::overrides::CatalogLimitsRequest {
                        model: model.to_owned(),
                        context,
                    },
                )
                .await
            }
            None => None,
        };
    }
    let resolution = crate::inference::provider::factory::resolve_cloud_slug_for(
        role,
        &slug,
        &route.model,
        config,
        extensions,
    )
    .await
    .ok()?;
    if resolution.codex.using_oauth || resolution.entry.auth_style == AuthStyle::OpenhumanJwt {
        return None;
    }
    let spec = crate::inference::provider::factory::cloud_provider_spec(
        resolution.entry,
        &resolution.effective_model,
    );
    let options = ProviderBuildOptions {
        auth_style: Some(resolution.entry.auth_style.into()),
        codex: Some(&resolution.codex),
        ..ProviderBuildOptions::default()
    };
    ProviderRegistry::new(
        crate::inference::provider::factory::CloudCredentialResolver(&resolution.key),
    )
    .discovery_request(&spec, &options)
    .ok()
    .flatten()
}

/// Static hints owned outside TinyInference's generic model catalog.
fn static_route_facts(
    model: &str,
    extensions: Option<Option<&crate::core::slots::Extensions>>,
) -> ContextWindowRouteInput {
    let model = model.trim();
    if model.is_empty() {
        return ContextWindowRouteInput::default();
    }
    let provider = crate::inference::provider::factory::PROVIDER_OPENHUMAN;
    let provider_model_window = match extensions {
        Some(extensions) => crate::inference::registry::provider_model_context_window_for(
            extensions, provider, model,
        ),
        None => crate::inference::registry::provider_model_context_window(provider, model),
    };
    let managed_default = match extensions {
        Some(extensions) => crate::inference::registry::provider_default_model_for(
            extensions, provider,
        )
        .and_then(|managed_model| {
            crate::inference::registry::provider_model_context_window_for(
                extensions,
                provider,
                &managed_model,
            )
            .map(|window| ManagedDefaultWindow {
                model: managed_model,
                window,
            })
        }),
        None => {
            crate::inference::registry::provider_default_model(provider).and_then(|managed_model| {
                crate::inference::registry::provider_model_context_window(provider, &managed_model)
                    .map(|window| ManagedDefaultWindow {
                        model: managed_model,
                        window,
                    })
            })
        }
    };
    ContextWindowRouteInput {
        catalog_window: tinyagents_registry::pricing::lookup(model)
            .map(|price| u64::from(price.context_window)),
        provider_model_window,
        managed_default,
        ..Default::default()
    }
}

/// Builds the host-configured model-listing client lazily, only when the
/// native resolver determines discovery can run.
#[cfg(not(test))]
fn model_limits_client(config: &Config, provider: &str) -> reqwest::Client {
    model_limits_client_with(
        config,
        provider,
        |pem, service, read_timeout, connect_timeout| {
            crate::util::tls::client_with_ca_bundle_with_timeouts(
                pem,
                service,
                read_timeout,
                connect_timeout,
            )
            .ok()
        },
        crate::config::build_runtime_proxy_client_with_timeouts,
    )
}

/// Projects a synchronous context window for a provider/model route without
/// performing discovery. A full discovery request, when available, scopes any
/// remembered provider result to the exact endpoint and auth/cache identity.
pub(crate) fn project_context_window(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
) -> ResolvedWindow {
    let local_kind = tinyinference_local::profile::kind_from_provider_string(provider);
    let route = route_input(role, provider, model, config, None, local_kind);
    resolver().project_for_route(provider, model, route_local_kind(local_kind), route)
}

/// Projects a static model-only window without borrowing remembered state from
/// the valid empty-provider route used by managed inference.
pub(crate) fn project_context_window_for_model(
    model: &str,
    extensions: Option<Option<&crate::core::slots::Extensions>>,
) -> ResolvedWindow {
    let route = static_route_facts(model, extensions);
    ContextWindowResolver::new().project_for_route("", model, None, route)
}

/// Creates a client that carries OpenHuman's configured CA and proxy policy.
/// Unit tests can supply factories to verify selection without network access.
fn model_limits_client_with(
    config: &Config,
    provider: &str,
    ca_client: impl FnOnce(&str, &str, u64, u64) -> Option<reqwest::Client>,
    proxy_client: impl FnOnce(&str, u64, u64) -> reqwest::Client,
) -> reqwest::Client {
    let provider_slug = provider
        .split_once(':')
        .map_or(provider, |(slug, _)| slug)
        .trim();
    if let Some(client) = config
        .cloud_provider_ca_certs
        .get(provider_slug)
        .filter(|pem| !pem.is_empty())
        .and_then(|pem| ca_client(pem, MODEL_LIMITS_SERVICE, 5, 3))
    {
        return client;
    }
    proxy_client(MODEL_LIMITS_SERVICE, 5, 3)
}

#[cfg(test)]
#[path = "context_window_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "context_window_discovery_tests.rs"]
mod discovery_tests;
