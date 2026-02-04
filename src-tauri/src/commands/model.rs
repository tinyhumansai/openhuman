//! Model Tauri Commands
//!
//! These commands provide local LLM access via Tauri's invoke() system.
//! - Desktop (Windows, macOS, Linux): Uses llama.cpp via llama-cpp-2 crate
//! - Android: Uses MediaPipe LLM Inference API via JNI
//! - iOS: Not yet supported

use serde::{Deserialize, Serialize};

// serde_json used for Android MediaPipe commands
#[cfg(target_os = "android")]
use serde_json::{json, Value as JsonValue};

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

// ============================================================================
// Android JNI Bridge to MediaPipe LLM
// ============================================================================

#[cfg(target_os = "android")]
mod android {
    use super::*;

    /// Call a static method on MediaPipeLlmBridge that returns a String.
    /// This uses Android's JNI to communicate with the Kotlin MediaPipe wrapper.
    pub fn call_mediapipe_method(method: &str) -> Result<String, String> {
        // For now, return a placeholder - actual JNI implementation requires
        // access to the JNI environment which needs to be passed from Tauri's
        // Android activity context.
        //
        // TODO: Implement proper JNI bridge using tauri's android module
        // The MediaPipeLlmBridge Kotlin object is already set up and ready.
        log::warn!(
            "MediaPipe LLM method '{}' called - JNI bridge pending implementation",
            method
        );
        Err(format!(
            "MediaPipe LLM JNI bridge not yet implemented for method: {}",
            method
        ))
    }

    /// Call a static method on MediaPipeLlmBridge with a String argument.
    pub fn call_mediapipe_method_with_arg(method: &str, arg: &str) -> Result<String, String> {
        log::warn!(
            "MediaPipe LLM method '{}' called with arg - JNI bridge pending implementation",
            method
        );
        let _ = arg;
        Err(format!(
            "MediaPipe LLM JNI bridge not yet implemented for method: {}",
            method
        ))
    }

    /// Parse a JSON response from MediaPipe bridge.
    pub fn parse_mediapipe_response(json: &str) -> Result<JsonValue, String> {
        serde_json::from_str(json).map_err(|e| format!("Failed to parse MediaPipe response: {}", e))
    }
}

/// Check if the local model API is available on this platform.
/// - Desktop: Uses llama.cpp (always available)
/// - Android: Uses MediaPipe LLM (available on supported devices)
/// - iOS: Not yet supported
#[tauri::command]
pub fn model_is_available() -> bool {
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        true
    }

    #[cfg(target_os = "android")]
    {
        // MediaPipe LLM is available on Android
        true
    }

    #[cfg(target_os = "ios")]
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

    #[cfg(target_os = "android")]
    {
        // MediaPipe LLM status - JNI bridge pending full implementation
        // For now, report available but not loaded
        ModelStatusResponse {
            available: true,
            loaded: false,
            loading: false,
            download_progress: None,
            error: Some("MediaPipe LLM: Download a model to get started".to_string()),
            model_path: None,
        }
    }

    #[cfg(target_os = "ios")]
    {
        ModelStatusResponse {
            available: false,
            loaded: false,
            loading: false,
            download_progress: None,
            error: Some("Model not available on iOS".to_string()),
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

    #[cfg(target_os = "android")]
    {
        // MediaPipe requires manual model download
        // TODO: Implement model download via MediaPipeLlmBridge.loadModel()
        Err("MediaPipe LLM: Please download a model first using the model manager".to_string())
    }

    #[cfg(target_os = "ios")]
    {
        Err("Model not available on iOS".to_string())
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

    #[cfg(target_os = "android")]
    {
        // TODO: Call MediaPipeLlmBridge.generateResponse() via JNI
        // The Kotlin bridge is ready, just needs JNI wiring
        let _ = request;
        Err("MediaPipe LLM generation pending JNI implementation".to_string())
    }

    #[cfg(target_os = "ios")]
    {
        let _ = request;
        Err("Model not available on iOS".to_string())
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

    #[cfg(target_os = "android")]
    {
        // TODO: Implement summarization via MediaPipe
        let _ = (text, max_tokens);
        Err("MediaPipe LLM summarization pending JNI implementation".to_string())
    }

    #[cfg(target_os = "ios")]
    {
        let _ = (text, max_tokens);
        Err("Model not available on iOS".to_string())
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

    #[cfg(target_os = "android")]
    {
        // TODO: Call MediaPipeLlmBridge.unloadModel() via JNI
        Ok(())
    }

    #[cfg(target_os = "ios")]
    {
        Err("Model not available on iOS".to_string())
    }
}

// ============================================================================
// Android-specific MediaPipe LLM Commands
// ============================================================================

/// Get recommended models for download (Android only).
/// Returns a list of MediaPipe-compatible models with download URLs.
#[tauri::command]
pub fn model_get_recommended() -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        // Return hardcoded recommended models for now
        // TODO: Call MediaPipeLlmBridge.getRecommendedModels() via JNI
        let models = serde_json::json!({
            "success": true,
            "models": [
                {
                    "name": "Gemma 3 1B (4-bit)",
                    "id": "gemma-3-1b-it-int4",
                    "size_mb": 550,
                    "description": "Compact, fast model suitable for most devices",
                    "url": "https://huggingface.co/litert-community/Gemma3-1B-IT/resolve/main/gemma3-1b-it-int4.task"
                },
                {
                    "name": "Gemma 3n E2B (4-bit)",
                    "id": "gemma-3n-e2b-it-int4",
                    "size_mb": 1400,
                    "description": "Effective 2B model with multimodal support",
                    "url": "https://huggingface.co/litert-community/Gemma3n-E2B-IT/resolve/main/gemma3n-e2b-it-int4.task"
                },
                {
                    "name": "Gemma 3n E4B (4-bit)",
                    "id": "gemma-3n-e4b-it-int4",
                    "size_mb": 2800,
                    "description": "Effective 4B model, best quality, requires high-end device",
                    "url": "https://huggingface.co/litert-community/Gemma3n-E4B-IT/resolve/main/gemma3n-e4b-it-int4.task"
                }
            ]
        });
        Ok(models.to_string())
    }

    #[cfg(not(target_os = "android"))]
    {
        Err("This command is only available on Android".to_string())
    }
}

/// List downloaded models (Android only).
#[tauri::command]
pub fn model_list_downloaded() -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        // TODO: Call MediaPipeLlmBridge.listModels() via JNI
        let result = serde_json::json!({
            "success": true,
            "models": [],
            "models_dir": "/data/data/com.alphahuman.app/files/models"
        });
        Ok(result.to_string())
    }

    #[cfg(not(target_os = "android"))]
    {
        Err("This command is only available on Android".to_string())
    }
}

/// Load a specific model by path (Android only).
#[tauri::command]
pub fn model_load_path(
    model_path: String,
    max_tokens: Option<i32>,
    top_k: Option<i32>,
    temperature: Option<f32>,
) -> Result<String, String> {
    #[cfg(target_os = "android")]
    {
        // TODO: Call MediaPipeLlmBridge.loadModel() via JNI
        let _ = (model_path, max_tokens, top_k, temperature);
        Err("MediaPipe model loading pending JNI implementation".to_string())
    }

    #[cfg(not(target_os = "android"))]
    {
        let _ = (model_path, max_tokens, top_k, temperature);
        Err("This command is only available on Android".to_string())
    }
}
