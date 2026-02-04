//! Model Tauri Commands
//!
//! These commands provide local LLM access via Tauri's invoke() system.
//! Available on desktop and Android (not iOS).

use serde::{Deserialize, Serialize};

/// Model status response for frontend.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelStatusResponse {
    /// Whether the model API is available on this platform.
    pub available: bool,
    /// Whether the model is currently loaded in memory.
    pub loaded: bool,
    /// Whether the model is currently being loaded or downloaded.
    pub loading: bool,
    /// Download progress (0.0 to 1.0) if downloading.
    pub download_progress: Option<f32>,
    /// Error message if loading failed.
    pub error: Option<String>,
    /// Model file path if known.
    pub model_path: Option<String>,
}

/// Generation configuration from frontend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct GenerateRequest {
    /// Input prompt.
    pub prompt: String,
    /// Maximum tokens to generate (default: 2048).
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
    /// Sampling temperature (default: 0.7).
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Top-p sampling (default: 0.9).
    #[serde(default = "default_top_p")]
    pub top_p: f32,
}

fn default_max_tokens() -> u32 {
    2048
}
fn default_temperature() -> f32 {
    0.7
}
fn default_top_p() -> f32 {
    0.9
}

/// Check if the local model API is available on this platform.
/// Note: Currently only available on desktop (Windows, macOS, Linux).
/// Android/iOS support requires additional llama.cpp NDK configuration.
#[tauri::command]
pub fn model_is_available() -> bool {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        true
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        false
    }
}

/// Get the current model status.
#[tauri::command]
pub fn model_get_status() -> ModelStatusResponse {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let status = crate::services::llama::LLAMA_MANAGER.get_status();
        ModelStatusResponse {
            available: status.available,
            loaded: status.loaded,
            loading: status.loading,
            download_progress: status.download_progress,
            error: status.error,
            model_path: status.model_path,
        }
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        ModelStatusResponse {
            available: false,
            loaded: false,
            loading: false,
            download_progress: None,
            error: Some("Model not available on mobile platforms".to_string()),
            model_path: None,
        }
    }
}

/// Ensure the model is loaded (downloads if necessary).
/// This is useful for preloading the model.
#[tauri::command]
pub async fn model_ensure_loaded() -> Result<(), String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        crate::services::llama::LLAMA_MANAGER.ensure_loaded().await
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        Err("Model not available on mobile platforms".to_string())
    }
}

/// Generate text from a prompt.
#[tauri::command]
pub async fn model_generate(request: GenerateRequest) -> Result<String, String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        use crate::services::llama::GenerateConfig;

        let config = GenerateConfig {
            max_tokens: request.max_tokens,
            temperature: request.temperature,
            top_p: request.top_p,
        };

        crate::services::llama::LLAMA_MANAGER
            .generate(&request.prompt, config)
            .await
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = request;
        Err("Model not available on mobile platforms".to_string())
    }
}

/// Summarize text using a built-in prompt.
#[tauri::command]
pub async fn model_summarize(text: String, max_tokens: Option<u32>) -> Result<String, String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        let tokens = max_tokens.unwrap_or(500);
        crate::services::llama::LLAMA_MANAGER
            .summarize(&text, tokens)
            .await
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        let _ = (text, max_tokens);
        Err("Model not available on mobile platforms".to_string())
    }
}

/// Unload the model from memory to free resources.
#[tauri::command]
pub fn model_unload() -> Result<(), String> {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        crate::services::llama::LLAMA_MANAGER.unload();
        Ok(())
    }

    #[cfg(any(target_os = "android", target_os = "ios"))]
    {
        Err("Model not available on mobile platforms".to_string())
    }
}
