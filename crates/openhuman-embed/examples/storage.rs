//! Title: SQLite writes and MongoDB configuration through Embed
//! Summary: Open a real local SQLite backend and describe a MongoDB deployment without connecting.
//! Run: offline; optional live MongoDB with OPENHUMAN_EXAMPLE_MONGODB_URL and storage-mongodb.
//! Feature: storage-sqlite

mod support;
use openhuman_embed::seams::{CollectionSpec, Precondition, Scope};
use openhuman_embed::{Runtime, RuntimeBuilder, Workspace};

fn main() -> anyhow::Result<()> {
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let directory = tempfile::tempdir()?;
    // ANCHOR: storage_sqlite
    let mut config = support::offline_config();
    config.storage.url = Some(format!(
        "sqlite:{}",
        directory.path().join("storage").display()
    ));
    let runtime = Runtime::builder()
        .workspace(Workspace::Ephemeral)
        .config(config)
        .backend_url(backend.uri())
        .build()
        .await?;
    let storage = runtime.storage().expect("configured SQLite backend");
    let scoped = storage.for_scope(&Scope::local())?;
    let documents = scoped.documents();
    documents
        .ensure_collection(&CollectionSpec::new("settings"))
        .await?;
    let value = serde_json::json!({"theme":"dark"});
    documents
        .put("settings", "demo", value.clone(), Precondition::Absent)
        .await?;
    assert_eq!(
        documents
            .get("settings", "demo")
            .await?
            .expect("stored value")
            .doc,
        value
    );
    assert!(
        documents
            .delete("settings", "demo", Precondition::None)
            .await?
    );
    assert!(documents.get("settings", "demo").await?.is_none());
    assert_eq!(
        runtime.capabilities().storage.driver.as_deref(),
        Some("sqlite")
    );
    // ANCHOR_END: storage_sqlite
    drop(scoped);
    drop(storage);
    drop(runtime);

    // ANCHOR: storage_mongodb
    // Enable `storage-mongodb` for a deployment that builds this config.
    // Describing it does not contact MongoDB and never exposes credentials.
    let mut mongo = support::offline_config();
    mongo.storage.url = Some("mongodb://app:example-password@127.0.0.1:27017/openhuman".into());
    let builder = RuntimeBuilder::library().config(mongo);
    let description = builder.describe();
    assert_eq!(description.storage.driver.as_deref(), Some("mongodb"));
    assert!(description.storage.configured);
    assert!(!serde_json::to_string(&description)?.contains("example-password"));
    // An explicit environment variable is the only path to a live database.
    if let Ok(url) = std::env::var("OPENHUMAN_EXAMPLE_MONGODB_URL") {
        let live = RuntimeBuilder::library().storage(url).build().await?;
        assert!(live.storage().is_some(), "MongoDB backend");
        assert_eq!(
            live.capabilities().storage.driver.as_deref(),
            Some("mongodb")
        );
    }
    // ANCHOR_END: storage_mongodb
    println!("SQLite put/get/delete and credential-safe MongoDB configuration verified");
    support::passed("storage");
    Ok(())
}
