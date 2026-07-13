//! Crate-native OpenAI-compatible client construction (issue #4727, Motion B).
//!
//! The cutover replaces the in-house [`OpenAiCompatibleProvider`] wire client with
//! the vendored `tinyagents` crate's `OpenAiModel` — a `ChatModel` that speaks the
//! OpenAI Chat Completions wire and, since tinyagents #44/#47/#48, carries the
//! host-parity config the OpenHuman provider catalog needs: configurable auth
//! styles + static headers, per-model temperature suppression/override, and
//! system→user merging. `num_ctx` and other Ollama `options` ride
//! `ModelRequest.provider_options` (already supported upstream).
//!
//! This module is the **single boundary** where the host's resolved provider
//! config becomes a crate-native `ChatModel`. Bespoke providers that the crate
//! can't serve — the managed OpenHuman backend (session JWT + billing envelope),
//! `claude_code` / `claude_agent_sdk` (subprocess), and `openai_codex`
//! (`/v1/responses` + query-param auth) — stay as host `ChatModel` impls and do
//! **not** route through here.
//!
//! **Status: scaffolding.** The builder + auth mapping are complete and
//! unit-tested; wiring it as the factory's default construction path (and the
//! per-provider wire-parity validation that must precede deleting
//! `compatible*.rs`) is the follow-up within this cutover.

use std::sync::Arc;

use tinyagents::harness::model::ChatModel;
use tinyagents::harness::providers::openai::{AuthStyle as CrateAuthStyle, OpenAiModel};

use super::auth::AuthStyle as HostAuthStyle;

/// Map the host [`AuthStyle`](HostAuthStyle) to the crate's `AuthStyle`. The
/// variants are 1:1 (both were derived from the same OpenHuman provider catalog).
pub(crate) fn map_auth_style(host: HostAuthStyle) -> CrateAuthStyle {
    match host {
        HostAuthStyle::None => CrateAuthStyle::None,
        HostAuthStyle::Bearer => CrateAuthStyle::Bearer,
        HostAuthStyle::XApiKey => CrateAuthStyle::XApiKey,
        HostAuthStyle::Anthropic => CrateAuthStyle::Anthropic,
        HostAuthStyle::Custom(header) => CrateAuthStyle::Custom(header),
    }
}

/// The resolved config for one OpenAI-compatible provider, mirroring the inputs
/// the host [`build_compatible_provider`](super::factory) helper takes. Kept as a
/// struct (rather than a long positional arg list) so the factory maps its
/// resolved provider string + credentials + catalog flags in one place.
pub(crate) struct CrateOpenAiConfig<'a> {
    /// Provider family id (telemetry + normalized errors), e.g. `"openai"`.
    pub provider_name: &'a str,
    /// Base URL (no trailing slash needed; the crate trims it).
    pub endpoint: &'a str,
    /// API credential; empty is fine for [`AuthStyle::None`] (local runtimes).
    pub api_key: &'a str,
    /// How the credential is sent.
    pub auth_style: HostAuthStyle,
    /// Default model id baked onto the client (a per-call `ModelRequest.model`
    /// still overrides it).
    pub model: &'a str,
    /// Model-id `*`-glob patterns whose targets reject a `temperature` param.
    pub temperature_unsupported_models: &'a [String],
    /// Fixed temperature override for every call, when set.
    pub temperature_override: Option<f64>,
    /// Fold system messages into the first user message (endpoints w/o a
    /// `system` role).
    pub merge_system_into_user: bool,
    /// Static headers attached to every request (e.g. provider attribution).
    pub extra_headers: &'a [(String, String)],
    /// Override the model's advertised **native** tool-calling capability.
    /// `None` keeps the crate default (`true`); `Some(false)` is required for
    /// local runtimes (Ollama et al.) that reject the OpenAI `tools` parameter,
    /// so the harness embeds tool specs in the prompt instead. Maps to the crate
    /// `OpenAiModel::with_native_tool_calling`.
    pub native_tool_calling: Option<bool>,
    /// Override the model's advertised vision (image-in) capability. `None` keeps
    /// the crate default; `Some(false)` marks a text-only local model. Maps to
    /// `OpenAiModel::with_vision`.
    pub vision: Option<bool>,
    /// Provider options baked onto every request (e.g. Ollama's
    /// `{"options": {"num_ctx": 8192}}`), merged under each call's own
    /// `provider_options`. `None` bakes nothing. Maps to
    /// `OpenAiModel::with_default_provider_options`.
    pub default_provider_options: Option<serde_json::Value>,
    /// Route calls to the OpenAI **Responses API** (`/v1/responses`) instead of
    /// Chat Completions — the OpenAI Codex OAuth backend. Maps to
    /// `OpenAiModel::with_responses_api_primary`.
    pub responses_api_primary: bool,
    /// (Responses path) omit `max_output_tokens`, which the Codex backend
    /// rejects. Maps to `OpenAiModel::with_responses_omit_max_output_tokens`.
    pub responses_omit_max_output_tokens: bool,
    /// Static query parameters appended to every request URL (e.g. the Codex
    /// `client_version`). Maps to `OpenAiModel::with_extra_query_param`.
    pub extra_query_params: &'a [(String, String)],
    /// `User-Agent` header override (e.g. the Codex CLI UA). Maps to
    /// `OpenAiModel::with_user_agent`.
    pub user_agent: Option<&'a str>,
}

