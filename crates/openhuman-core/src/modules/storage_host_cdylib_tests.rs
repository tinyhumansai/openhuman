//! The storage host against a REAL loaded module (#7772).
//!
//! `tests/fixtures/storage_probe_module` is a cdylib that relays one raw call
//! to `ai.tinyhumans.tinystorage.Storage` and reports the answer. Here it is
//! built, hash-pinned in a `modules.toml` beside it and loaded through
//! `ModuleHost::load_file`, so every call below crosses the real ABI, the real
//! broker and the real `authenticated_sender` rule of a loaded module. The
//! host double only decides which registry id the probe's peer stands for
//! (`tinymcp` or `tinyjuice`, so the compiled grants apply).
//!
//! What this proves, per the issue:
//! * a module cannot reach another module's collections, another scope, or a
//!   collection its grant never declared;
//! * a blob over the inline cap (1 MiB) is refused and stores nothing;
//! * a blob over the 16 MiB bus frame round-trips by `PutStream`/`GetStream`
//!   on a peer-bound `tinybus.Stream`, and a body no frame could carry is a
//!   clear error instead of a hung call;
//! * a watch invalidation reaches the module that opened the watch, and no
//!   other module sees it;
//! * a second module can neither read the first module's blobs nor take the
//!   stream the first opened.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};
use tinybus::broker::Broker;
use tinybus::module::ModuleHost;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, Proxy};
use tinystoragedrivers::secrets::{DerivedKeys, KeyProvider};
use zeroize::Zeroizing;

use crate::modules::storage_grants::{declared, StorageGrant};
use crate::modules::storage_host::{
    module_scope, AuditRecord, AuditSink, CallerDirectory, Environment, StorageHost,
};
use crate::modules::storage_host_linked::attach;
use crate::storage::{Scope, StorageBackend, StorageError};
use async_trait::async_trait;
use std::sync::Arc;
use tinybus::BusName;
use tinystoragedrivers_bus::STORAGE_INTERFACE;

const PROBE: &str = "ai.tinyhumans.test.StorageProbe";

use crate::modules::storage_host_linked::TEST_TURN as TURN;

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repository root")
}

/// The built probe library: `OPENHUMAN_STORAGE_PROBE_MODULE` when set, else a
/// build of the fixture under the checkout's own `target/storage-probe`.
fn probe_artifact() -> &'static Path {
    static BUILT: OnceLock<PathBuf> = OnceLock::new();
    BUILT.get_or_init(|| {
        if let Some(path) = std::env::var_os("OPENHUMAN_STORAGE_PROBE_MODULE") {
            return PathBuf::from(path);
        }
        let root = repo_root();
        let target = root.join("target/storage-probe");
        let status =
            std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
                .args(["build", "--quiet", "--manifest-path"])
                .arg(root.join("tests/fixtures/storage_probe_module/Cargo.toml"))
                .arg("--target-dir")
                .arg(&target)
                // The test run's own flags (coverage instrumentation, a linker choice)
                // must not leak into a fixture that is a different build.
                .env_remove("RUSTFLAGS")
                .env_remove("CARGO_ENCODED_RUSTFLAGS")
                .env_remove("RUSTC_WRAPPER")
                .env_remove("CARGO_TARGET_DIR")
                .status()
                .expect("run cargo to build the storage probe module");
        assert!(status.success(), "building the storage probe module failed");
        target.join("debug").join(format!(
            "{}storage_probe_module{}",
            std::env::consts::DLL_PREFIX,
            std::env::consts::DLL_SUFFIX
        ))
    })
}

/// Stages the probe beside a `modules.toml` pinning its digest, which is what
/// makes a loaded module an attested recipient.
fn stage(directory: &Path) -> PathBuf {
    let artifact = probe_artifact();
    let name = artifact.file_name().expect("artifact name");
    let staged = directory.join(name);
    std::fs::copy(artifact, &staged).expect("stage the probe");
    let hash = tinybus::module::sha256_file(&staged).expect("digest");
    std::fs::write(
        directory.join("modules.toml"),
        format!("{:?} = {hash:?}\n", name.to_str().expect("utf-8 name")),
    )
    .expect("write modules.toml");
    staged
}

