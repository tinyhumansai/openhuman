//! Tests for the embeddings module root (provider factory + trait defaults).
//!
//! Split out of `mod.rs` to keep the module root export-focused. Declared via
//! `#[path = "mod_tests.rs"] mod tests;` so `super::*` still resolves to the
//! `embeddings` module.

use super::*;
use tinyinference_embeddings::{
    DEFAULT_CLOUD_DIMENSIONS as DEFAULT_CLOUD_EMBEDDING_DIMENSIONS,
    DEFAULT_CLOUD_MODEL as DEFAULT_CLOUD_EMBEDDING_MODEL,
};

// ── create_embedding_provider_with_credentials ───────────

#[tokio::test]
async fn factory_with_credentials_voyage() {
    let p = factory::create_embedding_provider_with_credentials(
        "voyage",
        "voyage-3-large",
        1024,
        "voyage-test-key",
        None,
    )
    .await
    .expect("voyage with key");
    assert_eq!(p.name(), "voyage");
    assert_eq!(p.model_id(), "voyage-3-large");
    assert_eq!(p.dimensions(), 1024);
}

#[tokio::test]
async fn factory_with_credentials_cohere() {
    let p = factory::create_embedding_provider_with_credentials(
        "cohere",
        "embed-english-v3.0",
        1024,
        "cohere-test-key",
        None,
    )
    .await
    .expect("cohere with key");
    assert_eq!(p.name(), "cohere");
    assert_eq!(p.model_id(), "embed-english-v3.0");
    assert_eq!(p.dimensions(), 1024);
}

#[tokio::test]
async fn factory_with_credentials_custom() {
    let p = factory::create_embedding_provider_with_credentials(
        "custom",
        "custom-model",
        768,
        "custom-key",
        Some("http://localhost:9999"),
    )
    .await
    .expect("custom provider with endpoint");
    // Custom is backed by OpenAiEmbedding
    assert_eq!(p.name(), "openai");
    assert_eq!(p.dimensions(), 768);
}

#[tokio::test]
async fn factory_with_credentials_managed_ignores_key() {
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
    .await
    .expect("managed ignores key");
    assert_eq!(p.name(), "cloud");
    assert_eq!(p.dimensions(), DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
}
