use super::*;
use tinystoragedrivers::secrets::DerivedKeys;
use tinystoragedrivers::{MemoryStorage, Scope, StorageBackend};
use zeroize::Zeroizing;

/// The first-generation schema: no `transport`, `deployment_url`, `enabled` or
/// `catalog_managed` column, which older installs never gained.
const OLD_CLIENTS: &str = "
    CREATE TABLE mcp_servers (
        server_id TEXT PRIMARY KEY, qualified_name TEXT NOT NULL, display_name TEXT NOT NULL,
        description TEXT, icon_url TEXT, command_kind TEXT NOT NULL DEFAULT 'node',
        command TEXT NOT NULL, args_json TEXT NOT NULL DEFAULT '[]',
        env_keys_json TEXT NOT NULL DEFAULT '[]', config_json TEXT,
        installed_at INTEGER NOT NULL, last_connected_at INTEGER
    );
    CREATE TABLE mcp_client_env (
        server_id TEXT NOT NULL, key TEXT NOT NULL, value TEXT NOT NULL,
        PRIMARY KEY (server_id, key)
    );
    CREATE TABLE mcp_registry_cache (cache_key TEXT PRIMARY KEY, body_json TEXT, cached_at INTEGER);
    INSERT INTO mcp_servers VALUES
        ('srv-1', 'io.example/files', 'Files', 'Reads files', NULL, 'npx', 'files-mcp',
         '[\"--root\",\"/tmp\"]', '[\"API_KEY\",\"REGION\"]', '{\"k\":1}', 1700000000000, 1700000005000),
        ('srv-2', 'io.example/plain', 'Plain', NULL, NULL, 'node', 'plain.js', '[]', '[]', NULL,
         1700000100000, NULL);
    INSERT INTO mcp_client_env VALUES
        ('srv-1', 'API_KEY', 'sk-secret'), ('srv-1', 'REGION', 'eu');
    INSERT INTO mcp_registry_cache VALUES ('browse', '{}', 1);
";

const AUDIT: &str = "
    CREATE TABLE mcp_writes (
        id INTEGER PRIMARY KEY AUTOINCREMENT, timestamp_ms INTEGER NOT NULL,
        client_info TEXT NOT NULL, tool_name TEXT NOT NULL, args_summary TEXT,
        resulting_chunk_id TEXT, success INTEGER NOT NULL, error_message TEXT
    );
    INSERT INTO mcp_writes (timestamp_ms, client_info, tool_name, args_summary, resulting_chunk_id, success, error_message)
    VALUES (10, 'claude', 'memory.write', '{\"len\":3}', 'chunk-1', 1, NULL),
           (20, 'claude', 'memory.write', '{}', NULL, 0, 'boom');
";

fn legacy_dir() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (path, schema) in [(CLIENTS_DB, OLD_CLIENTS), (AUDIT_DB, AUDIT)] {
        let file = dir.path().join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        Connection::open(file)
            .unwrap()
            .execute_batch(schema)
            .unwrap();
    }
    dir
}

struct Rig {
    scoped: ScopedStorage,
    secrets: DocumentSecrets,
}

fn rig() -> Rig {
    let backend = MemoryStorage::new();
    let scoped = backend
        .for_scope(&Scope::new("module:tinymcp@local").unwrap())
        .unwrap();
    let secrets = DocumentSecrets::new(
        &scoped,
        Arc::new(DerivedKeys::new(Zeroizing::new([3u8; 32]))),
    );
    Rig { scoped, secrets }
}

fn digest(path: &Path) -> Vec<u8> {
    sha2::Sha256::digest(std::fs::read(path).unwrap()).to_vec()
}

#[tokio::test]
async fn servers_env_and_audit_rows_move_into_the_modules_storage() {
    let legacy = legacy_dir();
    let rig = rig();
    let outcome = import_into(
        &DispatchCtx::host_default(),
        &rig.scoped,
        &rig.secrets,
        legacy.path(),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        Outcome::Imported {
            servers: 2,
            secrets: 1,
            audit: 2
        }
    );

    let docs = rig.scoped.documents();
    let one = docs.get(SERVERS, "srv-1").await.unwrap().unwrap();
    let body = one.doc;
    assert_eq!(body["qualified_name"], "io.example/files");
    assert_eq!(body["args"], json!(["--root", "/tmp"]));
    assert_eq!(body["env_keys"], json!(["API_KEY", "REGION"]));
    assert_eq!(body["config"], json!({"k": 1}));
    // Columns the old schema lacked read as what every old row was.
    assert_eq!(body["transport"], "stdio");
    assert_eq!(body["enabled"], true);
    assert_eq!(body["catalog_managed"], false);
    assert_eq!(body["last_connected_at"], 1_700_000_005_000_i64);
    let claim_id = hex::encode(sha2::Sha256::digest(b"io.example/files"));
    let claim = docs.get(SERVER_NAMES, &claim_id).await.unwrap().unwrap();
    assert_eq!(claim.doc["server_id"], "srv-1");

    // Env values only ever land in the secret, never in a document.
    assert!(!body.to_string().contains("sk-secret"));
    let stored = rig
        .secrets
        .get("mcp_client_env/srv-1")
        .await
        .unwrap()
        .expect("env secret");
    let env: std::collections::BTreeMap<String, String> = serde_json::from_slice(&stored).unwrap();
    assert_eq!(env["API_KEY"], "sk-secret");
    assert_eq!(env["REGION"], "eu");
    assert!(rig
        .secrets
        .get("mcp_client_env/srv-2")
        .await
        .unwrap()
        .is_none());

    let audit = rig
        .scoped
        .streams()
        .read_window(AUDIT_STREAM, 0, 10)
        .await
        .unwrap();
    assert_eq!(audit.len(), 2);
    assert_eq!(audit[0].value["tool_name"], "memory.write");
    assert_eq!(audit[0].value["resulting_chunk_id"], "chunk-1");
    assert_eq!(audit[0].value["args_summary"], json!({"len": 3}));
    assert_eq!(audit[1].value["success"], false);
    assert_eq!(audit[1].value["error_message"], "boom");
}

