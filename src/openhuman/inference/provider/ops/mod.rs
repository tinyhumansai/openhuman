//! Provider operations — split from a single `ops.rs` into sub-modules.
//!
//! Sub-modules:
//! - `sanitize`         — secret scrubbing, error formatting
//! - `http_error`       — HTTP error classification, Sentry routing, `api_error`
//! - `models`           — model listing (`list_configured_models`, parsing)
//! - `provider_factory` — provider construction (`create_*`, `ProviderRuntimeOptions`)

mod http_error;
mod models;
mod provider_factory;
mod sanitize;

// ── public surface (preserves the original `pub use ops::*` contract) ──

pub use sanitize::{
    format_anyhow_chain, format_error_chain, sanitize_api_error, scrub_secret_patterns,
    MAX_API_ERROR_CHARS,
};

pub use http_error::{
    api_error, is_backend_auth_failure, is_backend_error_code_owned, is_budget_exhausted_http_400,
    is_context_window_exceeded_message, is_custom_openai_upstream_bad_request_http_400,
    is_provider_access_policy_denied_http_403, is_provider_config_rejection_http,
    log_backend_error_code_owned, log_budget_exhausted_http_400, log_context_window_exceeded,
    log_custom_openai_upstream_bad_request_http_400, log_provider_access_policy_denied_http_403,
    log_provider_config_rejection, publish_backend_session_expired,
    should_report_provider_http_failure,
};

pub use models::{
    append_query_param, is_openrouter_provider, list_configured_models,
    list_configured_models_from_config, merge_openai_codex_model_hints, model_items_from_body,
    parse_models_response, synthesize_local_runtime_entry, ModelInfo,
};

pub use provider_factory::{
    canonical_china_provider_name, create_backend_inference_provider,
    create_intelligent_routing_provider, create_resilient_provider,
    create_resilient_provider_with_options, create_routed_provider,
    create_routed_provider_with_options, is_glm_alias, is_minimax_alias, is_moonshot_alias,
    is_qianfan_alias, is_qwen_alias, is_qwen_oauth_alias, is_zai_alias, list_providers,
    ProviderInfo, ProviderRuntimeOptions, INFERENCE_BACKEND_ID,
};

// ── test re-exports for ops_tests.rs ──

#[cfg(test)]
pub(crate) use super::openai_codex::openai_codex_client_version;
#[cfg(test)]
pub(crate) use super::openhuman_backend;

// ── test companion ──

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
