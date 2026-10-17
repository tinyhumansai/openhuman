use super::tests_support::ForcedDocumentSource;
use super::*;
use crate::config::Config;
use crate::storage::{MemoryStorage, Scope, ScopedStorage, StorageBackend};
use std::sync::Arc;
use tinystoragedrivers::secrets::{crypto, DerivedKeys, KeyProvider};
use zeroize::Zeroizing;

fn keys(byte: u8) -> Arc<dyn KeyProvider> {
    Arc::new(DerivedKeys::new(Zeroizing::new([byte; 32])))
}

fn scoped(storage: &MemoryStorage, scope: &str) -> ScopedStorage {
    storage.for_scope(&Scope::new(scope).unwrap()).unwrap()
}

fn document_source(storage: &MemoryStorage, scope: &str, file: &Path) -> DocumentConfigSource {
    document_source_keyed(storage, scope, file, keys(1))
}

fn document_source_keyed(
    storage: &MemoryStorage,
    scope: &str,
    file: &Path,
    keys: Arc<dyn KeyProvider>,
) -> DocumentConfigSource {
    DocumentConfigSource::new(
        Arc::clone(scoped(storage, scope).documents()),
        Scope::new(scope).unwrap(),
        keys,
        FileConfigSource::new(file),
    )
}

fn config_at(dir: &Path, model: &str) -> Config {
    Config {
        config_path: dir.join("config.toml"),
        workspace_dir: dir.join("workspace"),
        default_model: Some(model.to_string()),
        ..Default::default()
    }
}

// ── File source ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn the_file_source_round_trips_hand_edits_and_commits_atomically() {
    let tmp = tempfile::tempdir().unwrap();
    let path = tmp.path().join("config.toml");
    let source = FileConfigSource::new(&path);
    assert!(!source.exists().await.unwrap());

    source.write("default_model = \"one\"\n").await.unwrap();
    assert!(source.exists().await.unwrap());

    // A hand edit (with a comment) is read back verbatim.
    std::fs::write(&path, "# my note\ndefault_model = \"hand-edited\"\n").unwrap();
    let read = source.read().await.unwrap();
    assert_eq!(
        read.contents,
        "# my note\ndefault_model = \"hand-edited\"\n"
    );
    assert!(!read.recovered);

    // The next write replaces it atomically: the previous bytes become `.bak`
    // and no staged temp file is left behind.
    source.write("default_model = \"two\"\n").await.unwrap();
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap(),
        "# my note\ndefault_model = \"hand-edited\"\n"
    );
    let leftovers: Vec<_> = std::fs::read_dir(tmp.path())
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|name| name.contains(".tmp-"))
        .collect();
    assert!(leftovers.is_empty(), "{leftovers:?}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
    }
}

#[tokio::test]
async fn a_save_keeps_hand_edits_but_not_comments() {
    // Pins today's behaviour: `Config::save` re-serialises the whole config,
    // so a hand edit to a value survives and a comment does not.
    let tmp = tempfile::tempdir().unwrap();
    let config = config_at(tmp.path(), "before");
    config.save().await.unwrap();
    let edited = std::fs::read_to_string(&config.config_path)
        .unwrap()
        .replace("before", "hand-edited");
    std::fs::write(&config.config_path, format!("# keep me?\n{edited}")).unwrap();

    let reloaded = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .unwrap();
    assert_eq!(reloaded.default_model.as_deref(), Some("hand-edited"));
    reloaded.save().await.unwrap();
    let saved = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(saved.contains("hand-edited"));
    assert!(!saved.contains("keep me?"), "comments are not preserved");
}

// ── Document source ──────────────────────────────────────────────────────────

#[tokio::test]
async fn a_document_holds_no_plaintext_and_no_bootstrap_tables() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.toml");
    let storage = MemoryStorage::new();
    let source = document_source(&storage, "alice", &file);

    source
        .write("default_model = \"m\"\napi_key = \"sk-very-secret\"\n[storage]\nurl = \"mongodb://u:pw@db/x\"\n")
        .await
        .unwrap();

    let docs = Arc::clone(scoped(&storage, "alice").documents());
    let stored = docs
        .get("config.documents", "alice")
        .await
        .unwrap()
        .unwrap();
    let raw = serde_json::to_string(&stored.doc).unwrap();
    assert!(raw.contains("enc2:"), "{raw}");
    for leaked in ["sk-very-secret", "mongodb", "default_model", "[storage]"] {
        assert!(!raw.contains(leaked), "{leaked} leaked into {raw}");
    }
    // It decrypts back to the text without the bootstrap table.
    let read = source.read().await.unwrap().contents;
    assert!(read.contains("sk-very-secret") && !read.contains("mongodb"));
    assert!(source.encrypts_body());
}

