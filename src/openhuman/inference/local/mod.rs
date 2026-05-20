//! Local AI runtime — Ollama, LM Studio, Whisper, Piper sub-process management.
//!
//! This module was previously `src/openhuman/local_ai/`. It now lives under
//! `inference/local/` so all inference concerns share a single domain root.

#[cfg(test)]
pub(crate) static INFERENCE_TEST_MUTEX: once_cell::sync::Lazy<std::sync::Mutex<()>> =
    once_cell::sync::Lazy::new(|| std::sync::Mutex::new(()));

#[cfg(test)]
pub(crate) fn inference_test_guard() -> std::sync::MutexGuard<'static, ()> {
    INFERENCE_TEST_MUTEX
        .lock()
        .unwrap_or_else(|p| p.into_inner())
}

mod core;
pub mod ops;
mod schemas;

// Re-expose inference-level modules under `local::` so that files that
// were moved from `local_ai/` and used `super::model_ids` etc. continue
// to compile without rewriting every callsite.
pub use super::device;
pub use super::model_ids;
pub use super::parse;
pub use super::paths;
pub use super::presets;
pub use super::sentiment;
pub use super::types;

pub mod install;
pub(crate) mod install_piper;
pub(crate) mod install_whisper;
pub(crate) mod lm_studio;
pub(crate) mod model_requirements;
mod ollama;
mod process_util;
pub(crate) mod provider;
pub(crate) use model_requirements::{evaluate_context, ContextEligibility, MIN_CONTEXT_TOKENS};
pub(crate) use ollama::{ollama_base_url, OLLAMA_BASE_URL};
pub mod service;
pub(crate) mod voice_install_common;

pub use core::*;
pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_local_ai_controller_schemas,
    all_registered_controllers as all_local_ai_registered_controllers,
};
pub(crate) use service::whisper_engine;
pub use service::LocalAiService;
