//! Local AI runtime configuration.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Per-feature flags controlling which subsystems route through the selected
/// local runtime. All default to `false` (use cloud instead). Guarded by
/// `LocalAiConfig::runtime_enabled` — when that is `false` every helper
/// method below returns `false` regardless of these values.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
#[derive(Default)]
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
    /// subconscious evaluation and execution.
    #[serde(default)]
    pub subconscious: bool,
}

#[derive(Clone, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct LocalAiConfig {
    /// Master runtime switch. Defaults to `false` — local AI is OFF by default.
    /// Note: the old on-disk field was `enabled`; that key is now unknown to
    /// serde and will be silently ignored on load (intentional forced reset).
    #[serde(default = "default_runtime_enabled")]
    pub runtime_enabled: bool,
    /// Local provider identifier. Supported values are `ollama`, `lm_studio`,
    /// and `omlx`; unknown values normalize to `ollama` at runtime.
    #[serde(default = "default_provider")]
    pub provider: String,
    /// Optional provider base URL. For LM Studio this defaults to
    /// `http://localhost:1234/v1`.
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
    /// Legacy voice STT routing string. `"cloud"` (the default) means "use
    /// `voice_server.stt_engine`"; a third-party `"<slug>[:<model>]"` overrides
    /// the engine outright. The local `"whisper"` value it once accepted is
    /// dead — `config::migrations` rewrites it back to `"cloud"`.
    #[serde(default = "default_stt_provider")]
    pub stt_provider: String,
    #[serde(default = "default_tts_voice_id")]
    pub tts_voice_id: String,
    /// Voice TTS provider selector. `"cloud"` (default) routes through the
    /// backend ElevenLabs proxy and returns rich visemes; `"piper"` runs
    /// local Piper via the `PIPER_BIN` env var.
    #[serde(default = "default_tts_provider")]
    pub tts_provider: String,
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
    /// When true and Ollama is available, pass raw transcription through a
    /// local LLM to fix grammar/punctuation using conversation context.
    #[serde(default = "default_voice_llm_cleanup_enabled")]
    pub voice_llm_cleanup_enabled: bool,
    /// Ollama `options.num_ctx` override. When set, every chat request to
    /// an Ollama provider includes `"options": {"num_ctx": <value>}` so
    /// the model allocates at least this much KV-cache. Ollama defaults
    /// to 2048 for many models which is too small for agentic use.
    #[serde(default)]
    pub num_ctx: Option<u32>,
    /// Per-feature flags. Each gate is AND-ed with `runtime_enabled`.
    /// All default to `false` (cloud path).
    #[serde(default)]
    pub usage: LocalAiUsage,
}

impl std::fmt::Debug for LocalAiConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LocalAiConfig")
            .field("runtime_enabled", &self.runtime_enabled)
            .field("provider", &self.provider)
            .field("base_url", &self.base_url)
            .field("api_key", &self.api_key.as_ref().map(|_| "<redacted>"))
            .field("model_id", &self.model_id)
            .field("chat_model_id", &self.chat_model_id)
            .field("vision_model_id", &self.vision_model_id)
            .field("embedding_model_id", &self.embedding_model_id)
            .field("stt_model_id", &self.stt_model_id)
            .field("stt_download_url", &self.stt_download_url)
            .field("stt_provider", &self.stt_provider)
            .field("tts_voice_id", &self.tts_voice_id)
            .field("tts_provider", &self.tts_provider)
            .field("tts_download_url", &self.tts_download_url)
            .field("tts_config_download_url", &self.tts_config_download_url)
            .field("quantization", &self.quantization)
            .field("preload_vision_model", &self.preload_vision_model)
            .field("preload_embedding_model", &self.preload_embedding_model)
            .field("preload_stt_model", &self.preload_stt_model)
            .field("preload_tts_voice", &self.preload_tts_voice)
            .field("download_url", &self.download_url)
            .field("autosummary_debounce_ms", &self.autosummary_debounce_ms)
            .field("selected_tier", &self.selected_tier)
            .field("opt_in_confirmed", &self.opt_in_confirmed)
            .field("ollama_binary_path", &self.ollama_binary_path)
            .field("voice_llm_cleanup_enabled", &self.voice_llm_cleanup_enabled)
            .field("num_ctx", &self.num_ctx)
            .field("usage", &self.usage)
            .finish()
    }
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
    // bge-m3 (1024 dims, 8192-token context): the default local embedding
    // model for tool discovery and the `embeddings` RPC.
    "bge-m3".to_string()
}

fn default_stt_model_id() -> String {
    "ggml-base-q5_1.bin".to_string()
}

fn default_tts_voice_id() -> String {
    "en_US-lessac-medium".to_string()
}

fn default_stt_provider() -> String {
    "cloud".to_string()
}

fn default_tts_provider() -> String {
    "cloud".to_string()
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

fn default_voice_llm_cleanup_enabled() -> bool {
    true
}

impl LocalAiConfig {
    /// Returns `true` when the local Ollama runtime is active.
    /// This is the primary gate; all per-feature helpers below AND with this.
    pub fn is_active(&self) -> bool {
        self.runtime_enabled
    }

    /// **Deprecated** — read from `Config::workload_uses_local("embeddings")`
    /// instead. This helper only consults the legacy `usage.*` booleans, which
    /// are no longer the source of truth after the unified AI settings
    /// migration (schema_version >= 2).
    #[deprecated(note = "Use Config::workload_uses_local(\"embeddings\")")]
    pub fn use_local_for_embeddings(&self) -> bool {
        self.runtime_enabled && self.usage.embeddings
    }

    /// **Deprecated** — read from `Config::workload_uses_local("heartbeat")`.
    #[deprecated(note = "Use Config::workload_uses_local(\"heartbeat\")")]
    pub fn use_local_for_heartbeat(&self) -> bool {
        self.runtime_enabled && self.usage.heartbeat
    }

    /// **Deprecated** — read from `Config::workload_uses_local("subconscious")`.
    #[deprecated(note = "Use Config::workload_uses_local(\"subconscious\")")]
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
            stt_provider: default_stt_provider(),
            tts_voice_id: default_tts_voice_id(),
            tts_provider: default_tts_provider(),
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
            voice_llm_cleanup_enabled: default_voice_llm_cleanup_enabled(),
            num_ctx: None,
            usage: LocalAiUsage::default(),
        }
    }
}

#[cfg(test)]
#[path = "local_ai_tests.rs"]
mod tests;