#[tokio::test]
async fn a_document_does_not_decrypt_under_another_scope_or_master_key() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let alice = document_source(&storage, "alice", &tmp.path().join("a.toml"));
    alice.write("default_model = \"m\"\n").await.unwrap();

    // Another master key fails closed.
    let wrong_key = document_source_keyed(&storage, "alice", &tmp.path().join("a.toml"), keys(2));
    assert!(wrong_key.read().await.is_err());

    // Another scope's data key cannot open alice's ciphertext even when the
    // sealed value is copied across (per-scope key derivation).
    let docs = Arc::clone(scoped(&storage, "alice").documents());
    let sealed = docs
        .get("config.documents", "alice")
        .await
        .unwrap()
        .unwrap()
        .doc;
    let bob_docs = Arc::clone(scoped(&storage, "bob").documents());
    bob_docs
        .put(
            "config.documents",
            "bob",
            sealed,
            tinystoragedrivers::Precondition::None,
        )
        .await
        .unwrap();
    let bob = document_source(&storage, "bob", &tmp.path().join("b.toml"));
    assert!(bob.read().await.is_err(), "copied ciphertext must not open");
}

#[tokio::test]
async fn a_malformed_bootstrap_file_is_an_error_not_an_empty_one() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.toml");
    let storage = MemoryStorage::new();
    let source = document_source(&storage, "alice", &file);
    source.write("default_model = \"m\"\n").await.unwrap();
    std::fs::write(&file, "[storage\n").unwrap();
    assert!(source.read().await.is_err());
}

#[tokio::test]
async fn a_document_read_takes_its_bootstrap_tables_from_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.toml");
    std::fs::write(&file, "[storage]\nurl = \"sqlite:/data\"\n").unwrap();
    let storage = MemoryStorage::new();
    let source = document_source(&storage, "alice", &file);
    source.write("default_model = \"m\"\n").await.unwrap();

    let read = source.read().await.unwrap();
    let table: toml::Table = toml::from_str(&read.contents).unwrap();
    assert_eq!(table["default_model"].as_str(), Some("m"));
    assert_eq!(table["storage"]["url"].as_str(), Some("sqlite:/data"));

    // Even a document that somehow carries a [storage] table is overruled.
    let docs = Arc::clone(scoped(&storage, "alice").documents());
    let key = keys(1).data_key(&Scope::new("alice").unwrap()).unwrap();
    let sealed = crypto::encrypt_enc2(&key, b"[storage]\nurl = \"evil\"\n").unwrap();
    docs.put(
        "config.documents",
        "alice",
        serde_json::json!({ "toml_enc": sealed }),
        tinystoragedrivers::Precondition::None,
    )
    .await
    .unwrap();
    let read = source.read().await.unwrap();
    assert!(!read.contents.contains("evil"), "{}", read.contents);
    assert!(read.contents.contains("sqlite:/data"));
}

#[tokio::test]
async fn without_a_document_the_source_reads_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.toml");
    let storage = MemoryStorage::new();
    let source = document_source(&storage, "alice", &file);
    assert!(!source.exists().await.unwrap());

    std::fs::write(&file, "default_model = \"from-file\"\n").unwrap();
    assert!(source.exists().await.unwrap());
    assert_eq!(
        source.read().await.unwrap().contents,
        "default_model = \"from-file\"\n"
    );
    assert_eq!(source.label(), "document");
}

#[tokio::test]
async fn two_scopes_keep_their_config_apart() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let alice = document_source(&storage, "alice", &tmp.path().join("a.toml"));
    let bob = document_source(&storage, "bob", &tmp.path().join("b.toml"));

    alice
        .write("default_model = \"alice-model\"\n")
        .await
        .unwrap();
    assert!(alice.exists().await.unwrap());
    assert!(
        !bob.exists().await.unwrap(),
        "bob sees nothing of alice's config"
    );

    bob.write("default_model = \"bob-model\"\n").await.unwrap();
    assert!(alice.read().await.unwrap().contents.contains("alice-model"));
    assert!(bob.read().await.unwrap().contents.contains("bob-model"));
    assert!(!bob.read().await.unwrap().contents.contains("alice-model"));
}

// ── Through Config ───────────────────────────────────────────────────────────

#[tokio::test]
async fn config_save_and_reload_use_the_document_on_a_shared_backend() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let _forced = ForcedDocumentSource::new(
        scoped(&storage, "tenant-a"),
        Scope::new("tenant-a").unwrap(),
        keys(3),
    );

    let mut config = config_at(tmp.path(), "doc-model");
    config.storage.url = Some("sqlite:/bootstrap".to_string());
    config.save().await.unwrap();
    assert!(
        !config.config_path.exists(),
        "a document-backed save does not touch the file"
    );

    // The bootstrap table comes from the file, which is the only place it lives.
    std::fs::write(
        &config.config_path,
        "[storage]\nurl = \"sqlite:/bootstrap\"\n",
    )
    .unwrap();
    let reloaded = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .unwrap();
    assert_eq!(reloaded.default_model.as_deref(), Some("doc-model"));
    assert_eq!(reloaded.storage.url.as_deref(), Some("sqlite:/bootstrap"));
}

