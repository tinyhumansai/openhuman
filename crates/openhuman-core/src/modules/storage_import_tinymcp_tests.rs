use super::*;
use crate::storage::lease::LeaseStore as _;
use crate::storage::{Scope, Storage};
use std::sync::Arc;
use tinystoragedrivers::secrets::{DerivedKeys, KeyProvider, SecretStore};
use tinystoragedrivers::StorageBackend;
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
    _dir: tempfile::TempDir,
    backend: Arc<dyn StorageBackend>,
    scope: Scope,
}

fn rig() -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite:{}", dir.path().join("storage.sqlite").display());
    let backend =
        crate::storage::block_on(async move { crate::storage::open(&url).await }).unwrap();
    Rig {
        _dir: dir,
        backend,
        scope: Scope::new("module:tinymcp@local").unwrap(),
    }
}

impl Rig {
    fn scoped(&self) -> Storage {
        self.backend.for_scope(&self.scope).unwrap()
    }

    fn secrets(&self) -> impl SecretStore {
        let scoped = self.scoped();
        crate::storage::secrets::over(&scoped, self.scope.clone(), keys())
    }
}

fn keys() -> Arc<dyn KeyProvider> {
    Arc::new(DerivedKeys::new(Zeroizing::new([7; 32])))
}

async fn import(rig: &Rig, data_dir: &Path) -> Result<Outcome> {
    import_into(&rig.backend, &rig.scope, data_dir, Some(keys())).await
}

fn digest(path: &Path) -> Vec<u8> {
    sha2::Sha256::digest(std::fs::read(path).unwrap()).to_vec()
}

#[tokio::test]
async fn servers_env_and_audit_rows_move_into_the_modules_storage() {
    let legacy = legacy_dir();
    let rig = rig();
    let outcome = import(&rig, legacy.path()).await.unwrap();
    assert_eq!(
        outcome,
        Outcome::Imported {
            servers: 2,
            secrets: 1,
            audit: 2
        }
    );

    let scoped = rig.scoped();
    let docs = scoped.documents();
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
        .secrets()
        .get("mcp_client_env/srv-1")
        .await
        .unwrap()
        .expect("env secret");
    let env: std::collections::BTreeMap<String, String> = serde_json::from_slice(&stored).unwrap();
    assert_eq!(env["API_KEY"], "sk-secret");
    assert_eq!(env["REGION"], "eu");
    assert!(rig
        .secrets()
        .get("mcp_client_env/srv-2")
        .await
        .unwrap()
        .is_none());

    let audit = scoped
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
    import(&rig, legacy.path()).await.unwrap();
    let scoped = rig.scoped();
    let docs = scoped.documents();
    let held = docs.get(SERVERS, "srv-1").await.unwrap().unwrap();
    docs.delete(SERVERS, "srv-1", held.unchanged())
        .await
        .unwrap();

    let again = import(&rig, legacy.path()).await.unwrap();
    assert_eq!(again, Outcome::AlreadyDone);
    assert!(docs.get(SERVERS, "srv-1").await.unwrap().is_none());
    assert_eq!(
        scoped.streams().len(AUDIT_STREAM).await.unwrap(),
        2,
        "the audit log is not duplicated"
    );
}

#[tokio::test]
async fn the_legacy_files_are_left_untouched() {
    let legacy = legacy_dir();
    let before = [CLIENTS_DB, AUDIT_DB].map(|path| digest(&legacy.path().join(path)));
    let rig = rig();
    import(&rig, legacy.path()).await.unwrap();
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
    let scoped = rig.scoped();
    let docs = scoped.documents();
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
    rig.secrets()
        .set("mcp_client_env/srv-1", b"{\"API_KEY\":\"rotated\"}")
        .await
        .unwrap();

    let outcome = import(&rig, legacy.path()).await.unwrap();
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
        .secrets()
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
    assert_eq!(import(&rig, empty.path()).await.unwrap(), Outcome::NoSource);
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
    assert!(import(&rig, dir.path()).await.is_err());
    let done = crate::storage::import_state::host_import_done(
        &rig.scoped(),
        MARKER_COLLECTION,
        &marker_id(dir.path()),
    )
    .await
    .unwrap();
    assert!(!done);
}

