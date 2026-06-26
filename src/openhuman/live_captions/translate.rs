//! Real-time translation for live captions.
//!
//! Uses the project's existing LLM inference pipeline to translate text between
//! languages. This approach leverages whatever model is configured (GPT-4,
//! Claude, local LLM) and supports all language pairs without shipping separate
//! translation model weights.
//!
//! For offline/edge deployments, swap to a dedicated translation model via the
//! inference provider config.

use tracing::debug;

const LOG_PREFIX: &str = "[live-captions-translate]";

/// Supported translation directions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TranslationPair {
    EnEs,
    EnFr,
    EnDe,
    EnZh,
    EnJa,
    EnHi,
    EsEn,
    FrEn,
    DeEn,
    ZhEn,
    JaEn,
    HiEn,
}

impl TranslationPair {
    pub fn source_lang(&self) -> &'static str {
        match self {
            Self::EnEs | Self::EnFr | Self::EnDe | Self::EnZh | Self::EnJa | Self::EnHi => {
                "English"
            }
            Self::EsEn => "Spanish",
            Self::FrEn => "French",
            Self::DeEn => "German",
            Self::ZhEn => "Chinese",
            Self::JaEn => "Japanese",
            Self::HiEn => "Hindi",
        }
    }

    pub fn target_lang(&self) -> &'static str {
        match self {
            Self::EnEs => "Spanish",
            Self::EnFr => "French",
            Self::EnDe => "German",
            Self::EnZh => "Chinese",
            Self::EnJa => "Japanese",
            Self::EnHi => "Hindi",
            Self::EsEn | Self::FrEn | Self::DeEn | Self::ZhEn | Self::JaEn | Self::HiEn => {
                "English"
            }
        }
    }

    /// Parse from source/target language codes (ISO 639-1).
    pub fn from_codes(src: &str, tgt: &str) -> Option<Self> {
        let src = src.trim().to_ascii_lowercase();
        let tgt = tgt.trim().to_ascii_lowercase();
        match (src.as_str(), tgt.as_str()) {
            ("en", "es") => Some(Self::EnEs),
            ("en", "fr") => Some(Self::EnFr),
            ("en", "de") => Some(Self::EnDe),
            ("en", "zh") => Some(Self::EnZh),
            ("en", "ja") => Some(Self::EnJa),
            ("en", "hi") => Some(Self::EnHi),
            ("es", "en") => Some(Self::EsEn),
            ("fr", "en") => Some(Self::FrEn),
            ("de", "en") => Some(Self::DeEn),
            ("zh", "en") => Some(Self::ZhEn),
            ("ja", "en") => Some(Self::JaEn),
            ("hi", "en") => Some(Self::HiEn),
            _ => None,
        }
    }
}

/// Translation result.
#[derive(Debug, Clone)]
pub struct TranslationResult {
    pub source_text: String,
    pub translated_text: String,
    pub source_lang: String,
    pub target_lang: String,
}

/// Build the translation prompt for the LLM.
pub fn build_translation_prompt(text: &str, pair: TranslationPair) -> String {
    format!(
        "Translate the following text from {} to {}. Output ONLY the translation, nothing else.\n\n{}",
        pair.source_lang(),
        pair.target_lang(),
        text
    )
}

/// Translate using the project's LLM inference (async).
/// Caller provides the LLM response text (from `create_chat_provider`).
pub fn parse_translation_response(
    source_text: &str,
    llm_response: &str,
    pair: TranslationPair,
) -> TranslationResult {
    let translated = llm_response.trim().to_string();
    debug!(
        "{LOG_PREFIX} translated {} chars ({} → {})",
        source_text.len(),
        pair.source_lang(),
        pair.target_lang()
    );
    TranslationResult {
        source_text: source_text.to_string(),
        translated_text: translated,
        source_lang: pair.source_lang().to_string(),
        target_lang: pair.target_lang().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_pair_from_codes() {
        assert_eq!(
            TranslationPair::from_codes("en", "es"),
            Some(TranslationPair::EnEs)
        );
        assert_eq!(
            TranslationPair::from_codes("zh", "en"),
            Some(TranslationPair::ZhEn)
        );
        assert_eq!(TranslationPair::from_codes("xx", "yy"), None);
    }

    #[test]
    fn from_codes_normalizes_case_and_whitespace() {
        // Mixed case and surrounding whitespace must still resolve to the pair.
        assert_eq!(
            TranslationPair::from_codes("EN", "es"),
            Some(TranslationPair::EnEs)
        );
        assert_eq!(
            TranslationPair::from_codes(" en ", "ES"),
            Some(TranslationPair::EnEs)
        );
        assert_eq!(
            TranslationPair::from_codes("Zh", " EN "),
            Some(TranslationPair::ZhEn)
        );
    }

    #[test]
    fn build_prompt_contains_languages() {
        let prompt = build_translation_prompt("Hello world", TranslationPair::EnEs);
        assert!(prompt.contains("English"));
        assert!(prompt.contains("Spanish"));
        assert!(prompt.contains("Hello world"));
    }

    #[test]
    fn parse_response_trims_whitespace() {
        let result = parse_translation_response("Hello", "  Hola  \n", TranslationPair::EnEs);
        assert_eq!(result.translated_text, "Hola");
        assert_eq!(result.source_lang, "English");
        assert_eq!(result.target_lang, "Spanish");
    }
}