/// Build a crate-native `OpenAiModel` (`ChatModel`) for the given OpenAI-compatible
/// provider config — the cutover replacement for constructing an
/// `OpenAiCompatibleProvider`.
pub(crate) fn build_crate_openai_model(config: CrateOpenAiConfig<'_>) -> Arc<dyn ChatModel<()>> {
    let mut model = OpenAiModel::compatible_provider(
        config.provider_name,
        config.api_key,
        config.endpoint,
        config.model,
    )
    .with_auth_style(map_auth_style(config.auth_style));

    if !config.temperature_unsupported_models.is_empty() {
        model = model
            .with_temperature_unsupported_models(config.temperature_unsupported_models.to_vec());
    }
    if config.temperature_override.is_some() {
        model = model.with_temperature_override(config.temperature_override);
    }
    if config.merge_system_into_user {
        model = model.with_merge_system_into_user();
    }
    for (name, value) in config.extra_headers {
        model = model.with_header(name.clone(), value.clone());
    }
    // Capability toggles must be applied *after* provider/model are set (which
    // `compatible_provider` above already did), because those re-derive the
    // profile the toggles mutate.
    if let Some(enabled) = config.native_tool_calling {
        model = model.with_native_tool_calling(enabled);
    }
    if let Some(enabled) = config.vision {
        model = model.with_vision(enabled);
    }
    if let Some(options) = config.default_provider_options {
        model = model.with_default_provider_options(options);
    }
    for (name, value) in config.extra_query_params {
        model = model.with_extra_query_param(name.clone(), value.clone());
    }
    if let Some(user_agent) = config.user_agent {
        model = model.with_user_agent(user_agent);
    }
    if config.responses_api_primary {
        model = model.with_responses_api_primary();
    }
    if config.responses_omit_max_output_tokens {
        model = model.with_responses_omit_max_output_tokens();
    }

    Arc::new(model)
}

/// Factory-level crate-native builder — the drop-in parallel to the host
/// `make_openai_compatible_provider_with_config`, taking the same resolved
/// inputs (provider slug, endpoint, credential, host auth style, model, the
/// config temperature-suppression list + per-workload override) and returning a
/// crate `ChatModel` instead of a `Box<dyn Provider>`.
///
/// The cutover swaps each generic OpenAI-compatible construction site over to
/// this. `merge_system_into_user` is threaded per-provider (the catalog knows
/// which endpoints reject a `system` role); `supports_responses_fallback` has no
/// crate equivalent — providers that need `/v1/responses` (only `openai_codex`)
/// stay host-side and never call here.
#[allow(clippy::too_many_arguments)]
pub(crate) fn make_crate_openai_chat_model(
    provider_name: &str,
    endpoint: &str,
    api_key: &str,
    auth_style: HostAuthStyle,
    model: &str,
    temperature_unsupported_models: &[String],
    temperature_override: Option<f64>,
    merge_system_into_user: bool,
) -> Arc<dyn ChatModel<()>> {
    build_crate_openai_model(CrateOpenAiConfig {
        provider_name,
        endpoint,
        api_key,
        auth_style,
        model,
        temperature_unsupported_models,
        temperature_override,
        merge_system_into_user,
        extra_headers: &[],
        native_tool_calling: None,
        vision: None,
        default_provider_options: None,
        responses_api_primary: false,
        responses_omit_max_output_tokens: false,
        extra_query_params: &[],
        user_agent: None,
    })
}

