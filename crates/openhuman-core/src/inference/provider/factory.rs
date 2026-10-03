//! Unified chat-provider factory.
//!
//! Resolves workload names (e.g. `"reasoning"`, `"summarization"`) to a
//! crate-native `ChatModel` plus the concrete model id selected for a workload.
//!
//! ## Provider-string grammar
//!
//! ```text
//! "openhuman"                    → OpenHumanBackendModel; model = config.default_model
//! "cloud" / missing              → primary_cloud; legacy custom inference_url wins when
//!                                  primary still points at OpenHuman after migration
//! "ollama:<model>[@<temp>]"      → local Ollama at config.local_ai.base_url
//! "lmstudio:<model>[@<temp>]"    → local LM Studio
//! "mlx:<model>[@<temp>]"         → local MLX-compatible server
//! "local-openai:<model>[@<temp>]"→ generic local OpenAI-compatible
//! "<slug>:<model>[@<temp>]"      → cloud_providers entry keyed by slug;
//!                                  builds the crate-native OpenAI client (Bearer) or
//!                                  Anthropic flavour depending on auth_style.
//! ```
//!
//! The optional `@<temp>` suffix pins a per-workload temperature override on
//! the built provider. The model id sent upstream never includes the suffix.
//!
//! Unknown slugs and missing-creds configurations produce actionable errors.
//!
//! ## Module layout
//!
//! - [`tiers`] — managed tier / `hint:*` vocabulary and capability lookups.
//! - [`routing`] — role → configured provider string resolution.
//! - [`primary_cloud`] — `primary_cloud` / legacy `inference_url` resolution.
//! - [`access_gates`] — privacy-mode, session, and egress chokepoints.
//! - [`credentials`] — auth-profile key lookup for cloud slugs.
//! - [`managed_backend`] — the managed OpenHuman backend model.
//! - [`local_runtime`] — Ollama / LM Studio / MLX / OMLX / local-openai.
//! - [`cloud_slug`] — `<slug>:<model>` BYOK cloud providers.
//! - [`subprocess_providers`] — Claude Agent SDK and Claude Code CLI.
//! - [`chat_model`] — the one-shot `create_chat_model*` entry points.
//! - [`turn_model`] — the per-turn `create_turn_chat_model*` entry points.

pub(crate) mod access_gates;
mod chat_model;
mod cloud_slug;
mod credentials;
mod discovery;
mod local_runtime;
mod managed_backend;
mod primary_cloud;
mod routing;
mod subprocess_providers;
mod tiers;
mod turn_model;

pub(crate) use access_gates::current_host_requires_session;
pub(crate) use chat_model::resolves_to_managed_backend;
pub use chat_model::{
    create_chat_model, create_chat_model_from_string, create_chat_model_from_string_with_model_id,
    create_chat_model_with_model_id, probe_inference_readiness,
};
pub(crate) use credentials::openai_bearer_is_oauth;
pub use credentials::{auth_key_for_slug, lookup_key_for_slug, redact_endpoint};
pub(crate) use discovery::model_limits_request;
pub(crate) use local_runtime::create_local_chat_model_from_string;
pub(crate) use managed_backend::make_openhuman_backend_model;
pub(crate) use routing::role_uses_implicit_cloud_fallback;
pub use routing::{provider_for_role, role_bypasses_managed_credits};
pub(crate) use tiers::{
    is_known_openhuman_tier, is_raw_passthrough_model, oh_tier_supports_vision,
};
pub use tiers::{resolve_model_for_hint, role_for_model_tier};
pub(crate) use turn_model::{
    create_turn_chat_model, create_turn_chat_model_from_string,
    create_turn_chat_model_from_string_with_native_tools_and_route,
    create_turn_chat_model_with_native_tools_and_route,
};

