//! Tests for the surrounding module.

use async_trait::async_trait;

use super::{format_embedding_signature, EmbeddingProvider, NoopEmbedding};

/// The signature format is a **persisted key**, pinned to literal values.
///
/// Written against golden strings rather than against another copy of the
/// function on purpose: the host used to hold a byte-identical duplicate of
/// this file and the two silently diverged once already. A guard that
/// compares two implementations stops protecting anything the moment one of
/// them goes away — which is exactly what happened when the duplicate was
/// removed. Literals outlive that.
///
/// Every vector on disk is keyed by one of these strings, so a change here
/// is a migration, never an edit.
#[test]
fn signature_format_is_pinned_to_its_persisted_form() {
    assert_eq!(
        format_embedding_signature("ollama", "nomic-embed-text", 768),
        "provider=ollama;model=nomic-embed-text;dims=768"
    );
    assert_eq!(
        format_embedding_signature("none", "none", 0),
        "provider=none;model=none;dims=0"
    );
}

/// Two distinct embedding spaces must never share one signature.
///
/// Without escaping these two collide exactly: both format to
/// `provider=a;model=b;model=c;dims=3`. A collision here is not a cosmetic
/// problem — the signature is what decides which vectors are comparable, so
/// two models' vectors would be scored against each other as though they
/// came from one space.
#[test]
fn delimiter_characters_cannot_make_distinct_spaces_collide() {
    let first = format_embedding_signature("a;model=b", "c", 3);
    let second = format_embedding_signature("a", "b;model=c", 3);
    assert_ne!(first, second);
}

/// Escaping `%` last would make the encoding itself ambiguous.
#[test]
fn an_already_percent_encoded_name_does_not_collide_with_a_literal_one() {
    assert_ne!(
        format_embedding_signature("a%3Bb", "m", 3),
        format_embedding_signature("a;b", "m", 3)
    );
}

/// The escaping is not a migration: every identifier shaped like the ones
/// actually in use formats to the same bytes it always did.
#[test]
fn identifiers_in_real_use_are_untouched_by_the_escaping() {
    for (provider, model) in [
        ("ollama", "nomic-embed-text"),
        ("openai", "text-embedding-3-small"),
        ("huggingface", "sentence-transformers/all-MiniLM-L6-v2"),
        ("local", "bge_base.en-v1.5"),
        ("backend", "tinyhumans:default"),
    ] {
        assert_eq!(
            format_embedding_signature(provider, model, 768),
            format!("provider={provider};model={model};dims=768"),
            "{provider}/{model} must not be rewritten — it is a persisted key"
        );
    }
}

#[tokio::test]
async fn noop_returns_one_empty_vector_per_input() {
    let provider = NoopEmbedding;
    assert_eq!(provider.signature(), "provider=none;model=none;dims=0");
    assert_eq!(
        provider.embed(&["a", "b"]).await.unwrap(),
        vec![Vec::<f32>::new(), Vec::<f32>::new()]
    );
    assert!(provider.embed_one("a").await.unwrap().is_empty());
}

struct EmptyProvider;

#[async_trait]
impl EmbeddingProvider for EmptyProvider {
    fn name(&self) -> &str {
        "empty"
    }
    fn model_id(&self) -> &str {
        "empty"
    }
    fn dimensions(&self) -> usize {
        0
    }
    async fn embed(&self, _: &[&str]) -> anyhow::Result<Vec<Vec<f32>>> {
        Ok(Vec::new())
    }
}

#[tokio::test]
async fn embed_one_rejects_a_provider_that_returns_no_vectors() {
    let error = EmptyProvider.embed_one("text").await.unwrap_err();
    assert!(error.to_string().contains("Empty embedding result"));
}