#[tokio::test]
async fn without_a_shared_backend_config_stays_on_the_file() {
    // No forced scope and no installed backend: the file source.
    let tmp = tempfile::tempdir().unwrap();
    let config = config_at(tmp.path(), "file-model");
    config.save().await.unwrap();
    assert!(config.config_path.exists());
    assert_eq!(for_config(&config.config_path).unwrap().label(), "file");
}

#[tokio::test]
async fn a_document_backed_config_keeps_secrets_readable_on_reload() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let _forced = ForcedDocumentSource::new(
        scoped(&storage, "tenant-a"),
        Scope::new("tenant-a").unwrap(),
        keys(3),
    );
    let mut config = config_at(tmp.path(), "m");
    config.api_key = Some("sk-portable".to_string());
    config.save().await.unwrap();

    // Sealed as a whole under the scope key, not field-by-field under the
    // process-local key: the stored text has no per-field ciphertext, and a
    // reload hands back the usable value.
    let reloaded = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .unwrap();
    assert_eq!(reloaded.api_key.as_deref(), Some("sk-portable"));
}

#[test]
fn forced_sources_nest_and_restore() {
    let storage = MemoryStorage::new();
    let outer = ForcedDocumentSource::new(
        scoped(&storage, "outer"),
        Scope::new("outer").unwrap(),
        keys(1),
    );
    {
        let _inner = ForcedDocumentSource::new(
            scoped(&storage, "inner"),
            Scope::new("inner").unwrap(),
            keys(1),
        );
        assert_eq!(
            tests_support::forced_document_scope()
                .unwrap()
                .scope
                .as_str(),
            "inner"
        );
    }
    assert_eq!(
        tests_support::forced_document_scope()
            .unwrap()
            .scope
            .as_str(),
        "outer"
    );
    drop(outer);
    assert!(tests_support::forced_document_scope().is_none());
}

#[tokio::test]
async fn a_present_but_malformed_document_is_an_error_not_absence() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.toml");
    std::fs::write(&file, "default_model = \"stale-file\"\n").unwrap();
    let storage = MemoryStorage::new();
    let source = document_source(&storage, "alice", &file);
    let docs = Arc::clone(scoped(&storage, "alice").documents());
    docs.put(
        "config.documents",
        "alice",
        serde_json::json!({ "toml_enc": 123 }),
        tinystoragedrivers::Precondition::None,
    )
    .await
    .unwrap();
    assert!(source.exists().await.unwrap());
    assert!(
        source.read().await.is_err(),
        "must not fall back to the file"
    );
}

#[tokio::test]
async fn a_document_that_does_not_parse_is_an_error_and_leaves_the_bootstrap_file_alone() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let _forced = ForcedDocumentSource::new(
        scoped(&storage, "tenant-a"),
        Scope::new("tenant-a").unwrap(),
        keys(3),
    );
    let config = config_at(tmp.path(), "m");
    let docs = Arc::clone(scoped(&storage, "tenant-a").documents());
    let key = keys(3).data_key(&Scope::new("tenant-a").unwrap()).unwrap();
    let sealed = crypto::encrypt_enc2(&key, b"default_model = 5\n").unwrap();
    docs.put(
        "config.documents",
        "tenant-a",
        serde_json::json!({ "toml_enc": sealed }),
        tinystoragedrivers::Precondition::None,
    )
    .await
    .unwrap();
    let bootstrap = "[storage]\nurl = \"sqlite:/b\"\n";
    std::fs::write(&config.config_path, bootstrap).unwrap();

    let error = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .unwrap_err();
    assert!(
        format!("{error:#}").contains("could not be parsed"),
        "{error:#}"
    );
    assert_eq!(
        std::fs::read_to_string(&config.config_path).unwrap(),
        bootstrap
    );
    assert!(!tmp.path().join("config.toml.bak").exists());
}

#[tokio::test]
async fn a_read_reports_whether_its_text_is_a_document_body() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.toml");
    std::fs::write(&file, "default_model = \"from-file\"\n").unwrap();
    let storage = MemoryStorage::new();
    let source = document_source(&storage, "alice", &file);
    assert!(!source.read().await.unwrap().from_document, "file fallback");
    source.write("default_model = \"doc\"\n").await.unwrap();
    assert!(source.read().await.unwrap().from_document);
    assert!(
        !FileConfigSource::new(&file)
            .read()
            .await
            .unwrap()
            .from_document
    );
}

