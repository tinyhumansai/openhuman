//! Where to ask a provider for a model's limits: the [`DiscoveryRequest`] for a
//! role's provider string, built from the same endpoint and credential
//! resolution the chat factory uses, so the discovered window describes the
//! endpoint the turn actually talks to.

use super::*;
use tinyinference_llm::model::discover::DiscoveryRequest;

/// The discovery request for `provider` (a role's resolved provider string)
/// serving `model`, or `None` when the route has no listing to ask:
///
/// - `openhuman` / an `OpenhumanJwt` entry: the registered managed catalog's
///   limits request, keyed by the managed inference base so overflow
///   corrections from the managed chat model land on the same entry.
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

/// The managed backend's catalogue listing request, from the registered
/// catalog's limits function (`inference::registry`). With none registered the
/// managed route has no listing to ask.
fn managed_limits_request(model: &str, config: &Config) -> Option<DiscoveryRequest> {
    let Some(limits) =
        crate::inference::registry::catalog(PROVIDER_OPENHUMAN).and_then(|entry| entry.limits)
    else {
        log::debug!("[model_limits] no managed catalog registered");
        return None;
    };
    log::debug!("[model_limits] managed limits served by a registered catalog");
    limits(model, config)
}

#[cfg(test)]
#[path = "discovery_tests.rs"]
mod tests;
