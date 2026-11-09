---
description: "Configure SQLite or MongoDB through Embed and inspect the selected backend."
icon: code
---

# Storage drivers

Enable `storage-sqlite` for SQLite or `storage-mongodb` for MongoDB on your Embed dependency. These Cargo gates make the driver available; choosing a URL selects the runtime backend. Set `RuntimeConfig.storage.url`, pass a URL through `RuntimeBuilder::storage`, or set the `url` field in the `[storage]` section of the host's configuration. `OPENHUMAN_STORAGE_URL` is the environment override used by host configuration resolution.

The SQLite example sets `config.storage.url` to a `sqlite:` URL under a temporary directory, then obtains `Runtime::storage`. It creates a scoped collection, writes a document, reads it back, and deletes it. A configured backend also supplies the default driver-backed session stores; an explicit host session-store adapter takes precedence.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/storage.rs#storage_sqlite -->

```rust
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
```

<!-- END EMBED -->

Run `cargo run -p openhuman-embed --example storage --features storage-sqlite`. This performs real local SQLite writes. The [complete storage example](https://github.com/tinyhumansai/openhuman/blob/main/crates/openhuman-embed/examples/storage.rs) also describes a MongoDB URL without connecting.

<!-- BEGIN EMBED: crates/openhuman-embed/examples/storage.rs#storage_mongodb -->

```rust
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
```

<!-- END EMBED -->

`RuntimeBuilder::describe()` reports the driver and whether storage is configured, omitting URL credentials. Describing a MongoDB URL does not prove the server is reachable or that the MongoDB driver was compiled. Building a runtime with that URL requires `storage-mongodb` and a reachable server.

The only live database path in this example is `OPENHUMAN_EXAMPLE_MONGODB_URL`. To exercise it deliberately, enable both `storage-sqlite` and `storage-mongodb` and supply your own MongoDB URL. The offline CI runner clears this setting. A deployed host supplies its persistent URL and credentials through its own configuration.
