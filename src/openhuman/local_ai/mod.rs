//! Bundled local AI stack (Ollama, whisper.cpp, Piper).

mod core;
pub mod device;
pub mod gif_decision;
pub mod ops;
pub mod presets;
mod schemas;
pub mod sentiment;

mod install;
pub(crate) mod model_ids;
mod ollama_api;
mod parse;
pub(crate) mod paths;
mod service;
mod types;

pub use core::*;
pub use device::DeviceProfile;
pub use gif_decision::{GifDecision, TenorGifResult, TenorSearchResult};
pub use ops as rpc;
pub use ops::*;
pub use presets::{ModelPreset, ModelTier};
pub use sentiment::SentimentResult;
pub use schemas::{
    all_controller_schemas as all_local_ai_controller_schemas,
    all_registered_controllers as all_local_ai_registered_controllers,
};
pub(crate) use service::whisper_engine;
pub use service::LocalAiService;
pub use types::{
    LocalAiAssetStatus, LocalAiAssetsStatus, LocalAiDownloadProgressItem, LocalAiDownloadsProgress,
    LocalAiEmbeddingResult, LocalAiSpeechResult, LocalAiStatus, LocalAiTtsResult, Suggestion,
};