/// Build a crate-native `ChatModel` for a **local OpenAI-compatible runtime**
/// (Ollama, LM Studio, MLX, OMLX, local-openai) — the crate-native counterpart
/// of the host `make_*_provider` local builders. Local runtimes reject the
/// OpenAI `tools` parameter and are text-only, so native tool calling and vision
/// are forced off; `num_ctx` (Ollama) rides baked provider options as
/// `{"options": {"num_ctx": N}}`, matching the host provider's wire shape.
#[allow(clippy::too_many_arguments)]
pub(crate) fn make_crate_local_runtime_chat_model(
    provider_name: &str,
    endpoint: &str,
    api_key: &str,
    auth_style: HostAuthStyle,
    model: &str,
    temperature_unsupported_models: &[String],
    temperature_override: Option<f64>,
    num_ctx: Option<u32>,
) -> Arc<dyn ChatModel<()>> {
    let default_provider_options = num_ctx.map(|n| {
        serde_json::json!({
            "options": { "num_ctx": n }
        })
    });
    build_crate_openai_model(CrateOpenAiConfig {
        provider_name,
        endpoint,
        api_key,
        auth_style,
        model,
        temperature_unsupported_models,
        temperature_override,
        // Local runtimes have a native `system` role; no merge needed.
        merge_system_into_user: false,
        extra_headers: &[],
        // Parity with the host local providers, which set these off.
        native_tool_calling: Some(false),
        vision: Some(false),
        default_provider_options,
        responses_api_primary: false,
        responses_omit_max_output_tokens: false,
        extra_query_params: &[],
        user_agent: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_every_host_auth_style_one_to_one() {
        assert_eq!(map_auth_style(HostAuthStyle::None), CrateAuthStyle::None);
        assert_eq!(
            map_auth_style(HostAuthStyle::Bearer),
            CrateAuthStyle::Bearer
        );
        assert_eq!(
            map_auth_style(HostAuthStyle::XApiKey),
            CrateAuthStyle::XApiKey
        );
        assert_eq!(
            map_auth_style(HostAuthStyle::Anthropic),
            CrateAuthStyle::Anthropic
        );
        assert_eq!(
            map_auth_style(HostAuthStyle::Custom("x-key".to_string())),
            CrateAuthStyle::Custom("x-key".to_string())
        );
    }

    #[test]
    fn builds_a_chat_model_with_the_configured_profile() {
        let model = build_crate_openai_model(CrateOpenAiConfig {
            provider_name: "deepseek",
            endpoint: "https://api.deepseek.com/v1",
            api_key: "secret",
            auth_style: HostAuthStyle::Bearer,
            model: "deepseek-chat",
            temperature_unsupported_models: &[],
            temperature_override: None,
            merge_system_into_user: false,
            extra_headers: &[],
            native_tool_calling: None,
            vision: None,
            default_provider_options: None,
            responses_api_primary: false,
            responses_omit_max_output_tokens: false,
            extra_query_params: &[],
            user_agent: None,
        });
        // The built model carries the configured provider + model on its profile.
        let profile = model.profile().expect("openai models expose a profile");
        assert_eq!(profile.provider.as_deref(), Some("deepseek"));
        assert_eq!(profile.model.as_deref(), Some("deepseek-chat"));
        assert!(profile.tool_calling);
    }

    #[test]
    fn factory_level_builder_carries_provider_and_model() {
        let model = make_crate_openai_chat_model(
            "groq",
            "https://api.groq.com/openai/v1",
            "secret",
            HostAuthStyle::Bearer,
            "llama-3.3-70b-versatile",
            &["o1*".to_string()],
            None,
            false,
        );
        let profile = model.profile().expect("openai models expose a profile");
        assert_eq!(profile.provider.as_deref(), Some("groq"));
        assert_eq!(profile.model.as_deref(), Some("llama-3.3-70b-versatile"));
    }

    #[test]
    fn builder_applies_local_none_auth_without_panicking() {
        // Local runtime shape: no auth, empty key, merge-system on.
        let _model = build_crate_openai_model(CrateOpenAiConfig {
            provider_name: "ollama",
            endpoint: "http://localhost:11434/v1",
            api_key: "",
            auth_style: HostAuthStyle::None,
            model: "llama3.2",
            temperature_unsupported_models: &["o1*".to_string()],
            temperature_override: Some(0.0),
            merge_system_into_user: true,
            extra_headers: &[("X-Attr".to_string(), "openhuman".to_string())],
            native_tool_calling: Some(false),
            vision: Some(false),
            default_provider_options: None,
            responses_api_primary: false,
            responses_omit_max_output_tokens: false,
            extra_query_params: &[],
            user_agent: None,
        });
    }

    #[test]
    fn local_runtime_builder_disables_native_tools_and_vision() {
        let model = make_crate_local_runtime_chat_model(
            "ollama",
            "http://localhost:11434/v1",
            "",
            HostAuthStyle::None,
            "qwen2.5",
            &[],
            None,
            Some(8192),
        );
        let profile = model.profile().expect("openai models expose a profile");
        assert_eq!(profile.provider.as_deref(), Some("ollama"));
        assert_eq!(profile.model.as_deref(), Some("qwen2.5"));
        // Local runtimes must not advertise native tools or vision.
        assert!(!profile.tool_calling);
        assert!(!profile.modalities.image_in);
    }
}
