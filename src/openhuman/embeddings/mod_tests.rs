//! Tests for the embeddings module root (provider factory + trait defaults).
//!
//! Split out of `mod.rs` to keep the module root export-focused. Declared via
//! `#[path = "mod_tests.rs"] mod tests;` so `super::*` still resolves to the
//! `embeddings` module.

use super::*;

// ── Trait default method ─────────────────────────────────

#[test]
fn noop_name_and_dims() {
    let p = NoopEmbedding;
    assert_eq!(p.name(), "none");
    assert_eq!(p.model_id(), "none");
    assert_eq!(p.dimensions(), 0);
    assert_eq!(p.signature(), "provider=none;model=none;dims=0");
}

#[tokio::test]
async fn noop_embed_returns_empty() {
    let p = NoopEmbedding;
    let result = p.embed(&["hello"]).await.unwrap();
    assert!(result.is_empty());
}

#[tokio::test]
async fn noop_embed_one_returns_error() {
    // embed returns empty vec → pop() returns None → error from default impl
    let p = NoopEmbedding;
    let err = p.embed_one("hello").await.unwrap_err();
    assert!(err.to_string().contains("Empty embedding result"));
}

#[tokio::test]
async fn noop_embed_empty_batch() {
    let p = NoopEmbedding;
    let result = p.embed(&[]).await.unwrap();
    assert!(result.is_empty());
}

// ── Factory — success ────────────────────────────────────

#[test]
fn factory_ollama() {
    let p = create_embedding_provider("ollama", DEFAULT_OLLAMA_MODEL, 768).unwrap();
    assert_eq!(p.name(), "ollama");
    assert_eq!(p.model_id(), DEFAULT_OLLAMA_MODEL);
    assert_eq!(p.dimensions(), 768);
    assert_eq!(p.signature(), "provider=ollama;model=bge-m3;dims=768");
}

#[test]
fn factory_openai() {
    let p = create_embedding_provider("openai", "text-embedding-3-small", 1536).unwrap();
    assert_eq!(p.name(), "openai");
    assert_eq!(p.model_id(), "text-embedding-3-small");
    assert_eq!(p.dimensions(), 1536);
}

#[test]
fn factory_custom_url() {
    let p = create_embedding_provider("custom:http://localhost:1234", "model", 768).unwrap();
    assert_eq!(p.name(), "openai"); // OpenAI-compatible under the hood
    assert_eq!(p.dimensions(), 768);
}

#[test]
fn factory_custom_empty_url() {
    let p = create_embedding_provider("custom:", "model", 768).unwrap();
    assert_eq!(p.name(), "openai");
}

#[test]
fn factory_none() {
    let p = create_embedding_provider("none", "", 0).unwrap();
    assert_eq!(p.name(), "none");
    assert_eq!(p.dimensions(), 0);
}

#[test]
fn factory_voyage() {
    let p = create_embedding_provider("voyage", "voyage-3-large", 1024).unwrap();
    assert_eq!(p.name(), "voyage");
    assert_eq!(p.dimensions(), 1024);
}

#[test]
fn factory_cohere() {
    let p = create_embedding_provider("cohere", "embed-english-v3.0", 1024).unwrap();
    assert_eq!(p.name(), "cohere");
    assert_eq!(p.dimensions(), 1024);
}

// ── Factory — errors ─────────────────────────────────────

#[test]
fn factory_unknown_provider_errors() {
    let result = create_embedding_provider("deepseek", "model", 1536);
    let msg = result.err().expect("should be an error").to_string();
    assert!(
        msg.contains("deepseek"),
        "should include provider name: {msg}"
    );
    assert!(msg.contains("unknown"), "should say unknown: {msg}");
}

#[test]
fn factory_empty_string_errors() {
    let result = create_embedding_provider("", "model", 1536);
    assert!(result
        .err()
        .expect("should error")
        .to_string()
        .contains("unknown"));
}

#[test]
fn factory_fastembed_errors() {
    let result = create_embedding_provider("fastembed", "BGESmallENV15", 384);
    assert!(result
        .err()
        .expect("should error")
        .to_string()
        .contains("fastembed"));
}

#[test]
fn factory_cloud() {
    let p = create_embedding_provider(
        "cloud",
        DEFAULT_CLOUD_EMBEDDING_MODEL,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    )
    .unwrap();
    assert_eq!(p.name(), "cloud");
    assert_eq!(p.dimensions(), DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
}

#[test]
fn factory_managed() {
    let p = create_embedding_provider(
        "managed",
        DEFAULT_CLOUD_EMBEDDING_MODEL,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    )
    .unwrap();
    assert_eq!(p.name(), "cloud");
    assert_eq!(p.dimensions(), DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
}

// ── Default provider ─────────────────────────────────────

#[test]
fn default_provider_uses_cloud() {
    let p = default_embedding_provider();
    assert_eq!(p.name(), "cloud");
    assert_eq!(p.dimensions(), DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
}

#[test]
fn default_local_provider_uses_ollama() {
    let p = default_local_embedding_provider();
    assert_eq!(p.name(), "ollama");
    assert_eq!(p.dimensions(), DEFAULT_OLLAMA_DIMENSIONS);
}

// ── create_embedding_provider_with_credentials ───────────

#[test]
fn factory_with_credentials_voyage() {
    let p = factory::create_embedding_provider_with_credentials(
        "voyage",
        "voyage-3-large",
        1024,
        "voyage-test-key",
        None,
    )
    .expect("voyage with key");
    assert_eq!(p.name(), "voyage");
    assert_eq!(p.model_id(), "voyage-3-large");
    assert_eq!(p.dimensions(), 1024);
}

#[test]
fn factory_with_credentials_cohere() {
    let p = factory::create_embedding_provider_with_credentials(
        "cohere",
        "embed-english-v3.0",
        1024,
        "cohere-test-key",
        None,
    )
    .expect("cohere with key");
    assert_eq!(p.name(), "cohere");
    assert_eq!(p.model_id(), "embed-english-v3.0");
    assert_eq!(p.dimensions(), 1024);
}

#[test]
fn factory_with_credentials_custom() {
    let p = factory::create_embedding_provider_with_credentials(
        "custom",
        "custom-model",
        768,
        "custom-key",
        Some("http://localhost:9999"),
    )
    .expect("custom provider with endpoint");
    // Custom is backed by OpenAiEmbedding
    assert_eq!(p.name(), "openai");
    assert_eq!(p.dimensions(), 768);
}

#[test]
fn factory_with_credentials_managed_ignores_key() {
    // Managed/cloud provider does not use the API key — it routes through
    // the OpenHuman backend. Creating it with an arbitrary key must succeed
    // and produce the cloud provider.
    let p = factory::create_embedding_provider_with_credentials(
        "managed",
        DEFAULT_CLOUD_EMBEDDING_MODEL,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
        "should-be-ignored",
        None,
    )
    .expect("managed ignores key");
    assert_eq!(p.name(), "cloud");
    assert_eq!(p.dimensions(), DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
}
