//! One-shot memory doctor over the active user's real workspace:
//!
//!     cargo run --example run_memory_doctor
//!
//! Run it with NO `OPENHUMAN_WORKSPACE` override for a logged-in install —
//! the loader then resolves the active-user marker exactly like the app and
//! reads `users/<id>/config.toml`. Pointing the env var at a user-scoped
//! workspace instead synthesizes `users/<id>/.openhuman/config.toml`, which
//! does not exist, and the doctor silently reports a default (unconfigured)
//! config. The stderr probe line prints which config actually loaded so a
//! wrong verdict is visible as a wrong path.

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let mut config = openhuman_core::openhuman::config::Config::load_or_init()
        .await
        .unwrap_or_default();
    config.apply_env_overrides();

    openhuman_core::openhuman::memory::host::install_memory_event_sink();
    #[cfg(feature = "modules")]
    openhuman_core::openhuman::modules::memory::set_modules_policy(std::sync::Arc::new(
        config.clone(),
    ));

    eprintln!(
        "config_path={} embeddings_provider={:?} embedding_endpoint={:?}",
        config.config_path.display(),
        config.embeddings_provider,
        config.memory_tree.embedding_endpoint,
    );

    let report = openhuman_core::openhuman::memory::tree::health::report::run_doctor(&config).await;
    println!("{}", serde_json::to_string_pretty(&report)?);
    Ok(())
}
