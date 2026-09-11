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

/// Returns `Some(reason)` when `model` is unmistakably **not** an embeddings
/// model and must be rejected before it is persisted as the embeddings model.
///
/// Conservative by design (zero false positives on real embedding ids): the
/// only hard signal is the OpenRouter `:free` chat/reasoning tier suffix — no
/// embeddings model is served under it, and a chat model id pasted into the
/// free-text custom-model field is exactly how TAURI-RUST-9SK happened
/// (`nvidia/nemotron-3-super-120b-a12b:free` saved as the embeddings model,
/// then 400 "does not exist" on every memory re-embed — 2205 events).
///
/// This is a source-gate for the persist paths that have **no** live verify
/// probe (`config::ops::model::apply_memory_settings`); the Custom-provider
/// setup flow already runs a save-time test embed (`embeddings::rpc::
/// update_settings`), and the broad long tail of chat ids is caught at runtime
/// by the 400 "does not exist" classifier in `core::observability`. Kept
/// deliberately tight: a false positive blocks a user's valid embeddings model.
pub fn non_embedding_model_reason(model: &str) -> Option<&'static str> {
    if model.trim().to_ascii_lowercase().ends_with(":free") {
        return Some(
            "`:free` denotes an OpenRouter chat/reasoning tier, not an embeddings model — \
             pick an embeddings-capable model in Settings → Memory",
        );
    }
    None
}

#[cfg(test)]
#[path = "catalog_tests.rs"]
mod tests;
