use super::*;
use crate::openhuman::embeddings::{
    DEFAULT_CLOUD_EMBEDDING_DIMENSIONS, DEFAULT_CLOUD_EMBEDDING_MODEL, DEFAULT_OLLAMA_DIMENSIONS,
    DEFAULT_OLLAMA_MODEL,
};

fn config_with_provider(provider: &str, model: &str, dims: usize) -> Config {
    let mut config = Config::default();
    config.memory.embedding_provider = provider.to_string();
    config.memory.embedding_model = model.to_string();
    config.memory.embedding_dimensions = dims;
    config
}

#[test]
fn rewrites_fastembed_to_managed_with_cloud_defaults() {
    let mut config = config_with_provider("fastembed", "BGESmallENV15", 384);

    // No local Ollama reachable ⇒ managed cloud target.
    let stats = run(&mut config, false).expect("migration should succeed");

    assert!(stats.provider_migrated, "fastembed must be migrated");
    assert!(!stats.migrated_to_local, "managed target is not local");
    assert_eq!(stats.old_dimensions, 384);
    assert_eq!(stats.new_dimensions, DEFAULT_CLOUD_EMBEDDING_DIMENSIONS);
    assert_eq!(config.memory.embedding_provider, "managed");
    assert_eq!(config.memory.embedding_model, DEFAULT_CLOUD_EMBEDDING_MODEL);
    assert_eq!(
        config.memory.embedding_dimensions,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS
    );
}

#[test]
fn rewrites_fastembed_to_ollama_when_local_preferred() {
    let mut config = config_with_provider("fastembed", "BGESmallENV15", 384);

    // A reachable local Ollama ⇒ stay local (preserve offline intent).
    let stats = run(&mut config, true).expect("migration should succeed");

    assert!(stats.provider_migrated, "fastembed must be migrated");
    assert!(stats.migrated_to_local, "ollama target is local");
    assert_eq!(stats.old_dimensions, 384);
    assert_eq!(stats.new_dimensions, DEFAULT_OLLAMA_DIMENSIONS);
    assert_eq!(config.memory.embedding_provider, "ollama");
    assert_eq!(config.memory.embedding_model, DEFAULT_OLLAMA_MODEL);
    assert_eq!(
        config.memory.embedding_dimensions,
        DEFAULT_OLLAMA_DIMENSIONS
    );
    // Both targets land on the memory tree's fixed 1024-dim format.
    assert_eq!(
        DEFAULT_OLLAMA_DIMENSIONS,
        DEFAULT_CLOUD_EMBEDDING_DIMENSIONS
    );
}

#[test]
fn is_idempotent() {
    let mut config = config_with_provider("fastembed", "BGESmallENV15", 384);
    run(&mut config, false).expect("first run");
    let stats = run(&mut config, false).expect("second run");
    assert!(
        !stats.provider_migrated,
        "second run must be a no-op once provider is rewritten"
    );
    assert_eq!(config.memory.embedding_provider, "managed");
}

#[test]
fn is_idempotent_after_local_rewrite() {
    let mut config = config_with_provider("fastembed", "BGESmallENV15", 384);
    run(&mut config, true).expect("first run");
    // Provider is now "ollama" (not "fastembed"), so a second run is a no-op
    // regardless of the prefer_local flag.
    let stats = run(&mut config, false).expect("second run");
    assert!(!stats.provider_migrated, "second run must be a no-op");
    assert_eq!(config.memory.embedding_provider, "ollama");
}

#[test]
fn matches_case_insensitively_and_trims() {
    let mut config = config_with_provider("  FastEmbed  ", "BGESmallENV15", 384);
    let stats = run(&mut config, false).expect("migration should succeed");
    assert!(stats.provider_migrated);
    assert_eq!(config.memory.embedding_provider, "managed");
}

#[test]
fn leaves_valid_providers_untouched() {
    for provider in ["managed", "ollama", "voyage", "none", "openai"] {
        // Untouched regardless of the prefer_local flag — only "fastembed" is rewritten.
        for prefer_local in [false, true] {
            let mut config = config_with_provider(provider, "some-model", 1024);
            let stats = run(&mut config, prefer_local).expect("migration should succeed");
            assert!(!stats.provider_migrated, "{provider} must not be migrated");
            assert_eq!(config.memory.embedding_provider, provider);
            assert_eq!(config.memory.embedding_model, "some-model");
            assert_eq!(config.memory.embedding_dimensions, 1024);
        }
    }
}

#[tokio::test]
async fn reachable_probe_false_when_no_server() {
    // Nothing listening ⇒ connection refused ⇒ false (caller falls back to managed).
    assert!(!local_ollama_reachable("http://127.0.0.1:1").await);
}

#[tokio::test]
async fn reachable_probe_true_for_2xx_server() {
    use axum::{routing::get, Router};
    use tokio::net::TcpListener;

    let app = Router::new().route("/api/tags", get(|| async { "{\"models\":[]}" }));
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });

    // Trailing slash exercises the `trim_end_matches('/')` join.
    assert!(local_ollama_reachable(&format!("http://{addr}/")).await);
}
