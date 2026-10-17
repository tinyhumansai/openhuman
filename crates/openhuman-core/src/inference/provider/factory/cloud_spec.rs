//! `CloudProviderCreds` → `ProviderSpec` → `ProviderRegistry` mapping.
//!
//! The host owns saved configuration, routing and access gates; this module
//! hands the resolved route to `tinyinference-llm`'s [`ProviderRegistry`],
//! which owns adapter construction (Chat Completions, Anthropic Messages,
//! Codex Responses, OpenRouter routing options) and listing discovery. Only
//! host policy is expressed here: which auth style a slug uses, which extra
//! headers the ephemeral route and Codex OAuth add, and the TLS client for a
//! provider with a custom CA bundle.

use super::*;
use tinyinference_llm::providers::openai::codex::OpenAiCodexRouting;
use tinyinference_llm::providers::registry::{ProviderBuildOptions, ProviderRegistry};
use tinyinference_llm::{ProviderKind, ProviderSpec};

use crate::config::schema::cloud_providers::CloudProviderCreds;

/// Maps a configured cloud entry and its effective model onto the registry's
/// spec. Credentials are resolved by the host beforehand, so no env var is
/// named and a key is not required at the spec level (a blank key is a host
/// decision, made in `resolve_cloud_slug`).
pub(super) fn cloud_provider_spec(entry: &CloudProviderCreds, model: &str) -> ProviderSpec {
    ProviderSpec {
        kind: match entry.auth_style {
            AuthStyle::Anthropic => ProviderKind::Anthropic,
            _ => ProviderKind::Compatible,
        },
        provider: entry.slug.clone(),
        model: model.to_string(),
        base_url: entry.endpoint.clone(),
        api_key_env: None,
        requires_api_key: false,
    }
}

/// The registry auth style for a host [`AuthStyle`]; `None` for the managed
/// JWT style, which never reaches the registry.
pub(super) fn registry_auth_style(style: AuthStyle) -> Option<CompatAuthStyle> {
    match style {
        AuthStyle::Bearer => Some(CompatAuthStyle::Bearer),
        AuthStyle::Anthropic => Some(CompatAuthStyle::Anthropic),
        AuthStyle::None => Some(CompatAuthStyle::None),
        AuthStyle::OpenhumanJwt => None,
    }
}

/// Host-resolved inputs for [`build_cloud_model`].
pub(super) struct CloudBuild<'a> {
    pub(super) spec: ProviderSpec,
    pub(super) auth_style: CompatAuthStyle,
    pub(super) key: &'a str,
    pub(super) temperature_override: Option<f64>,
    pub(super) temperature_unsupported_models: &'a [String],
    pub(super) native_tool_calling: bool,
    pub(super) extra_headers: &'a [(String, String)],
    pub(super) extra_query_params: &'a [(String, String)],
    pub(super) user_agent: Option<&'a str>,
    pub(super) codex: Option<&'a OpenAiCodexRouting>,
    pub(super) http: Option<reqwest::Client>,
}

impl<'a> CloudBuild<'a> {
    fn options(&self) -> ProviderBuildOptions<'a> {
        ProviderBuildOptions {
            auth_style: Some(self.auth_style.clone()),
            temperature_override: self.temperature_override,
            temperature_unsupported_models: self.temperature_unsupported_models,
            native_tool_calling: Some(self.native_tool_calling),
            extra_headers: self.extra_headers,
            extra_query_params: self.extra_query_params,
            user_agent: self.user_agent,
            codex: self.codex,
            http: self.http.clone(),
            ..ProviderBuildOptions::default()
        }
    }

    fn registry(
        &self,
    ) -> ProviderRegistry<impl tinyinference_llm::providers::registry::CredentialResolver + '_>
    {
        let key = self.key.to_string();
        ProviderRegistry::new(move |_: &ProviderSpec| Ok(key.clone()))
    }
}

/// Builds the chat model for a resolved cloud route through the registry.
pub(super) fn build_cloud_model(build: CloudBuild<'_>) -> anyhow::Result<Arc<dyn ChatModel<()>>> {
    log::debug!(
        "[providers][cloud-spec] build slug={} kind={} auth={:?}",
        build.spec.provider,
        build.spec.kind.as_str(),
        build.auth_style
    );
    build
        .registry()
        .build_with(&build.spec, build.options())
        .map_err(|error| anyhow::anyhow!("[chat-factory] {error}"))
}

/// The registry's listing request for a resolved cloud route (headers, pinned
/// OpenRouter providers), or `None` when the route has no discoverable
/// listing.
pub(super) fn cloud_discovery_request(
    build: &CloudBuild<'_>,
) -> Option<tinyinference_llm::model::discover::DiscoveryRequest> {
    match build
        .registry()
        .discovery_request(&build.spec, &build.options())
    {
        Ok(request) => request,
        Err(error) => {
            log::debug!(
                "[model_limits] registry discovery unavailable slug={}: {error}",
                build.spec.provider
            );
            None
        }
    }
}

#[cfg(test)]
#[path = "cloud_spec_tests.rs"]
mod tests;
