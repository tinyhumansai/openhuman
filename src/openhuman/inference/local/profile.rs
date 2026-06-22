//! Local provider profiles — capability metadata for local inference runtimes.
//!
//! Instead of treating all local OpenAI-compatible providers identically, each
//! provider type (Ollama, LM Studio, MLX-compatible, generic local OpenAI) gets
//! a profile that declares its capabilities, quirks, and default context window.
//! The factory and agent harness consult these profiles for:
//!
//! - Tool dispatch strategy (native vs prompt-guided)
//! - Context window defaults for unknown model names
//! - Request body extras (`options.num_ctx`, `think` field suppression)
//! - Temperature handling

use serde::{Deserialize, Serialize};

/// Identifies a local provider type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LocalProviderKind {
    Ollama,
    LmStudio,
    /// MLX-compatible local server (e.g. `mlx_lm.server`).
    Mlx,
    /// OMLX — OpenAI v1-compatible MLX server that requires an API key.
    Omlx,
    /// Generic local OpenAI-compatible endpoint (llama.cpp, vLLM, etc.).
    LocalOpenai,
}

impl LocalProviderKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lmstudio",
            Self::Mlx => "mlx",
            Self::Omlx => "omlx",
            Self::LocalOpenai => "local-openai",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
            Self::Mlx => "MLX",
            Self::Omlx => "OMLX",
            Self::LocalOpenai => "Local OpenAI",
        }
    }

    /// Parse a provider kind from a string, accepting common aliases.
    pub fn from_str_loose(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "ollama" => Some(Self::Ollama),
            "lmstudio" | "lm-studio" | "lm_studio" => Some(Self::LmStudio),
            "mlx" | "mlx-server" | "mlx_lm" => Some(Self::Mlx),
            "omlx" | "omlx-server" => Some(Self::Omlx),
            "local-openai" | "local_openai" | "llamacpp" | "llama.cpp" | "vllm" => {
                Some(Self::LocalOpenai)
            }
            _ => None,
        }
    }
}

/// How the provider handles tool calling.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolSupport {
    /// Provider reliably supports native OpenAI-style tool calling.
    Native,
    /// Provider does NOT support native tools — use prompt-guided dispatch.
    PromptGuided,
    /// Support depends on the specific model; probe or consult model profile.
    ModelDependent,
}

/// Extra request body options for a local provider.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct RequestQuirks {
    /// Ollama `options.num_ctx` override. When set, injected into the
    /// request body as `{"options": {"num_ctx": <value>}}`.
    pub num_ctx: Option<u32>,
    /// When true, suppress reasoning/thinking fields in the request.
    /// Some Ollama models reject requests containing `think` parameters.
    pub suppress_thinking: bool,
    /// When true, omit the `temperature` field entirely (model uses its
    /// own default). Distinct from `temperature_unsupported_models` which
    /// is pattern-based — this is a blanket provider-level override.
    pub omit_temperature: bool,
    /// When true, merge system messages into user messages (provider
    /// rejects `role: system`).
    pub merge_system_into_user: bool,
}

/// Static capability profile for a local provider type.
#[derive(Debug, Clone)]
pub struct LocalProviderProfile {
    pub kind: LocalProviderKind,
    /// Default tool support level for this provider type.
    pub tool_support: ToolSupport,
    /// Default context window (tokens) when the model name is not
    /// recognized by `context_window_for_model`. `None` means "unknown,
    /// skip preflight trimming".
    pub default_context_window: Option<u64>,
    /// Whether the provider supports the Responses API (`/v1/responses`).
    pub supports_responses_api: bool,
    /// Whether the provider supports SSE streaming.
    pub supports_streaming: bool,
    /// Default request quirks for this provider type.
    pub default_quirks: RequestQuirks,
    /// Default base URL when none is configured.
    pub default_base_url: &'static str,
    /// Environment variable name for base URL override.
    pub base_url_env: &'static str,
}

