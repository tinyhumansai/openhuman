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

// ── public surface (preserves the original `pub use ops::*` contract) ──

#[cfg(test)]
pub(crate) use http_error::config_rejection_demoted;
pub use http_error::{
    api_error, is_backend_auth_failure, is_backend_error_code_owned, log_byo_provider_auth_failure,
    publish_backend_session_expired, should_report_provider_http_failure,
};

pub use models::{
    list_configured_models, list_configured_models_from_config, synthesize_local_runtime_entry,
};

pub use provider_factory::{
    list_providers, ProviderInfo, ProviderRuntimeOptions, INFERENCE_BACKEND_ID,
};

// ── test re-exports for ops_tests.rs ──

#[cfg(test)]
pub(crate) use super::openhuman_backend_model;

// ── test companion ──

#[cfg(test)]
#[path = "../ops_tests.rs"]
mod tests;
