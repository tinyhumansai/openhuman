//! Local LLM Service using llama-cpp-2
//!
//! Provides local AI model inference for skills using llama.cpp.
//! Desktop only - not available on Android/iOS.

mod manager;

pub use manager::GenerateConfig;
pub use manager::LlamaManager;
pub use manager::ModelStatus;
pub use manager::LLAMA_MANAGER;