/// Ollama profile: conservative defaults, no native tools.
pub const OLLAMA_PROFILE: LocalProviderProfile = LocalProviderProfile {
    kind: LocalProviderKind::Ollama,
    tool_support: ToolSupport::PromptGuided,
    default_context_window: Some(8_192),
    supports_responses_api: false,
    supports_streaming: true,
    default_quirks: RequestQuirks {
        num_ctx: None,
        suppress_thinking: false,
        omit_temperature: false,
        merge_system_into_user: false,
    },
    default_base_url: "http://127.0.0.1:11434",
    base_url_env: "OLLAMA_HOST",
};

/// LM Studio profile: conservative defaults, no native tools.
pub const LM_STUDIO_PROFILE: LocalProviderProfile = LocalProviderProfile {
    kind: LocalProviderKind::LmStudio,
    tool_support: ToolSupport::PromptGuided,
    default_context_window: Some(8_192),
    supports_responses_api: false,
    supports_streaming: true,
    default_quirks: RequestQuirks {
        num_ctx: None,
        suppress_thinking: false,
        omit_temperature: false,
        merge_system_into_user: false,
    },
    default_base_url: "http://127.0.0.1:1234/v1",
    base_url_env: "LM_STUDIO_URL",
};

/// MLX-compatible server profile (mlx_lm.server, etc.).
pub const MLX_PROFILE: LocalProviderProfile = LocalProviderProfile {
    kind: LocalProviderKind::Mlx,
    tool_support: ToolSupport::PromptGuided,
    default_context_window: Some(4_096),
    supports_responses_api: false,
    supports_streaming: true,
    default_quirks: RequestQuirks {
        num_ctx: None,
        suppress_thinking: false,
        omit_temperature: false,
        merge_system_into_user: false,
    },
    default_base_url: "http://127.0.0.1:8080/v1",
    base_url_env: "MLX_SERVER_URL",
};

/// OMLX profile: OpenAI v1-compatible MLX server, default port 8000, key required.
pub const OMLX_PROFILE: LocalProviderProfile = LocalProviderProfile {
    kind: LocalProviderKind::Omlx,
    tool_support: ToolSupport::PromptGuided,
    default_context_window: Some(4_096),
    supports_responses_api: false,
    supports_streaming: true,
    default_quirks: RequestQuirks {
        num_ctx: None,
        suppress_thinking: false,
        omit_temperature: false,
        merge_system_into_user: false,
    },
    default_base_url: "http://127.0.0.1:8000/v1",
    base_url_env: "OMLX_SERVER_URL",
};

/// Generic local OpenAI-compatible server (llama.cpp, vLLM, etc.).
pub const LOCAL_OPENAI_PROFILE: LocalProviderProfile = LocalProviderProfile {
    kind: LocalProviderKind::LocalOpenai,
    tool_support: ToolSupport::PromptGuided,
    default_context_window: None,
    supports_responses_api: false,
    supports_streaming: true,
    default_quirks: RequestQuirks {
        num_ctx: None,
        suppress_thinking: false,
        omit_temperature: false,
        merge_system_into_user: false,
    },
    default_base_url: "http://127.0.0.1:8080/v1",
    base_url_env: "LOCAL_OPENAI_URL",
};

/// Look up the static profile for a provider kind.
pub fn profile_for_kind(kind: LocalProviderKind) -> &'static LocalProviderProfile {
    match kind {
        LocalProviderKind::Ollama => &OLLAMA_PROFILE,
        LocalProviderKind::LmStudio => &LM_STUDIO_PROFILE,
        LocalProviderKind::Mlx => &MLX_PROFILE,
        LocalProviderKind::Omlx => &OMLX_PROFILE,
        LocalProviderKind::LocalOpenai => &LOCAL_OPENAI_PROFILE,
    }
}

/// Resolve the provider kind from a provider string prefix.
///
/// Returns `None` for cloud/openhuman/unknown providers.
pub fn kind_from_provider_string(provider: &str) -> Option<LocalProviderKind> {
    let p = provider.trim().to_ascii_lowercase();
    if p.starts_with("ollama:") || p == "ollama" {
        Some(LocalProviderKind::Ollama)
    } else if p.starts_with("lmstudio:")
        || p.starts_with("lm-studio:")
        || p.starts_with("lm_studio:")
    {
        Some(LocalProviderKind::LmStudio)
    } else if p.starts_with("mlx:") {
        Some(LocalProviderKind::Mlx)
    } else if p.starts_with("omlx:") {
        Some(LocalProviderKind::Omlx)
    } else if p.starts_with("local-openai:") || p.starts_with("local_openai:") {
        Some(LocalProviderKind::LocalOpenai)
    } else {
        None
    }
}

