//! Synthesized `cloud_providers` entries for well-known local-runtime slugs.

/// Synthesize a transient [`CloudProviderCreds`] entry for the well-known
/// local-runtime slugs (`ollama`, `lmstudio`) so [`super::list_configured_models`]
/// can probe their OpenAI-compatible `/v1/models` endpoint even when the
/// user has not registered a matching `cloud_providers` row.
///
/// Background: the AI settings panel registers an `ollama` `cloud_providers`
/// entry when the user configures Ollama (see comment on
/// [`crate::config::schema::cloud_providers::is_slug_reserved`]),
/// but in practice some users hit
/// `inference_list_models("ollama")` without that entry — config drift,
/// flush-vs-probe race, or upgrade from a build that only persisted
/// `config.local_ai.base_url`. Sentry TAURI-RUST-28Z captures this:
/// 24 events / 7d, all `domain=rpc, method=openhuman.inference_list_models,
/// operation=invoke_method`. Without this fallback, the dropdown surfaces
/// the bare `"no cloud provider with id or slug 'ollama' found"` error
/// (also visible in the Sentry breadcrumb) instead of returning models.
///
/// Returns `None` for any slug that is not a recognized local-runtime
/// alias — callers continue down the normal "no cloud provider" error
/// path for `openai` / `anthropic` / opaque ids / typos.
pub fn synthesize_local_runtime_entry(
    slug: &str,
    config: &crate::config::Config,
) -> Option<crate::config::schema::cloud_providers::CloudProviderCreds> {
    use crate::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};

    let endpoint = match slug {
        // Ollama's OpenAI-compatible surface at `<base>/v1/models` returns
        // the same `{"data": [...]}` shape the existing parser handles, so
        // we route through that rather than the native `/api/tags`.
        "ollama" => {
            let base = tinyinference_local::ollama::ollama_base_url_from_override(
                config.local_ai.base_url.as_deref(),
            );
            format!("{}/v1", base.trim_end_matches('/'))
        }
        // `lm_studio_base_url` already ends in `/v1`.
        "lmstudio" => {
            tinyinference_local::lm_studio::lm_studio_base_url(config.local_ai.base_url.as_deref())
        }
        _ => return None,
    };

    Some(CloudProviderCreds {
        id: format!("synthetic_local_{slug}"),
        slug: slug.to_string(),
        label: slug.to_string(),
        endpoint,
        // Local runtimes accept unauthenticated requests on loopback.
        // The probe at `<endpoint>/models` runs without an Authorization
        // header — `lookup_key_for_slug` may still return a key, but
        // `AuthStyle::None` ignores it (see auth-style match below).
        auth_style: AuthStyle::None,
        default_model: None,
    })
}