#[test]
fn a_context_from_before_profile_takeover_cannot_import_under_the_new_lease() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};

    fn profile_context() -> Arc<CoreContext> {
        CoreContext::for_test(DomainSet::full(), None).derive_with(
            ContextOverlay::new(
                crate::config::Config::default(),
                DomainSet::full(),
                Default::default(),
            )
            .profile("profile-1"),
        )
    }

    let old_context = profile_context();
    let old_turn = old_context.derive_with(ContextOverlay::new(
        crate::config::Config::default(),
        DomainSet::full(),
        Default::default(),
    ));
    let reopened_context = profile_context();
    let current_turn = reopened_context.derive_with(ContextOverlay::new(
        crate::config::Config::default(),
        DomainSet::full(),
        Default::default(),
    ));
    assert!(
        super::super::storage_import_support::require_profile_context(
            &reopened_context,
            &current_turn,
            "profile-1"
        )
        .is_ok()
    );
    let error = super::super::storage_import_support::require_profile_context(
        &reopened_context,
        &old_turn,
        "profile-1",
    )
    .unwrap_err();
    assert!(error.to_string().contains("currently admitted context"));
}

/// A host for `node` over the shared `backend`, with its own fence registry,
/// as each node of a cluster has.
fn cluster_node(
    root: &std::path::Path,
    node: &str,
    backend: &Arc<dyn StorageBackend>,
) -> crate::profiles::ProfileHost {
    use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
    let mut saas = SaasConfig::new(root);
    saas.node_id = Some(node.to_string());
    saas.lease_ttl_secs = TAKEOVER_TTL_SECS;
    saas.idle_evict_secs = 3600;
    crate::profiles::ProfileHost::with_backend(
        saas,
        CoreContext::for_test(DomainSet::full(), None),
        Some(Arc::clone(backend)),
    )
    .unwrap()
    .with_fences(Arc::new(crate::storage::fence::FenceRegistry::new(
        crate::profiles::lease::now_ms,
    )))
}

const TAKEOVER_TTL_SECS: u64 = 30;