/// Returns `true` when the provider string resolves to any local provider.
pub fn is_local_provider_string(provider: &str) -> bool {
    kind_from_provider_string(provider).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kind_from_str_loose_accepts_aliases() {
        assert_eq!(
            LocalProviderKind::from_str_loose("ollama"),
            Some(LocalProviderKind::Ollama)
        );
        assert_eq!(
            LocalProviderKind::from_str_loose("LM-Studio"),
            Some(LocalProviderKind::LmStudio)
        );
        assert_eq!(
            LocalProviderKind::from_str_loose("lm_studio"),
            Some(LocalProviderKind::LmStudio)
        );
        assert_eq!(
            LocalProviderKind::from_str_loose("mlx"),
            Some(LocalProviderKind::Mlx)
        );
        assert_eq!(
            LocalProviderKind::from_str_loose("mlx-server"),
            Some(LocalProviderKind::Mlx)
        );
        assert_eq!(
            LocalProviderKind::from_str_loose("llamacpp"),
            Some(LocalProviderKind::LocalOpenai)
        );
        assert_eq!(
            LocalProviderKind::from_str_loose("vllm"),
            Some(LocalProviderKind::LocalOpenai)
        );
        assert_eq!(LocalProviderKind::from_str_loose("unknown"), None);
    }

    #[test]
    fn kind_from_provider_string_parses_prefixes() {
        assert_eq!(
            kind_from_provider_string("ollama:qwen3:14b"),
            Some(LocalProviderKind::Ollama)
        );
        assert_eq!(
            kind_from_provider_string("lmstudio:mistral"),
            Some(LocalProviderKind::LmStudio)
        );
        assert_eq!(
            kind_from_provider_string("mlx:llama-3.1-8b"),
            Some(LocalProviderKind::Mlx)
        );
        assert_eq!(
            kind_from_provider_string("local-openai:qwen2"),
            Some(LocalProviderKind::LocalOpenai)
        );
        assert_eq!(kind_from_provider_string("openai:gpt-4o"), None);
        assert_eq!(kind_from_provider_string("openhuman"), None);
    }

    #[test]
    fn is_local_identifies_local_strings() {
        assert!(is_local_provider_string("ollama:phi3"));
        assert!(is_local_provider_string("mlx:model"));
        assert!(!is_local_provider_string("openai:gpt-4"));
        assert!(!is_local_provider_string("openhuman"));
    }

    #[test]
    fn profiles_have_correct_defaults() {
        let ollama = profile_for_kind(LocalProviderKind::Ollama);
        assert_eq!(ollama.tool_support, ToolSupport::PromptGuided);
        assert_eq!(ollama.default_context_window, Some(8_192));
        assert!(!ollama.supports_responses_api);

        let mlx = profile_for_kind(LocalProviderKind::Mlx);
        assert_eq!(mlx.default_context_window, Some(4_096));
    }

    #[test]
    fn ollama_profile_is_conservative_on_tools() {
        let profile = profile_for_kind(LocalProviderKind::Ollama);
        assert_eq!(profile.tool_support, ToolSupport::PromptGuided);
    }

    #[test]
    fn omlx_kind_and_profile() {
        assert_eq!(
            LocalProviderKind::from_str_loose("omlx"),
            Some(LocalProviderKind::Omlx)
        );
        assert_eq!(
            LocalProviderKind::from_str_loose("omlx-server"),
            Some(LocalProviderKind::Omlx)
        );
        assert_eq!(LocalProviderKind::Omlx.as_str(), "omlx");
        assert_eq!(LocalProviderKind::Omlx.display_name(), "OMLX");
        assert_eq!(
            profile_for_kind(LocalProviderKind::Omlx).default_base_url,
            "http://127.0.0.1:8000/v1"
        );
        assert_eq!(
            kind_from_provider_string("omlx:my-model"),
            Some(LocalProviderKind::Omlx)
        );
        assert!(is_local_provider_string("omlx:my-model"));
    }
}