#[tokio::test]
async fn the_import_runs_once_and_never_resurrects_a_deleted_server() {
    let legacy = legacy_dir();
    let rig = rig();
    import_into(
        &DispatchCtx::host_default(),
        &rig.scoped,
        &rig.secrets,
        legacy.path(),
    )
    .await
    .unwrap();
    let docs = rig.scoped.documents();
    let held = docs.get(SERVERS, "srv-1").await.unwrap().unwrap();
    docs.delete(SERVERS, "srv-1", held.unchanged())
        .await
        .unwrap();

    let again = import_into(
        &DispatchCtx::host_default(),
        &rig.scoped,
        &rig.secrets,
        legacy.path(),
    )
    .await
    .unwrap();
    assert_eq!(again, Outcome::AlreadyDone);
    assert!(docs.get(SERVERS, "srv-1").await.unwrap().is_none());
    assert_eq!(
        rig.scoped.streams().len(AUDIT_STREAM).await.unwrap(),
        2,
        "the audit log is not duplicated"
    );
}

#[tokio::test]
async fn the_legacy_files_are_left_untouched() {
    let legacy = legacy_dir();
    let before = [CLIENTS_DB, AUDIT_DB].map(|path| digest(&legacy.path().join(path)));
    let rig = rig();
    import_into(
        &DispatchCtx::host_default(),
        &rig.scoped,
        &rig.secrets,
        legacy.path(),
    )
    .await
    .unwrap();
    let after = [CLIENTS_DB, AUDIT_DB].map(|path| digest(&legacy.path().join(path)));
    assert_eq!(before, after);
    assert!(!legacy
        .path()
        .join("mcp_clients/mcp_clients.db-wal")
        .exists());
}

#[tokio::test]
async fn a_record_the_module_already_wrote_is_not_overwritten() {
    let legacy = legacy_dir();
    let rig = rig();
    let docs = rig.scoped.documents();
    docs.ensure_collection(&CollectionSpec::new(SERVERS))
        .await
        .unwrap();
    docs.put(
        SERVERS,
        "srv-1",
        json!({"server_id": "srv-1", "newer": true}),
        Precondition::Absent,
    )
    .await
    .unwrap();
    rig.secrets
        .set("mcp_client_env/srv-1", b"{\"API_KEY\":\"rotated\"}")
        .await
        .unwrap();

    let outcome = import_into(
        &DispatchCtx::host_default(),
        &rig.scoped,
        &rig.secrets,
        legacy.path(),
    )
    .await
    .unwrap();
    assert_eq!(
        outcome,
        Outcome::Imported {
            servers: 1,
            secrets: 0,
            audit: 2
        }
    );
    let kept = docs.get(SERVERS, "srv-1").await.unwrap().unwrap();
    assert_eq!(kept.doc["newer"], true);
    let secret = rig
        .secrets
        .get("mcp_client_env/srv-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(&secret[..], b"{\"API_KEY\":\"rotated\"}");
}

#[tokio::test]
async fn a_data_dir_with_no_legacy_files_is_a_no_op() {
    let empty = tempfile::tempdir().unwrap();
    let rig = rig();
    assert_eq!(
        import_into(
            &DispatchCtx::host_default(),
            &rig.scoped,
            &rig.secrets,
            empty.path()
        )
        .await
        .unwrap(),
        Outcome::NoSource
    );
}

#[tokio::test]
async fn an_unreadable_file_writes_no_marker_so_the_next_start_retries() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(CLIENTS_DB);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        b"this is not a sqlite database at all, just text padding",
    )
    .unwrap();
    let rig = rig();
    assert!(import_into(
        &DispatchCtx::host_default(),
        &rig.scoped,
        &rig.secrets,
        dir.path()
    )
    .await
    .is_err());
    let done = crate::storage::import_state::host_import_done(
        &rig.scoped,
        MARKER_COLLECTION,
        &marker_id(dir.path()),
    )
    .await
    .unwrap();
    assert!(!done);
}
