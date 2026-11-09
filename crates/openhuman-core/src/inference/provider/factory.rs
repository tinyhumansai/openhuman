//! Host adapter for provider construction and access policy. Provider route
//! grammar and route-kind classification live in TinyInference.
//!
//! - [`routing`] — role → configured provider string resolution.
//! - [`access_gates`] — privacy-mode, session, and egress chokepoints.
//! - [`credentials`] — auth-profile key lookup for cloud slugs.
//! - [`managed_backend`] — the managed provider resolution (tier pinning, privacy gate, egress) in front of the registered chat model.
//! - [`local_runtime`] — Ollama / LM Studio / MLX / OMLX / local-openai.
//! - [`cloud_slug`] — `<slug>:<model>` BYOK cloud providers.
//! - [`cloud_spec`] — `CloudProviderCreds` → `ProviderSpec` → `ProviderRegistry`.
//! - [`subprocess_providers`] — Claude Agent SDK and Claude Code CLI.
//! - [`chat_model`] — the one-shot `create_chat_model*` entry points.

pub(crate) mod access_gates;
mod chat_model;
mod cloud_slug;
mod cloud_spec;
mod credentials;
mod inspection;
mod local_runtime;
mod managed_backend;
mod routing;
mod subprocess_providers;

pub use chat_model::{
    create_chat_model_from_string, create_chat_model_from_string_with_model_id,
    create_chat_model_in, create_chat_model_with_model_id_in, probe_inference_readiness,
};
pub(crate) use chat_model::{resolves_to_managed_backend, OpenHumanModelFactory};
pub(crate) use cloud_spec::{cloud_provider_spec, CloudCredentialResolver};
pub use credentials::{auth_key_for_slug, lookup_key_for_slug};
pub(crate) use credentials::{
    openai_bearer_is_oauth, redact_endpoint_origin, resolve_codex_routing,
};
pub(crate) use credentials::{resolve_cloud_slug_for, RuntimeCredentialSnapshot};
pub(crate) use inspection::{
    create_chat_model_with_identity_in_for_inspection,
    create_chat_model_with_model_id_in_for_inspection,
};
pub(crate) use local_runtime::create_local_chat_model_from_string;
pub(crate) use routing::fallback_diagnostics_for_role;
pub use routing::{provider_for_role, role_bypasses_managed_credits};
pub(crate) use routing::{workload_router, ConfigRouteSource};

// Factory-internal helpers shared across the implementation modules.
use access_gates::enforce_local_only_inference;
#[cfg(test)]
use cloud_slug::try_create_cloud_slug_chat_model;
pub(crate) use managed_backend::make_openhuman_backend_model_for_thread_in;
use tinyinference_providers::route::split_model_and_temperature;

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
use crate::security::egress::EgressLedger;
use std::sync::Arc;
use tinyinference_llm::model::ChatModel;

/// Opaque per-report identity for an inspection completion. It contains the
/// effective endpoint, exact frozen credential, and actual wire model, but
/// deliberately has no Debug/Display implementation.
#[derive(Clone, Eq, Hash, PartialEq)]
pub(crate) struct InspectionRequestIdentity {
    endpoint: String,
    credential: String,
    wire_model: String,
}

impl InspectionRequestIdentity {
    pub(crate) fn new(endpoint: String, credential: String, wire_model: String) -> Self {
        Self {
            endpoint,
            credential,
            wire_model,
        }
    }
}

/// Whether the resolved inspection route can enforce the requested output
/// token ceiling on the provider request.
#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum InspectionOutputBudgetCapability {
    /// The provider request enforces its requested output-token limit.
    Bounded,
    /// The provider route cannot express a bounded output-token request.
    Unsupported,
}

/// Result shape shared by inspection-only provider constructors.
type InspectionChatModelResult =
    Option<anyhow::Result<(Arc<dyn ChatModel<()>>, String, String, String)>>;

/// Prepared, frozen model plus its opaque deduplication identity.
pub(crate) struct PreparedInspectionChatModel {
    pub(crate) model: Arc<dyn ChatModel<()>>,
    pub(crate) model_id: String,
    pub(crate) identity: InspectionRequestIdentity,
    pub(crate) output_budget: InspectionOutputBudgetCapability,
}

/// Sentinel meaning "use the OpenHuman backend session JWT".
pub const PROVIDER_OPENHUMAN: &str = "openhuman";
/// Provider id used for the managed OpenHuman inference backend.
pub const INFERENCE_BACKEND_ID: &str = PROVIDER_OPENHUMAN;
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
