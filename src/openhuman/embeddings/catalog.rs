//! Static catalog of supported embedding providers.
//!
//! Each entry declares its slug, display label, whether it requires an API key,
//! and the models + dimension presets it supports. The frontend reads this via
//! `openhuman.embeddings_get_settings` to populate the provider picker.

use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingModelPreset {
    pub id: &'static str,
    pub label: &'static str,
    pub default_dimensions: usize,
    pub allowed_dimensions: &'static [usize],
}

#[derive(Debug, Clone, Serialize)]
pub struct EmbeddingProviderEntry {
    pub slug: &'static str,
    pub label: &'static str,
    pub description: &'static str,
    pub requires_api_key: bool,
    pub requires_endpoint: bool,
    pub models: &'static [EmbeddingModelPreset],
}

pub const PROVIDER_MANAGED: &str = "managed";
pub const PROVIDER_VOYAGE: &str = "voyage";
pub const PROVIDER_OPENAI: &str = "openai";
pub const PROVIDER_COHERE: &str = "cohere";
pub const PROVIDER_OLLAMA: &str = "ollama";
pub const PROVIDER_CUSTOM: &str = "custom";
pub const PROVIDER_NONE: &str = "none";

static MANAGED_MODELS: &[EmbeddingModelPreset] = &[EmbeddingModelPreset {
    id: "embedding-v1",
    label: "Embedding v1 (Voyage-backed)",
    default_dimensions: 1024,
    allowed_dimensions: &[1024],
}];

static VOYAGE_MODELS: &[EmbeddingModelPreset] = &[
    EmbeddingModelPreset {
        id: "voyage-3-large",
        label: "Voyage 3 Large",
        default_dimensions: 1024,
        allowed_dimensions: &[256, 512, 1024, 2048],
    },
    EmbeddingModelPreset {
        id: "voyage-3",
        label: "Voyage 3",
        default_dimensions: 1024,
        allowed_dimensions: &[1024],
    },
    EmbeddingModelPreset {
        id: "voyage-code-3",
        label: "Voyage Code 3",
        default_dimensions: 1024,
        allowed_dimensions: &[1024],
    },
];

static OPENAI_MODELS: &[EmbeddingModelPreset] = &[
    EmbeddingModelPreset {
        id: "text-embedding-3-small",
        label: "Embedding 3 Small",
        default_dimensions: 1536,
        allowed_dimensions: &[512, 1536],
    },
    EmbeddingModelPreset {
        id: "text-embedding-3-large",
        label: "Embedding 3 Large",
        default_dimensions: 3072,
        allowed_dimensions: &[256, 1024, 3072],
    },
];

static COHERE_MODELS: &[EmbeddingModelPreset] = &[
    EmbeddingModelPreset {
        id: "embed-english-v3.0",
        label: "Embed English v3",
        default_dimensions: 1024,
        allowed_dimensions: &[1024],
    },
    EmbeddingModelPreset {
        id: "embed-multilingual-v3.0",
        label: "Embed Multilingual v3",
        default_dimensions: 1024,
        allowed_dimensions: &[1024],
    },
];

static OLLAMA_MODELS: &[EmbeddingModelPreset] = &[EmbeddingModelPreset {
    id: "bge-m3",
    label: "BGE-M3",
    default_dimensions: 1024,
    allowed_dimensions: &[1024],
}];

