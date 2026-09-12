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
fn non_embedding_model_reason_rejects_openrouter_free_tier() {
    // TAURI-RUST-9SK — the exact incident id and case/whitespace variants.
    for id in [
        "nvidia/nemotron-3-super-120b-a12b:free",
        "meta-llama/llama-3-70b-instruct:FREE",
        "  some-chat-model:free  ",
    ] {
        assert!(
            non_embedding_model_reason(id).is_some(),
            "{id:?} (`:free` chat tier) must be rejected as an embeddings model"
        );
    }
}

#[test]
fn non_embedding_model_reason_accepts_real_embedding_ids() {
    // Must NOT false-positive on genuine embedding model ids across providers.
    for id in [
        "text-embedding-3-small",
        "text-embedding-3-large",
        "voyage-3-large",
        "embed-english-v3.0",
        "nomic-embed-text:latest",
        "bge-m3",
        "mxbai-embed-large",
        "",
    ] {
        assert!(
            non_embedding_model_reason(id).is_none(),
            "{id:?} is a valid embeddings model and must not be rejected"
        );
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
