//! Where to ask a provider for a model's limits: the [`DiscoveryRequest`] for a
//! role's provider string, built from the same endpoint and credential
//! resolution the chat factory uses, so the discovered window describes the
//! endpoint the turn actually talks to.

use super::*;
use tinyinference_llm::model::discover::DiscoveryRequest;

/// The discovery request for `provider` (a role's resolved provider string)
/// serving `model`, or `None` when the route has no listing to ask:
///
/// - `openhuman` / an `OpenhumanJwt` entry: the managed backend's catalog
///   listing (`/openai/v1/models?catalog=openrouter`), keyed by the managed
///   inference base so overflow corrections from the managed chat model land
///   on the same entry.
/// - `<slug>:<model>`: the cloud entry's OpenAI-compatible `{endpoint}/models`
///   with its stored key (Bearer, or `x-api-key` for Anthropic).
/// - Local runtimes, subprocess providers, ChatGPT-OAuth (Codex) routing and
///   the BYOK-incomplete sentinel: `None` (they have their own profiles, or no
///   listing that reports windows).
pub(crate) fn model_limits_request(
    role: &str,
    provider: &str,
    model: &str,
    config: &Config,
) -> Option<DiscoveryRequest> {
    let p = provider.trim();
    if p.is_empty() || p == "cloud" || p == PROVIDER_OPENHUMAN {
        return managed_limits_request(model, config);
    }
    if p == BYOK_INCOMPLETE_SENTINEL
        || p == CLAUDE_AGENT_SDK_PROVIDER
        || p.starts_with(CLAUDE_AGENT_SDK_PREFIX)
        || p.starts_with(tinyagents_harness::providers::claude_code::PROVIDER_PREFIX)
        || tinyinference_local::profile::is_local_provider_string(p)
    {
        log::debug!("[model_limits] provider route has no discoverable listing role={role}");
        return None;
    }
    let (slug, raw) = p.split_once(':')?;
    let slug = slug.trim();
    let entry = config.cloud_providers.iter().find(|e| e.slug == slug)?;
    if entry.auth_style == AuthStyle::OpenhumanJwt {
        let (raw_model, _) = split_model_and_temperature(raw);
        let model = if raw_model.trim().is_empty() {
            model
        } else {
            raw_model.trim()
        };
        return managed_limits_request(model, config);
    }

    let (raw_model, _) = split_model_and_temperature(raw);
    let resolution = match resolve_cloud_slug(role, slug, &raw_model, config) {
        Ok(resolution) => resolution,
        Err(error) => {
            log::debug!(
                "[model_limits] cloud slug unresolvable for discovery role={role} slug={slug}: {error}"
            );
            return None;
        }
    };
    if resolution.codex.using_oauth {
        // The ChatGPT backend's catalogue is a different shape and carries no
        // per-model window; the Codex model hints cover it.
        log::debug!("[model_limits] codex oauth routing has no window listing slug={slug}");
        return None;
    }
    let auth_style = super::cloud_spec::registry_auth_style(resolution.entry.auth_style)?;
    let (extra_headers, temperature_unsupported) = (Vec::new(), Vec::new());
    super::cloud_spec::cloud_discovery_request(&super::cloud_spec::CloudBuild {
        spec: super::cloud_spec::cloud_provider_spec(resolution.entry, &resolution.effective_model),
        auth_style,
        key: resolution.key.as_str(),
        temperature_override: None,
        temperature_unsupported_models: &temperature_unsupported,
        native_tool_calling: true,
        extra_headers: &extra_headers,
        extra_query_params: &[],
        user_agent: None,
        codex: Some(&resolution.codex),
        http: None,
    })
}

/// The managed backend's catalogue listing, authenticated with the backend
/// credential when the URL is safe to carry it.
fn managed_limits_request(model: &str, config: &Config) -> Option<DiscoveryRequest> {
    use crate::inference::provider::openhuman_backend_model::{
        is_managed_endpoint_for_api_key, is_safe_endpoint_for_managed_bearer,
    };
    use crate::security::credentials::session_support::{
        resolve_backend_credential, BackendCredential,
    };

    let model = model.trim();
    if model.is_empty() {
        return None;
    }
    let inference_base = crate::backend::inference_base_url(&config.api_url).ok()?;
    let endpoint = format!("{}/openai/v1", inference_base.trim_end_matches('/'));
    let api_base = crate::backend::base_url(&config.api_url).ok()?;
    let listing_url = tinyinference_llm::catalog::append_query_param(
        &crate::util::url::join_url(&api_base, "/openai/v1/models"),
        "catalog",
        "openrouter",
    );
    let mut request = DiscoveryRequest::new(endpoint, model)
        .with_listing_url(listing_url.clone())
        .with_single_model_probe(false);
    match resolve_backend_credential(config) {
        Ok(BackendCredential::ApiKey(key)) if is_managed_endpoint_for_api_key(&listing_url) => {
            request = request.with_header("Authorization", format!("Bearer {key}"));
        }
        Ok(BackendCredential::Session(token))
            if is_safe_endpoint_for_managed_bearer(&listing_url) =>
        {
            request = request.with_header("Authorization", format!("Bearer {token}"));
        }
        Ok(_) => {
            log::warn!(
                "[model_limits] refusing to send the backend credential to an unsafe listing url"
            );
        }
        Err(_) => {
            log::debug!("[model_limits] no backend credential; managed listing probed anonymously");
        }
    }
    for (name, value) in crate::backend::attribution_headers().iter() {
        if let Ok(value) = value.to_str() {
            request = request.with_header(name.as_str(), value);
        }
    }
    Some(request)
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
