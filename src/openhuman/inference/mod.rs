//! Unified inference domain.
//!
//! This module is the canonical home for all inference concerns:
//! - `local/`    — Ollama / LM Studio / Whisper / Piper runtime management
//!                 (was `src/openhuman/local_ai/`)
//! - `provider/` — cloud + local provider trait, routing, reliability
//!                 (was `src/openhuman/providers/`)
//! - `voice/`    — transcription (STT) and TTS inference implementations
//!                 (moved from `src/openhuman/voice/`)
//! - `http/`     — OpenAI-compatible `/v1/chat/completions` endpoint
//!
//! The RPC surface remains under the `inference.*` and `local_ai.*` namespaces
//! for backwards compatibility.

pub mod device;
pub mod http;
pub mod local;
pub mod model_ids;
pub mod ops;
pub mod parse;
pub mod paths;
pub mod presets;
pub mod provider;
mod schemas;
pub mod sentiment;
pub mod types;
pub mod voice;

pub use ops as rpc;
pub use schemas::{
    all_controller_schemas as all_inference_controller_schemas,
    all_registered_controllers as all_inference_registered_controllers,
};

// Re-export the types that external callers (voice, agent, etc.) import from inference
pub use device::DeviceProfile;
pub use local::all_local_ai_controller_schemas;
pub use local::all_local_ai_registered_controllers;
pub use presets::{ModelPreset, ModelTier, VisionMode};
pub use sentiment::SentimentResult;
pub use types::{
    LocalAiAssetStatus, LocalAiAssetsStatus, LocalAiDownloadProgressItem, LocalAiDownloadsProgress,
    LocalAiEmbeddingResult, LocalAiSpeechResult, LocalAiStatus, LocalAiTtsResult,
};

// Test helpers (re-exported for sibling test files that use inference_test_guard)
#[cfg(test)]
pub(crate) fn inference_test_guard() -> std::sync::MutexGuard<'static, ()> {
    local::inference_test_guard()
}