// Factory-internal helpers shared across the submodules (and their tests)
// through `use super::*`.
use access_gates::{emit_inference_egress, enforce_local_only_inference};
use chat_model::{unresolved_chat_model_error, with_default_temperature};
use cloud_slug::{
    resolve_cloud_slug, try_create_cloud_slug_chat_model,
    try_create_cloud_slug_chat_model_from_string,
    try_create_cloud_slug_chat_model_with_native_tools,
};
use local_runtime::{
    try_create_local_runtime_chat_model, try_create_local_runtime_chat_model_from_string,
    OptionalChatModelResult,
};
pub(crate) use managed_backend::make_openhuman_backend_model_for_thread;
use managed_backend::{resolve_managed_backend, resolve_managed_backend_with_model_override};
use primary_cloud::{legacy_inference_slug, resolve_primary_cloud_provider_string};
use routing::split_model_and_temperature;
use subprocess_providers::{
    prepare_claude_agent_sdk_chat_model, try_create_claude_agent_sdk_chat_model,
    try_create_claude_agent_sdk_chat_model_from_string, try_create_claude_code_chat_model,
    try_create_claude_code_chat_model_from_string,
};
use tiers::{is_abstract_tier_model, managed_default_model};

/// Test-only seam: inject a mock [`ChatModel`] so e2e tests can drive the
/// autonomous run paths (for example `spawn_workflow_run_background`)
/// with a scripted LLM and no network. Process-global because those runs are
/// detached `tokio::spawn`s — a thread/task-local would not reach them.
///
/// Because it is global, tests that install an override MUST run serially
/// and clear it via the returned guard. Inert in production: the check below
/// is gated on `cfg(test)` or an off-by-default test/profiling feature,
/// so the override is never consulted in shipped builds.
#[cfg(any(test, feature = "e2e-test-support", feature = "rss-bench"))]
#[path = "factory_test_provider_override_tests.rs"]
pub mod test_provider_override;

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "factory_tests.rs"]
mod factory_tests;

use crate::config::schema::cloud_providers::AuthStyle;
use crate::config::Config;
use crate::inference::provider::openai_codex::resolve_openai_codex_routing;
use crate::inference::provider::openhuman_backend_model::OpenHumanBackendModel;
use crate::inference::provider::ProviderRuntimeOptions;
use crate::security::credentials::AuthService;
use std::sync::Arc;
use tinyagents_harness::providers::claude_agent_sdk::ClaudeAgentSdkProvider;
use tinyinference_llm::model::{ChatModel, ModelRequest, ModelResponse, ModelStream};
use tinyinference_llm::providers::openai::codex::{
    openai_codex_client_version, openai_codex_user_agent, OPENAI_CODEX_ACCOUNT_HEADER,
    OPENAI_CODEX_ORIGINATOR, OPENAI_CODEX_ORIGINATOR_HEADER,
};
use tinyinference_llm::providers::openai::AuthStyle as CompatAuthStyle;

/// Sentinel meaning "use the OpenHuman backend session JWT".
pub const PROVIDER_OPENHUMAN: &str = "openhuman";
/// Prefix for Ollama-local providers: `"ollama:<model>"`.
pub const OLLAMA_PROVIDER_PREFIX: &str = "ollama:";
/// Prefix for LM Studio-local providers: `"lmstudio:<model>"`.
pub const LM_STUDIO_PROVIDER_PREFIX: &str = "lmstudio:";
/// Prefix for MLX-compatible local providers: `"mlx:<model>"`.
pub const MLX_PROVIDER_PREFIX: &str = "mlx:";
/// Prefix for OMLX local providers: `"omlx:<model>"`.
pub const OMLX_PROVIDER_PREFIX: &str = "omlx:";
/// Prefix for generic local OpenAI-compatible providers: `"local-openai:<model>"`.
pub const LOCAL_OPENAI_PROVIDER_PREFIX: &str = "local-openai:";
/// Prefix for the Claude Agent SDK subprocess provider: `"claude_agent_sdk:<model>"`.
pub const CLAUDE_AGENT_SDK_PREFIX: &str = "claude_agent_sdk:";
/// Sentinel for the Claude Agent SDK provider without a model suffix.
pub const CLAUDE_AGENT_SDK_PROVIDER: &str = "claude_agent_sdk";
/// Sentinel returned when a user has expressed custom/BYOK inference intent
/// (via a non-openhuman `inference_url`) but no matching `cloud_providers`
/// entry was found. Passed through `provider_for_role` and caught early in
/// `create_chat_model_from_string` to produce a clear configuration error
/// instead of silently routing through the managed OpenHuman backend.
pub const BYOK_INCOMPLETE_SENTINEL: &str = "__byok_incomplete__";
