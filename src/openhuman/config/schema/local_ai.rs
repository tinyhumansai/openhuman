//! Local AI runtime configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-feature flags controlling which subsystems route through the local
/// Ollama runtime. All default to `false` (use cloud instead). Guarded by
/// `LocalAiConfig::runtime_enabled` — when that is `false` every helper
/// method below returns `false` regardless of these values.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LocalAiUsage {
    /// When true (and `runtime_enabled`), use the local model for embedding
    /// generation instead of the cloud backend.
    #[serde(default)]
    pub embeddings: bool,
    /// When true (and `runtime_enabled`), use the local model inside the
    /// heartbeat loop.
    #[serde(default)]
    pub heartbeat: bool,
    /// When true (and `runtime_enabled`), use the local model for
    /// learning/reflection passes.
    #[serde(default)]
    pub learning_reflection: bool,
    /// When true (and `runtime_enabled`), use the local model for
    /// subconscious evaluation and execution.
    #[serde(default)]
    pub subconscious: bool,
}

impl Default for LocalAiUsage {
    fn default() -> Self {
        Self {
            embeddings: false,
            heartbeat: false,
            learning_reflection: false,
            subconscious: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LocalAiConfig {
    /// Master runtime switch. Defaults to `false` — Ollama is OFF by default.
    /// Note: the old on-disk field was `enabled`; that key is now unknown to
    /// serde and will be silently ignored on load (intentional forced reset).
    #[serde(default = "default_runtime_enabled")]
    pub runtime_enabled: bool,
    #[serde(default = "default_provider")]
    pub provider: String,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default)]
    pub api_key: Option<String>,
    #[serde(default = "default_model_id")]
    pub model_id: String,
    #[serde(default = "default_chat_model_id")]
    pub chat_model_id: String,
    #[serde(default = "default_vision_model_id")]
    pub vision_model_id: String,
    #[serde(default = "default_embedding_model_id")]
    pub embedding_model_id: String,
    #[serde(default = "default_stt_model_id")]
    pub stt_model_id: String,
    #[serde(default = "default_stt_download_url")]
    pub stt_download_url: Option<String>,
    #[serde(default = "default_tts_voice_id")]
    pub tts_voice_id: String,
    #[serde(default = "default_tts_download_url")]
    pub tts_download_url: Option<String>,
    #[serde(default = "default_tts_config_download_url")]
    pub tts_config_download_url: Option<String>,
    #[serde(default = "default_quantization")]
    pub quantization: String,
    #[serde(default = "default_preload_vision_model")]
    pub preload_vision_model: bool,
    #[serde(default = "default_preload_embedding_model")]
    pub preload_embedding_model: bool,
    #[serde(default = "default_preload_stt_model")]
    pub preload_stt_model: bool,
    #[serde(default = "default_preload_tts_voice")]
    pub preload_tts_voice: bool,
    #[serde(default = "default_download_url")]
    pub download_url: Option<String>,
    #[serde(default = "default_autosummary_debounce_ms")]
    pub autosummary_debounce_ms: u64,
    #[serde(default)]
    pub selected_tier: Option<String>,
    /// Explicit MVP opt-in marker. Bootstrap disables local AI unless this is
    /// `true`, regardless of any prior `selected_tier` value. Existing installs
    /// (upgrading from pre-MVP) default to `false` and must re-opt-in from
    /// Settings. Set by `apply_preset` on any non-disabled tier.
    #[serde(default)]
    pub opt_in_confirmed: bool,
    /// Optional path to a manually-installed Ollama binary.
    #[serde(default)]
    pub ollama_binary_path: Option<String>,
    /// When true, load the whisper model in-process via whisper-rs instead of
    /// shelling out to whisper-cli for each transcription call.
    #[serde(default = "default_whisper_in_process")]
    pub whisper_in_process: bool,
    /// When true and Ollama is available, pass raw transcription through a
    /// local LLM to fix grammar/punctuation using conversation context.
    #[serde(default = "default_voice_llm_cleanup_enabled")]
    pub voice_llm_cleanup_enabled: bool,
    /// Per-feature flags. Each gate is AND-ed with `runtime_enabled`.
    /// All default to `false` (cloud path).
    #[serde(default)]
    pub usage: LocalAiUsage,
}

fn default_runtime_enabled() -> bool {
    false
}

fn default_provider() -> String {
    "ollama".to_string()
}

fn default_model_id() -> String {
    "gemma3:1b-it-qat".to_string()
}

fn default_chat_model_id() -> String {
    "gemma3:1b-it-qat".to_string()
}

fn default_vision_model_id() -> String {
    String::new()
}

fn default_embedding_model_id() -> String {
    "all-minilm:latest".to_string()
}

fn default_stt_model_id() -> String {
    "ggml-base-q5_1.bin".to_string()
}

fn default_tts_voice_id() -> String {
    "en_US-lessac-medium".to_string()
}

fn default_stt_download_url() -> Option<String> {
    Some(
        "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base-q5_1.bin?download=true"
            .to_string(),
    )
}

fn default_tts_download_url() -> Option<String> {
    Some(
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx?download=true"
            .to_string(),
    )
}

fn default_tts_config_download_url() -> Option<String> {
    Some(
        "https://huggingface.co/rhasspy/piper-voices/resolve/main/en/en_US/lessac/medium/en_US-lessac-medium.onnx.json?download=true"
            .to_string(),
    )
}

fn default_quantization() -> String {
    "q4".to_string()
}

fn default_preload_vision_model() -> bool {
    false
}

fn default_preload_embedding_model() -> bool {
    true
}

fn default_preload_stt_model() -> bool {
    false
}

fn default_preload_tts_voice() -> bool {
    false
}

fn default_download_url() -> Option<String> {
    None
}

fn default_autosummary_debounce_ms() -> u64 {
    2500
}

fn default_whisper_in_process() -> bool {
    true
}

fn default_voice_llm_cleanup_enabled() -> bool {
    true
}

impl LocalAiConfig {
    /// Returns `true` when the local Ollama runtime is active.
    /// This is the primary gate; all per-feature helpers below AND with this.
    pub fn is_active(&self) -> bool {
        self.runtime_enabled
    }