/// An import running on a dispatch context from before a lease takeover can
/// neither borrow the new holder's fence nor write through its own.
///
/// This used to release and reopen the profile on one host while still holding
/// the old context. `ProfileHost::release` refuses that by contract: the old
/// context keeps the tenant in use (`CoreContext::tenant_in_use`), and a host
/// must not close a tenant while that is true. So a stale context outliving
/// its admission only happens across nodes, when another node takes the lease
/// over, which is what this test models.
#[tokio::test]
async fn an_import_cannot_recapture_a_reopened_profiles_new_fence() {
    use crate::core::runtime::{ContextOverlay, DomainSet};
    use crate::profiles::{ProfileId, ProfileIdMode};
    use crate::storage::lease::DocumentLeases;
    use tinystoragedrivers_types::Precondition;

    let root = tempfile::tempdir().unwrap();
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend: Arc<dyn StorageBackend> = Arc::clone(&fixture.backend);
    let id = ProfileId::for_user("import-takeover", ProfileIdMode::Raw).unwrap();
    let module_scope = crate::storage::scope_for_module(Some(id.as_str()), MODULE_ID).unwrap();
    let turn_on = |context: &Arc<crate::core::runtime::CoreContext>| {
        context.derive_with(ContextOverlay::new(
            crate::config::Config::default(),
            DomainSet::full(),
            Default::default(),
        ))
    };

    // Node A admits the profile and starts a turn on it.
    let node_a = cluster_node(root.path(), "node-a", &backend);
    node_a.provision(&id).await.unwrap();
    let old_profile = node_a.open(&id).await.unwrap();
    let old_turn = turn_on(old_profile.context());

    // Node B takes the lease over (by its clock node A's lease ran out) and
    // opens the profile under its own, newer fence.
    let thief = DocumentLeases::cluster(
        backend.as_ref(),
        "node-b",
        None,
        std::time::Duration::from_secs(TAKEOVER_TTL_SECS),
    )
    .unwrap();
    thief
        .acquire(
            id.as_str(),
            crate::profiles::lease::now_ms() + 2 * TAKEOVER_TTL_SECS * 1_000,
        )
        .await
        .unwrap();
    let node_b = cluster_node(root.path(), "node-b", &backend);
    let current_profile = node_b.open(&id).await.unwrap();
    let current_turn = turn_on(current_profile.context());

    // The old turn cannot borrow node B's fence...
    assert!(
        super::super::storage_import_support::require_profile_context(
            current_profile.context(),
            &old_turn,
            id.as_str(),
        )
        .is_err()
    );
    let stale = super::super::storage_import_support::capture_profile_module_storage(
        &node_b,
        &id,
        id.as_str(),
        &old_turn,
        module_scope.clone(),
    );
    assert!(
        stale.is_err(),
        "old dispatch context cannot borrow the new fence"
    );

    // ...and what it can still capture on node A writes nothing: node A's
    // fence was superseded by the takeover.
    let old_write = async {
        let captured = super::super::storage_import_support::capture_profile_module_storage(
            &node_a,
            &id,
            id.as_str(),
            &old_turn,
            module_scope.clone(),
        )
        .map_err(|_| ())?;
        let storage = captured
            .backend
            .for_scope(&captured.scope)
            .map_err(|_| ())?;
        storage
            .documents()
            .put("notes", "n", serde_json::json!({}), Precondition::None)
            .await
            .map(|_| ())
            .map_err(|_| ())
    }
    .await;
    assert!(
        old_write.is_err(),
        "the superseded holder's import cannot write"
    );

    // The current turn captures node B's fenced storage and can use it.
    let captured = super::super::storage_import_support::capture_profile_module_storage(
        &node_b,
        &id,
        id.as_str(),
        &current_turn,
        module_scope,
    )
    .unwrap();
    captured
        .backend
        .for_scope(&captured.scope)
        .unwrap()
        .documents()
        .put("notes", "n", serde_json::json!({}), Precondition::None)
        .await
        .unwrap();
}

