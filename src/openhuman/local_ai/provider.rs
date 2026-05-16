//! Local AI provider selection helpers.

use crate::openhuman::config::Config;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalAiProvider {
    Ollama,
    LmStudio,
}

impl LocalAiProvider {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            Self::Ollama => "ollama",
            Self::LmStudio => "lm_studio",
        }
    }

    pub(crate) fn display_name(self) -> &'static str {
        match self {
            Self::Ollama => "Ollama",
            Self::LmStudio => "LM Studio",
        }
    }
}

pub(crate) fn normalize_provider(value: &str) -> String {
    match value.trim().to_ascii_lowercase().as_str() {
        "lmstudio" | "lm-studio" | "lm_studio" => LocalAiProvider::LmStudio.as_str().to_string(),
        _ => LocalAiProvider::Ollama.as_str().to_string(),
    }
}

pub(crate) fn provider_from_config(config: &Config) -> LocalAiProvider {
    match normalize_provider(&config.local_ai.provider).as_str() {
        "lm_studio" => LocalAiProvider::LmStudio,
        _ => LocalAiProvider::Ollama,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_provider_accepts_lm_studio_aliases() {
        assert_eq!(normalize_provider("lmstudio"), "lm_studio");
        assert_eq!(normalize_provider("lm-studio"), "lm_studio");
        assert_eq!(normalize_provider("LM_Studio"), "lm_studio");
    }

    #[test]
    fn normalize_provider_falls_back_to_ollama() {
        assert_eq!(normalize_provider(""), "ollama");
        assert_eq!(normalize_provider("unknown"), "ollama");
    }
}