#[tokio::test]
async fn a_recovered_bootstrap_file_is_restored_beside_the_document() {
    let tmp = tempfile::tempdir().unwrap();
    let file = tmp.path().join("config.toml");
    let storage = MemoryStorage::new();
    let source = document_source(&storage, "alice", &file);
    source.write("default_model = \"m\"\n").await.unwrap();
    let bootstrap = "[storage]\nurl = \"sqlite:/b\"\n";
    std::fs::write(tmp.path().join("config.toml.bak"), bootstrap).unwrap();
    std::fs::write(&file, [0xff, 0xfe, 0x00, 0x80]).unwrap();

    let read = source.read().await.unwrap();
    assert!(read.contents.contains("sqlite:/b"), "{}", read.contents);
    assert!(!read.recovered, "the document body itself was intact");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), bootstrap);
}

#[tokio::test]
async fn a_file_fallback_keeps_the_file_backup_recovery() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let _forced = ForcedDocumentSource::new(
        scoped(&storage, "tenant-a"),
        Scope::new("tenant-a").unwrap(),
        keys(3),
    );
    let config = config_at(tmp.path(), "unused");
    std::fs::write(&config.config_path, "default_model = 5\n").unwrap();
    std::fs::write(
        tmp.path().join("config.toml.bak"),
        "default_model = \"from-bak\"\n",
    )
    .unwrap();

    let reloaded = Config::load_from_config_path(&config.config_path, &config.workspace_dir)
        .await
        .unwrap();
    assert_eq!(reloaded.default_model.as_deref(), Some("from-bak"));
}

#[tokio::test]
async fn a_document_save_never_nests_process_key_ciphertext() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let _forced = ForcedDocumentSource::new(
        scoped(&storage, "tenant-a"),
        Scope::new("tenant-a").unwrap(),
        keys(3),
    );
    let mut config = config_at(tmp.path(), "m");
    assert!(config.secrets.encrypt);
    // A field still sealed under this node's key, as a file-seeded config
    // holds it before the load decrypts.
    let sealed = crate::security::secret_storage::SecretStore::new(tmp.path(), true)
        .encrypt("sk-node-local")
        .unwrap();
    assert!(sealed.starts_with("enc2:"));
    config.api_key = Some(sealed);
    config.save().await.unwrap();

    let body = document_source_keyed(&storage, "tenant-a", &config.config_path, keys(3))
        .read()
        .await
        .unwrap()
        .contents;
    assert!(body.contains("sk-node-local"), "{body}");
    assert!(!body.contains("enc2:"), "{body}");
}

#[tokio::test]
async fn a_document_save_refuses_process_key_secrets_it_cannot_open() {
    let tmp = tempfile::tempdir().unwrap();
    let other_node = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let _forced = ForcedDocumentSource::new(
        scoped(&storage, "tenant-a"),
        Scope::new("tenant-a").unwrap(),
        keys(3),
    );
    let mut config = config_at(tmp.path(), "m");
    // Sealed under another node's key: this node cannot open it.
    config.api_key = Some(
        crate::security::secret_storage::SecretStore::new(other_node.path(), true)
            .encrypt("sk-elsewhere")
            .unwrap(),
    );
    let error = config.save().await.unwrap_err();
    assert!(
        format!("{error:#}").contains("could not be opened"),
        "{error:#}"
    );
    let docs = Arc::clone(scoped(&storage, "tenant-a").documents());
    docs.ensure_collection(&tinystoragedrivers::CollectionSpec::new("config.documents"))
        .await
        .unwrap();
    assert!(
        docs.get("config.documents", "tenant-a")
            .await
            .unwrap()
            .is_none(),
        "nothing was written over the shared document"
    );
}

#[tokio::test]
async fn a_recovered_file_fallback_rewrites_the_bootstrap_file_too() {
    let tmp = tempfile::tempdir().unwrap();
    let storage = MemoryStorage::new();
    let _forced = ForcedDocumentSource::new(
        scoped(&storage, "tenant-a"),
        Scope::new("tenant-a").unwrap(),
        keys(3),
    );
    let mut config = config_at(tmp.path(), "recovered");
    config.storage.url = Some("sqlite:/boot".to_string());
    let source = for_config(&config.config_path).unwrap();
    assert_eq!(source.label(), "document");

    super::super::branches::save_recovered(&config, source.as_ref())
        .await
        .unwrap();

    let file = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(file.contains("[storage]"), "{file}");
    let body = source.read().await.unwrap();
    assert!(body.from_document);
    assert!(body.contents.contains("recovered"));
}