#[tokio::test]
async fn a_captured_import_claim_cannot_write_after_profile_lease_takeover() {
    use crate::core::runtime::{CoreContext, DomainSet, SaasConfig};
    use crate::profiles::{ProfileHost, ProfileId, ProfileIdMode};
    use crate::storage::import_state::{self, backend_with_condition};

    fn host(
        root: &std::path::Path,
        backend: Arc<dyn StorageBackend>,
        node_id: &str,
    ) -> ProfileHost {
        let mut saas = SaasConfig::new(root);
        saas.node_id = Some(node_id.to_owned());
        ProfileHost::with_backend(
            saas,
            CoreContext::for_test(DomainSet::full(), None),
            Some(backend),
        )
        .unwrap()
        .with_fences(Arc::new(crate::storage::fence::FenceRegistry::new(
            crate::profiles::lease::now_ms,
        )))
    }

    let root = tempfile::tempdir().unwrap();
    let fixture = crate::storage::test_support::sqlite_backend().unwrap();
    let backend: Arc<dyn StorageBackend> = Arc::clone(&fixture.backend);
    let id = ProfileId::for_user("import-live-takeover", ProfileIdMode::Raw).unwrap();
    let first = host(root.path(), Arc::clone(&backend), "import-first-node");
    first.provision(&id).await.unwrap();
    let profile_a = first.open(&id).await.unwrap();
    let scope = crate::storage::scope_for_module(Some(id.as_str()), MODULE_ID).unwrap();
    let captured = super::super::storage_import_support::capture_profile_module_storage(
        &first,
        &id,
        id.as_str(),
        profile_a.context(),
        scope.clone(),
    )
    .unwrap();
    let repo = crate::storage::documents::Repo::on(
        &captured.backend,
        &scope,
        "storage_import",
        import_state::collection_specs,
    )
    .unwrap();
    let second = host(root.path(), Arc::clone(&backend), "import-second-node");
    let lease_ttl = first.saas().lease_ttl();
    let lease_store = crate::storage::lease::DocumentLeases::cluster(
        backend.as_ref(),
        "takeover-probe",
        None,
        lease_ttl,
    )
    .unwrap();

    let attempt = import_state::with_claim(
        &repo,
        import_state::HOST_OWNER,
        "legacy-module-import",
        move |condition| {
            let claim_backend =
                backend_with_condition(Arc::clone(&captured.backend), condition.clone());
            let stale_storage = claim_backend
                .for_scope(&scope)
                .map_err(anyhow::Error::from)?;
            crate::storage::block_on_anyhow(async move {
                stale_storage
                    .documents()
                    .ensure_collection(&CollectionSpec::new(SERVERS))
                    .await?;

                let ttl_ms: u64 = lease_ttl.as_millis().try_into().unwrap_or(u64::MAX);
                let takeover_at =
                    crate::profiles::lease::now_ms().saturating_add(ttl_ms.saturating_mul(2));
                let stolen = lease_store.acquire(id.as_str(), takeover_at).await?;
                lease_store.release(stolen).await?;

                // A second host now acquires the same profile on the shared
                // backend. The old captured handle must keep refusing writes.
                let profile_b = second
                    .open(&id)
                    .await
                    .map_err(|error| anyhow::anyhow!("open profile on takeover node: {error}"))?;
                let new_backend = second
                    .storage_backend_for(&profile_b)
                    .map_err(anyhow::Error::from)?
                    .expect("the takeover host uses shared storage");
                let current_storage = new_backend.for_scope(&scope).map_err(anyhow::Error::from)?;

                let stale_write = stale_storage
                    .documents()
                    .put(
                        SERVERS,
                        "after-takeover",
                        json!({"server_id":"after-takeover"}),
                        Precondition::Absent,
                    )
                    .await;
                assert!(
                    stale_write.is_err(),
                    "the captured old epoch must reject payload writes"
                );
                let stale_marker = import_state::host_import_mark(
                    &stale_storage,
                    MARKER_COLLECTION,
                    "after-takeover",
                )
                .await;
                assert!(
                    stale_marker.is_err(),
                    "the old epoch must reject completion writes"
                );

                let current_write = current_storage
                    .documents()
                    .put(
                        SERVERS,
                        "current-node",
                        json!({"server_id":"current-node"}),
                        Precondition::Absent,
                    )
                    .await;
                current_write.expect("the current lease remains writable");
                Ok::<_, anyhow::Error>(())
            })
        },
    );
    let error = attempt.expect_err("the old claim cannot be released after profile takeover");
    assert!(error.to_string().contains("lease fence"), "{error:#}");
}

/// Another process holding the import claim is reported as `ClaimHeld`, so the
/// lifecycle does not open (and serve no legacy servers) on this call.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_foreign_claim_is_reported_as_claim_held() {
    use crate::storage::import_state;
    use std::sync::mpsc;

    let backend = crate::storage::test_support::memory_backend();
    let _installed =
        crate::storage::test_support::install_for_test("memory:mcp-claim", Arc::clone(&backend));
    let ctx = DispatchCtx::host_default();
    let data_dir = tempfile::tempdir().unwrap();
    let clients = data_dir.path().join(CLIENTS_DB);
    std::fs::create_dir_all(clients.parent().unwrap()).unwrap();
    std::fs::write(&clients, b"").unwrap();
    let scope = super::super::storage_host::current_module_scope(&ctx, MODULE_ID).unwrap();
    let repo = crate::storage::documents::Repo::on(
        &backend,
        &scope,
        "storage_import",
        import_state::collection_specs,
    )
    .unwrap();
    let step = import_state::step_id(MARKER_COLLECTION, &marker_id(data_dir.path()));
    let (held_tx, held_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        import_state::with_claim(&repo, import_state::HOST_OWNER, &step, |_| {
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap()
    });
    held_rx.recv().unwrap();

    let imported = crate::modules::import_legacy_mcp(&ctx, data_dir.path()).await;
    release_tx.send(()).unwrap();
    holder.join().unwrap();

    assert!(
        matches!(imported, Err(crate::modules::LegacyImportError::ClaimHeld)),
        "{imported:?}"
    );
}