/// Names each listed peer after the registry module it stands for, and nobody
/// else: the probe, and any extra peers a test adds.
struct ProbeDirectory {
    connection: Connection,
    peers: Vec<(&'static str, &'static str)>,
}

#[async_trait]
impl CallerDirectory for ProbeDirectory {
    async fn module_for(&self, sender: &BusName) -> Option<String> {
        for (name, module_id) in &self.peers {
            if let Ok(Some(owner)) = self.connection.name_owner(*name).await {
                if &owner == sender {
                    return Some((*module_id).to_string());
                }
            }
        }
        None
    }
}

struct Env(Arc<dyn StorageBackend>);

#[async_trait]
impl Environment for Env {
    fn backend(&self) -> Option<Arc<dyn StorageBackend>> {
        Some(Arc::clone(&self.0))
    }
    fn module_scope(&self, module_id: &str) -> Result<Scope, StorageError> {
        module_scope(None, module_id)
    }
    fn grant(&self, module_id: &str) -> Option<StorageGrant> {
        declared(module_id)
    }
    fn keys(&self) -> Result<Arc<dyn KeyProvider>, StorageError> {
        Ok(Arc::new(DerivedKeys::new(Zeroizing::new([9u8; 32]))))
    }
    async fn admit_write(&self, _scope: &Scope) -> Result<(), StorageError> {
        Ok(())
    }
}

#[derive(Default)]
struct Trail(Mutex<Vec<AuditRecord>>);