    /// Use the local model for embedding generation.
    pub fn use_local_for_embeddings(&self) -> bool {
        self.runtime_enabled && self.usage.embeddings
    }

    /// Use the local model inside the heartbeat loop.
    pub fn use_local_for_heartbeat(&self) -> bool {
        self.runtime_enabled && self.usage.heartbeat
    }

    /// Use the local model for learning/reflection passes.
    pub fn use_local_for_learning(&self) -> bool {
        self.runtime_enabled && self.usage.learning_reflection
    }

    /// Use the local model for subconscious evaluation and execution.
    pub fn use_local_for_subconscious(&self) -> bool {
        self.runtime_enabled && self.usage.subconscious
    }
}

impl Default for LocalAiConfig {
    fn default() -> Self {
        Self {
            runtime_enabled: default_runtime_enabled(),
            provider: default_provider(),
            base_url: None,
            api_key: None,
            model_id: default_model_id(),
            chat_model_id: default_chat_model_id(),
            vision_model_id: default_vision_model_id(),
            embedding_model_id: default_embedding_model_id(),
            stt_model_id: default_stt_model_id(),
            stt_download_url: default_stt_download_url(),
            tts_voice_id: default_tts_voice_id(),
            tts_download_url: default_tts_download_url(),
            tts_config_download_url: default_tts_config_download_url(),
            quantization: default_quantization(),
            preload_vision_model: default_preload_vision_model(),
            preload_embedding_model: default_preload_embedding_model(),
            preload_stt_model: default_preload_stt_model(),
            preload_tts_voice: default_preload_tts_voice(),
            download_url: default_download_url(),
            autosummary_debounce_ms: default_autosummary_debounce_ms(),
            selected_tier: None,
            opt_in_confirmed: false,
            ollama_binary_path: None,
            whisper_in_process: default_whisper_in_process(),
            voice_llm_cleanup_enabled: default_voice_llm_cleanup_enabled(),
            usage: LocalAiUsage::default(),
        }
    }
}
