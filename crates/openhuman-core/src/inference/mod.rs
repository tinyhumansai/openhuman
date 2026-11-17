//! OpenHuman inference integration domain.
//!
//! TinyInference owns reusable model/provider behavior. This module owns the
//! product-facing configuration, policy, and RPC seams. The local runtime
//! (Ollama, LM Studio, MLX, OMLX, any OpenAI-compatible server) is installed,
//! started, and stocked with models by the user; OpenHuman only talks to the
//! configured endpoint and never downloads models or manages that process:
//! - `host_runtime/` — OpenHuman policy, RPC, and speech bindings around
//!                     `tinyinference-local`
//! - `provider/` — native chat models, cloud/local routing, auth and errors
//!                 (was `providers/`)
//! - `http/`     — OpenAI-compatible `/v1/chat/completions` endpoint
//! - `tokenjuice/` — host adapter for the `tinyjuice` token-compression module
//!
//! The RPC surface is `inference.*`; old `local_ai_*` RPC names are resolved
//! by the legacy alias layer for backwards compatibility.

/// `true` when the crate was compiled with the `inference` feature (the
/// default), i.e. the `cpal` audio-device stack is linked. Lets tests and
/// callers distinguish a slim/headless build from the desktop build without
/// naming gated symbols. When `false`, `cpal` is dropped from the dependency
/// graph (verify with `cargo tree -i cpal`) and the microphone-permission probe
/// reports `Unknown`.
pub const INFERENCE_COMPILED_IN: bool = cfg!(feature = "inference");

pub mod auth_error_registry;
pub(crate) mod context_window;
pub mod embedding_host;
pub(crate) mod failure_copy;
pub(crate) mod host_controls;
pub mod host_runtime;
pub mod http;
pub mod model_context;
pub mod ops;
pub mod provider;
pub mod provider_model;
pub mod readiness;
pub(crate) mod reasoning;
pub mod registry;
pub(crate) mod schema_fields;
mod schemas;
pub mod tokenjuice;
pub mod turn_models;

pub use schemas::{
    all_controller_schemas as all_inference_controller_schemas,
    all_registered_controllers as all_inference_registered_controllers,
};

// Re-export the types that external callers (voice, agent, etc.) import from inference
pub use host_runtime::all_local_inference_controller_schemas;
pub use host_runtime::all_local_inference_registered_controllers;
pub use model_context::context_window_for_model;
use tinyinference_local::status::LocalAiStatus;

/// Wire label for the local vision mode: `"ondemand"` when a vision model is
/// configured, `"disabled"` otherwise. Mirrors TinyInference's own label: the
/// user's runtime serves the model on demand; OpenHuman never preloads or
/// pulls it.
pub fn local_vision_mode(config: &crate::config::Config) -> &'static str {
    if config.local_ai.vision_model_id.trim().is_empty() {
        "disabled"
    } else {
        "ondemand"
    }
}

/// Builds the TinyInference disabled snapshot from OpenHuman's configuration.
pub fn disabled_local_ai_status(config: &crate::config::Config) -> LocalAiStatus {
    LocalAiStatus::disabled(config, local_vision_mode(config))
}

/// Projects OpenHuman configuration into TinyInference's local-runtime input.
pub fn local_runtime_config(
    config: &crate::config::Config,
) -> tinyinference_local::service::RuntimeConfig {
    let local = &config.local_ai;
    tinyinference_local::service::RuntimeConfig {
        local_ai: tinyinference_local::service::LocalRuntimeSettings {
            runtime_enabled: local.runtime_enabled,
            provider: local.provider.clone(),
            base_url: local.base_url.clone(),
            api_key: local.api_key.clone(),
            model_id: local.model_id.clone(),
            chat_model_id: local.chat_model_id.clone(),
            vision_model_id: local.vision_model_id.clone(),
            embedding_model_id: local.embedding_model_id.clone(),
            stt_model_id: local.stt_model_id.clone(),
            tts_voice_id: local.tts_voice_id.clone(),
            autosummary_debounce_ms: local.autosummary_debounce_ms,
            opt_in_confirmed: local.opt_in_confirmed,
            num_ctx: local.num_ctx,
        },
        workspace_dir: config.workspace_dir.clone(),
        config_path: config.config_path.clone(),
        shared_root_dir: crate::config::default_root_openhuman_dir()
            .unwrap_or_else(|_| config.workspace_dir.clone()),
        default_temperature: config.default_temperature,
    }
}

impl tinyinference_local::models::LocalModelConfig for crate::config::Config {
    fn local_provider_name(&self) -> &str {
        &self.local_ai.provider
    }
    fn local_chat_model_id(&self) -> &str {
        &self.local_ai.chat_model_id
    }
    fn local_legacy_model_id(&self) -> &str {
        &self.local_ai.model_id
    }
    fn local_vision_model_id(&self) -> &str {
        &self.local_ai.vision_model_id
    }
    fn local_embedding_model_id(&self) -> &str {
        &self.local_ai.embedding_model_id
    }
    fn local_stt_model_id(&self) -> &str {
        &self.local_ai.stt_model_id
    }
    fn local_tts_voice_id(&self) -> &str {
        &self.local_ai.tts_voice_id
    }
}

// Test helpers (re-exported for sibling test files that use inference_test_guard)
#[cfg(test)]
pub(crate) fn inference_test_guard() -> tokio::sync::MutexGuard<'static, ()> {
    host_runtime::inference_test_guard()
}

#[cfg(test)]
pub(crate) async fn inference_test_guard_async() -> tokio::sync::MutexGuard<'static, ()> {
    host_runtime::inference_test_guard_async().await
}
