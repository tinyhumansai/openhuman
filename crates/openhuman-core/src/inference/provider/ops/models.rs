//! Model listing (`list_configured_models` and friends) for a configured
//! provider.
//!
//! Sub-modules, by responsibility:
//! - [`types`] — the [`ModelInfo`] catalog-entry type.
//! - [`catalog_listing`] — the `list_configured_models*` request/response
//!   pipeline.
//! - [`managed_provider`] — the managed (`openhuman`) entry synthesis and
//!   session-safety helpers `list_configured_models_from_config` needs.
//! - [`local_runtime`] — synthesized entries for `ollama` / `lmstudio` when
//!   the user has no matching `cloud_providers` row.
//! - [`openrouter`] — OpenRouter detection and API-key validation.
//! - [`parsing`] — parsing the `/models` response envelope into
//!   [`ModelInfo`] entries.

use super::super::openai_codex::resolve_openai_codex_routing;
use tinyinference_core::sanitize::sanitize_api_error;
use tinyinference_llm::providers::openai::codex::{
    openai_codex_client_version, openai_codex_user_agent, OpenAiCodexRouting,
    OPENAI_CODEX_ACCOUNT_HEADER, OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};

mod catalog_listing;
mod local_runtime;
mod openrouter;
use tinyinference_llm::catalog::{
    append_query_param, endpoint_hosts_models_listing, is_openrouter_provider,
    merge_openai_codex_model_hints, parse_models_response, ModelInfo,
};

#[cfg(test)]
use catalog_listing::resolve_local_runtime_key;

pub use catalog_listing::{list_configured_models, list_configured_models_from_config};
pub use local_runtime::synthesize_local_runtime_entry;

// Cross-submodule wiring: each submodule reaches these through `use super::*;`
// (mirrors the pre-split `include!`-shared scope).
use openrouter::validate_openrouter_api_key;

#[cfg(test)]
#[path = "models_tests.rs"]
mod tests;