static CATALOG: &[EmbeddingProviderEntry] = &[
    EmbeddingProviderEntry {
        slug: PROVIDER_MANAGED,
        label: "Managed (OpenHuman)",
        description: "Routes through the OpenHuman backend. No API key needed.",
        requires_api_key: false,
        requires_endpoint: false,
        models: MANAGED_MODELS,
    },
    EmbeddingProviderEntry {
        slug: PROVIDER_VOYAGE,
        label: "Voyage AI",
        description: "Direct Voyage AI API with your own key.",
        requires_api_key: true,
        requires_endpoint: false,
        models: VOYAGE_MODELS,
    },
    EmbeddingProviderEntry {
        slug: PROVIDER_OPENAI,
        label: "OpenAI",
        description: "OpenAI embeddings API with your own key.",
        requires_api_key: true,
        requires_endpoint: false,
        models: OPENAI_MODELS,
    },
    EmbeddingProviderEntry {
        slug: PROVIDER_COHERE,
        label: "Cohere",
        description: "Cohere embed API with your own key.",
        requires_api_key: true,
        requires_endpoint: false,
        models: COHERE_MODELS,
    },
    EmbeddingProviderEntry {
        slug: PROVIDER_OLLAMA,
        label: "Ollama (Local)",
        description: "Local Ollama server. No API key needed.",
        requires_api_key: false,
        requires_endpoint: false,
        models: OLLAMA_MODELS,
    },
    EmbeddingProviderEntry {
        slug: PROVIDER_CUSTOM,
        label: "Custom (OpenAI-compatible)",
        description: "Any OpenAI-compatible embedding endpoint.",
        requires_api_key: true,
        requires_endpoint: true,
        models: &[],
    },
    EmbeddingProviderEntry {
        slug: PROVIDER_NONE,
        label: "Disabled",
        description: "Disable semantic search. Keyword search only.",
        requires_api_key: false,
        requires_endpoint: false,
        models: &[],
    },
];

pub fn all_providers() -> &'static [EmbeddingProviderEntry] {
    CATALOG
}

pub fn find_provider(slug: &str) -> Option<&'static EmbeddingProviderEntry> {
    CATALOG.iter().find(|e| e.slug == slug)
}

pub fn find_model(provider_slug: &str, model_id: &str) -> Option<&'static EmbeddingModelPreset> {
    find_provider(provider_slug).and_then(|p| p.models.iter().find(|m| m.id == model_id))
}

pub fn default_model_for(provider_slug: &str) -> Option<&'static EmbeddingModelPreset> {
    find_provider(provider_slug).and_then(|p| p.models.first())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_is_non_empty() {
        assert!(!all_providers().is_empty());
    }

    #[test]
    fn managed_is_first() {
        assert_eq!(all_providers()[0].slug, PROVIDER_MANAGED);
    }

    #[test]
    fn find_voyage_model() {
        let m = find_model(PROVIDER_VOYAGE, "voyage-3-large").unwrap();
        assert!(m.allowed_dimensions.contains(&1024));
    }

    #[test]
    fn default_model_for_openai() {
        let m = default_model_for(PROVIDER_OPENAI).unwrap();
        assert_eq!(m.id, "text-embedding-3-small");
    }

    #[test]
    fn none_has_no_models() {
        let p = find_provider(PROVIDER_NONE).unwrap();
        assert!(p.models.is_empty());
    }

    #[test]
    fn unknown_provider_returns_none() {
        assert!(find_provider("unknown").is_none());
    }

    #[test]
    fn all_providers_have_unique_slugs() {
        let providers = all_providers();
        let mut seen = std::collections::HashSet::new();
        for entry in providers {
            assert!(
                seen.insert(entry.slug),
                "duplicate slug in CATALOG: \"{}\"",
                entry.slug
            );
        }
    }

    #[test]
    fn all_models_have_valid_dimensions() {
        for entry in all_providers() {
            for model in entry.models {
                assert!(
                    model.allowed_dimensions.contains(&model.default_dimensions),
                    "provider \"{}\" model \"{}\" has default_dimensions {} not in allowed_dimensions {:?}",
                    entry.slug,
                    model.id,
                    model.default_dimensions,
                    model.allowed_dimensions
                );
            }
        }
    }

    #[test]
    fn default_model_for_all_providers_with_models() {
        for entry in all_providers() {
            if !entry.models.is_empty() {
                assert!(
                    default_model_for(entry.slug).is_some(),
                    "default_model_for({:?}) returned None but provider has {} models",
                    entry.slug,
                    entry.models.len()
                );
            }
        }
    }
}