impl AuditSink for Trail {
    fn record(&self, record: &AuditRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}

struct Rig {
    backend: Arc<dyn StorageBackend>,
    trail: Arc<Trail>,
    /// A peer that is not the probe: the host must not know it.
    stranger: Connection,
    /// Extra peers standing for registry modules, with the host-side bus name
    /// of the storage interface's owner they talk to.
    peers: Vec<Connection>,
    host: BusName,
    probe: Proxy,
    _modules: ModuleHost,
    _staged: tempfile::TempDir,
}

/// The real probe, loaded, standing for registry module `module_id`.
async fn rig(module_id: &'static str) -> Rig {
    rig_with_peers(module_id, &[]).await
}

/// The bus name of the `n`th extra peer.
fn peer_name(n: usize) -> String {
    format!("ai.tinyhumans.test.Peer{n}")
}

/// [`rig`], plus one extra in-process peer per entry of `others`, each
/// standing for that registry module under the bus name [`peer_name`].
async fn rig_with_peers(module_id: &'static str, others: &[&'static str]) -> Rig {
    let transport = MemoryBus::new();
    let broker = Broker::new();
    broker.spawn(transport.clone());
    let modules = ModuleHost::new(broker);
    let host_connection = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let stranger = Connection::connect(transport.connect().await.unwrap())
        .await
        .unwrap();
    let backend: Arc<dyn StorageBackend> =
        Arc::new(crate::modules::storage_host_watching::WatchingStorage::new());
    let trail = Arc::new(Trail::default());
    let mut peers = Vec::new();
    let mut named = vec![(PROBE, module_id)];
    for (n, other) in others.iter().enumerate() {
        let connection = Connection::connect(transport.connect().await.unwrap())
            .await
            .unwrap();
        let name: &'static str = Box::leak(peer_name(n).into_boxed_str());
        connection.request_name(name).await.unwrap();
        named.push((name, *other));
        peers.push(connection);
    }
    let host = Arc::new(StorageHost::new(
        Arc::new(ProbeDirectory {
            connection: host_connection.clone(),
            peers: named,
        }),
        Arc::new(Env(Arc::clone(&backend))),
        trail.clone(),
    ));
    attach(&modules, &host_connection, host).await.unwrap();
    // TinyBus rejects module roots beneath symlinked ancestors. macOS exposes
    // `/tmp` through `/var`, so stage under its canonical path.
    let temp_root = std::env::temp_dir()
        .canonicalize()
        .expect("canonical temporary directory");
    let staged = tempfile::tempdir_in(temp_root).unwrap();
    let artifact = stage(staged.path());
    modules.load_file(&artifact).expect("load the probe cdylib");
    // `load_file` can return before the module's setup claims its name.
    let probe_name = BusName::new(PROBE).expect("probe bus name");
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while !matches!(stranger.name_owner(&probe_name).await, Ok(Some(_))) {
        assert!(
            std::time::Instant::now() < deadline,
            "the probe module never claimed {PROBE}"
        );
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
    let probe = stranger.proxy(PROBE, "/probe", PROBE).unwrap();
    let host = stranger
        .name_owner(STORAGE_INTERFACE)
        .await
        .unwrap()
        .expect("the storage host owns its name");
    Rig {
        backend,
        trail,
        stranger,
        peers,
        host,
        probe,
        _modules: modules,
        _staged: staged,
    }
}

impl Rig {
    /// One raw storage call made by the loaded module.
    async fn call(&self, object: &str, selector: &str, request: Value) -> Value {
        self.probe
            .call("Call", (object, selector, request, false))
            .await
            .expect("the probe answers")
    }

    fn scope(&self, module_id: &str) -> Scope {
        module_scope(None, module_id).unwrap()
    }
}

fn denied(answer: &Value, label: &str) -> bool {
    answer["err"]["name"] == "ai.tinyhumans.tinystorage.Error.Denied"
        && answer["err"]["message"] == label
}

fn put(collection: &str, id: &str, document: &Value) -> Value {
    json!({
        "collection": collection,
        "document_id": id,
        "document_json": document.to_string(),
        "precondition": {"kind": "none"},
    })
}

fn get(collection: &str, id: &str) -> Value {
    json!({"collection": collection, "document_id": id})
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loaded_module_reads_and_writes_its_own_granted_collection() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    let stored = rig
        .call(
            "Documents",
            "Put",
            put("mcp.servers", "files", &json!({"qualified_name": "files"})),
        )
        .await;
    assert!(stored.get("ok").is_some(), "{stored}");
    let read = rig
        .call("Documents", "Get", get("mcp.servers", "files"))
        .await;
    assert!(read.get("ok").is_some(), "{read}");
    assert!(read.to_string().contains("qualified_name"), "{read}");
    // It landed in the module's own scope, in the backend the host opened.
    let own = rig.backend.for_scope(&rig.scope("tinymcp")).unwrap();
    assert!(own
        .documents()
        .get("mcp.servers", "files")
        .await
        .unwrap()
        .is_some());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loaded_module_cannot_reach_another_modules_collections() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    // Another module's collection, seeded in that module's own scope.
    let theirs = crate::modules::storage_grants::declared("tinychannels")
        .expect("tinychannels declares storage")
        .collections[0]
        .name
        .clone();
    let victim = rig.backend.for_scope(&rig.scope("tinychannels")).unwrap();
    victim
        .documents()
        .ensure_collection(
            &declared("tinychannels")
                .unwrap()
                .collections
                .into_iter()
                .next()
                .unwrap(),
        )
        .await
        .unwrap();
    victim
        .documents()
        .put(
            &theirs,
            "session",
            json!({"marker": "OTHER-MODULES-SECRET"}),
            Default::default(),
        )
        .await
        .unwrap();

    for (selector, request) in [
        ("Get", get(&theirs, "session")),
        (
            "Put",
            put(&theirs, "session", &json!({"marker": "overwritten"})),
        ),
        (
            "Query",
            json!({"collection": theirs, "filter": null, "limit": 10}),
        ),
        (
            "AtomicBatch",
            json!({"ops": [{"kind": "delete", "collection": theirs, "document_id": "session"}]}),
        ),
    ] {
        let answer = rig.call("Documents", selector, request).await;
        assert!(
            denied(&answer, "target-not-granted"),
            "{selector}: {answer}"
        );
        assert!(!answer.to_string().contains("OTHER-MODULES-SECRET"));
    }
    // Untouched: the refusal came before the backend.
    let after = victim.documents().get(&theirs, "session").await.unwrap();
    assert!(after
        .expect("the other module's document survives")
        .doc
        .to_string()
        .contains("OTHER-MODULES-SECRET"));
    // And its own scope never got the collection.
    let own = rig.backend.for_scope(&rig.scope("tinymcp")).unwrap();
    assert!(!own
        .documents()
        .collections("")
        .await
        .unwrap()
        .contains(&theirs));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loaded_module_cannot_name_or_reach_another_scope() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    // The same collection and id in the host's default scope and in another
    // tenant's: neither may be visible to the module.
    for scope in [Scope::local(), Scope::new("someone-else").unwrap()] {
        let handle = rig.backend.for_scope(&scope).unwrap();
        let spec = declared("tinymcp")
            .unwrap()
            .collections
            .into_iter()
            .next()
            .unwrap();
        handle.documents().ensure_collection(&spec).await.unwrap();
        handle
            .documents()
            .put(
                "mcp.servers",
                "victim",
                json!({"marker": "OTHER-SCOPE-SECRET"}),
                Default::default(),
            )
            .await
            .unwrap();
    }
    // Same id, own scope: not found, not the other scope's document.
    let read = rig
        .call("Documents", "Get", get("mcp.servers", "victim"))
        .await;
    assert!(read["ok"].is_null(), "{read}");
    assert!(!read.to_string().contains("OTHER-SCOPE-SECRET"), "{read}");
    // A scope smuggled into the request is not a selector the wire knows.
    for field in ["scope", "agent", "module", "profile"] {
        let mut request = get("mcp.servers", "victim");
        request[field] = json!("local");
        let answer = rig.call("Documents", "Get", request).await;
        assert!(answer.get("err").is_some(), "{field}: {answer}");
        assert!(!answer.to_string().contains("OTHER-SCOPE-SECRET"));
    }
    // A write lands only in the module's own scope.
    let wrote = rig
        .call(
            "Documents",
            "Put",
            put("mcp.servers", "mine", &json!({"qualified_name": "mine"})),
        )
        .await;
    assert!(wrote.get("ok").is_some(), "{wrote}");
    for scope in [Scope::local(), Scope::new("someone-else").unwrap()] {
        let handle = rig.backend.for_scope(&scope).unwrap();
        assert!(handle
            .documents()
            .get("mcp.servers", "mine")
            .await
            .unwrap()
            .is_none());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_loaded_module_cannot_use_a_collection_it_never_declared() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    for selector in ["Get", "Query", "Count"] {
        let request = match selector {
            "Get" => get("never_declared", "x"),
            _ => json!({"collection": "never_declared"}),
        };
        let answer = rig.call("Documents", selector, request).await;
        assert!(
            denied(&answer, "target-not-granted"),
            "{selector}: {answer}"
        );
    }
    let wrote = rig
        .call(
            "Documents",
            "Put",
            put("never_declared", "x", &json!({"a": 1})),
        )
        .await;
    assert!(denied(&wrote, "target-not-granted"), "{wrote}");
    // Schema is the host's alone: a module cannot create the collection, nor
    // change a declared one, nor drop one.
    let created = rig
        .call(
            "Documents",
            "EnsureCollection",
            json!({"spec": {"name": "never_declared"}}),
        )
        .await;
    assert!(denied(&created, "schema-is-host-only"), "{created}");
    let dropped = rig
        .call(
            "Documents",
            "DropCollection",
            json!({"collection": "mcp.servers"}),
        )
        .await;
    assert!(denied(&dropped, "schema-is-host-only"), "{dropped}");
    let own = rig.backend.for_scope(&rig.scope("tinymcp")).unwrap();
    let listed = own.documents().collections("").await.unwrap();
    assert!(
        !listed.contains(&"never_declared".to_string()),
        "{listed:?}"
    );
    // The module's listing shows its own names only.
    let seen = rig.call("Documents", "Collections", json!({})).await;
    assert!(!seen.to_string().contains("never_declared"), "{seen}");
}

#[tokio::test(flavor = "multi_thread")]
async fn an_undeclared_object_and_prefix_are_refused() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    // tinymcp declares no blobs at all.
    let blob = rig
        .call("Blobs", "Get", json!({"key": "anything/at/all"}))
        .await;
    assert!(denied(&blob, "object-not-granted"), "{blob}");
    // A stream outside its audit prefix.
    let stream = rig
        .call("Streams", "Len", json!({"stream": "someone_elses_stream"}))
        .await;
    assert!(denied(&stream, "target-not-granted"), "{stream}");
    // A secret outside its prefixes, on the confidential route.
    let secret: Value = rig
        .probe
        .call(
            "Call",
            ("Secrets", "Get", json!({"name": "someone_elses/key"}), true),
        )
        .await
        .unwrap();
    assert!(denied(&secret, "target-not-granted"), "{secret}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_is_not_a_registry_module_is_refused() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    let proxy = rig
        .stranger
        .proxy(
            STORAGE_INTERFACE,
            "/ai/tinyhumans/tinystorage/Documents",
            STORAGE_INTERFACE,
        )
        .unwrap();
    let refused: tinybus::Result<Value> = proxy.call("Get", (get("mcp.servers", "files"),)).await;
    match refused {
        Err(tinybus::Error::MethodFailed { name, message }) => {
            assert_eq!(name, "ai.tinyhumans.tinystorage.Error.Denied");
            assert_eq!(message, "unknown-caller");
        }
        other => panic!("expected a refusal, got {other:?}"),
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_blob_over_the_inline_cap_is_refused_and_stores_nothing() {
    let _turn = TURN.lock().await;
    let rig = rig("tinyjuice").await;
    let small: Value = rig
        .probe
        .call("PutBlob", ("juice/ccr/small", 1024usize))
        .await
        .unwrap();
    assert!(
        small.get("ok").is_some(),
        "a small inline blob works: {small}"
    );
    // Past the contract's inline cap (1 MiB) the server refuses the body and
    // stores nothing; a body that size or bigger goes by `PutStream`, which
    // the tests below cover. (A body past the 16 MiB frame is covered by
    // `an_inline_blob_no_frame_can_carry_fails_fast_instead_of_hanging`.)
    let big: Value = rig
        .probe
        .call("PutBlob", ("juice/ccr/over-cap", 2usize * 1024 * 1024))
        .await
        .unwrap();
    assert!(
        big.get("err").is_some(),
        "an oversized inline blob must fail: {big}"
    );
    let handle = rig.backend.for_scope(&rig.scope("tinyjuice")).unwrap();
    assert!(handle
        .blobs()
        .head("juice/ccr/over-cap")
        .await
        .unwrap()
        .is_none());
    // The refusal does not take the module's connection down with it.
    let after: Value = rig
        .probe
        .call("PutBlob", ("juice/ccr/after", 2048usize))
        .await
        .unwrap();
    assert!(
        after.get("ok").is_some(),
        "the module keeps working: {after}"
    );
    // A blob key outside the CCR prefix is not the module's.
    let outside = rig
        .call("Blobs", "Get", json!({"key": "juice/other/x"}))
        .await;
    assert!(denied(&outside, "target-not-granted"), "{outside}");
}

#[tokio::test(flavor = "multi_thread")]
async fn every_call_is_audited_under_the_module_id_without_a_payload() {
    let _turn = TURN.lock().await;
    let rig = rig("tinymcp").await;
    let _ = rig
        .call(
            "Documents",
            "Put",
            put(
                "mcp.servers",
                "audited",
                &json!({"marker": "PAYLOAD-MARKER"}),
            ),
        )
        .await;
    let _ = rig
        .call("Documents", "Get", get("never_declared", "x"))
        .await;
    let trail = rig.trail.0.lock().unwrap().clone();
    assert!(trail.iter().all(|record| record.module == "tinymcp"));
    assert!(trail
        .iter()
        .any(|record| record.op == "Put" && record.outcome == "ok"));
    assert!(trail
        .iter()
        .any(|record| record.outcome == "target-not-granted"));
    assert!(!format!("{trail:?}").contains("PAYLOAD-MARKER"));
}

#[path = "storage_host_cdylib_stream_tests.rs"]
mod streams;
