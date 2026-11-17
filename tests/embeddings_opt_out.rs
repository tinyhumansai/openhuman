//! Config loading and versioned migration must preserve the embedding opt-out.

use openhuman_core::config::{migrations, Config};

#[tokio::test]
async fn disk_config_keeps_embeddings_disabled_on_repeated_loads() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    let workspace = dir.path().join("workspace");
    std::fs::write(
        &config_path,
        "embeddings_provider = \"none\"\n[memory]\nembedding_provider = \"none\"\nengine = \"\"\n",
    )
    .unwrap();

    for _ in 0..2 {
        let config = Config::load_from_config_path(&config_path, &workspace)
            .await
            .unwrap();
        assert_eq!(config.embeddings_provider.as_deref(), Some("none"));
        assert_eq!(config.memory.embedding_provider, "none");
        config.save().await.unwrap();
    }
}

#[tokio::test]
async fn versioned_migrations_keep_embedding_opt_out_without_a_cloud_provider_entry() {
    let dir = tempfile::tempdir().unwrap();
    let mut config = Config {
        config_path: dir.path().join("config.toml"),
        workspace_dir: dir.path().join("workspace"),
        action_dir: dir.path().join("actions"),
        schema_version: 4,
        embeddings_provider: Some("none".into()),
        reasoning_provider: Some("none".into()),
        ..Config::default()
    };
    config.memory.embedding_provider = "none".into();
    config.memory.engine.clear();

    migrations::run_pending(&mut config).await;

    assert_eq!(config.embeddings_provider.as_deref(), Some("none"));
    assert_eq!(config.reasoning_provider, None);
    let reloaded = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .unwrap();
    assert_eq!(reloaded.embeddings_provider.as_deref(), Some("none"));
}

#[tokio::test]
async fn unknown_embedding_routes_still_fall_back_to_managed() {
    let dir = tempfile::tempdir().unwrap();
    let config_path = dir.path().join("config.toml");
    std::fs::write(&config_path, "embeddings_provider = \"removed-provider\"\n").unwrap();
    let config = Config::load_from_config_path(&config_path, &dir.path().join("workspace"))
        .await
        .unwrap();
    assert_eq!(config.embeddings_provider.as_deref(), Some("openhuman"));
}
