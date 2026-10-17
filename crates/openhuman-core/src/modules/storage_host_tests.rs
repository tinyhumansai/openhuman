use super::*;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

use serde_json::json;
use tinystoragedrivers::secrets::DerivedKeys;
use tinystoragedrivers::{CollectionSpec, MemoryStorage};
use zeroize::Zeroizing;

struct Directory(HashMap<String, String>);

#[async_trait]
impl CallerDirectory for Directory {
    async fn module_for(&self, sender: &BusName) -> Option<String> {
        self.0.get(sender.as_str()).cloned()
    }
}

/// A test environment: one backend, a profile that can change, and the grants
/// each module declares.
struct Env {
    backend: Mutex<Option<Arc<dyn StorageBackend>>>,
    profile: Mutex<Option<String>>,
    grants: HashMap<&'static str, StorageGrant>,
    fences: Mutex<Option<Arc<crate::storage::fence::FenceRegistry>>>,
    keys_ready: AtomicBool,
}

#[async_trait]
impl Environment for Env {
    fn backend(&self) -> Option<Arc<dyn StorageBackend>> {
        self.backend.lock().unwrap().clone()
    }

    fn module_scope(&self, module_id: &str) -> Result<Scope, StorageError> {
        let profile = self.profile.lock().unwrap().clone();
        module_scope(profile.as_deref(), module_id)
    }

    fn grant(&self, module_id: &str) -> Option<StorageGrant> {
        self.grants.get(module_id).cloned()
    }

    fn keys(&self) -> Result<Arc<dyn KeyProvider>, StorageError> {
        if !self.keys_ready.load(Ordering::SeqCst) {
            return Err(StorageError::unavailable("key provider unavailable"));
        }
        Ok(Arc::new(DerivedKeys::new(Zeroizing::new([7u8; 32]))))
    }

    async fn admit_write(&self, scope: &Scope) -> Result<(), StorageError> {
        let fences = self.fences.lock().unwrap().clone();
        match fences {
            Some(fences) => fences.guard(scope.as_str(), "test-write").await,
            None => Ok(()),
        }
    }
}

#[derive(Default)]
struct Trail(Mutex<Vec<AuditRecord>>);

impl AuditSink for Trail {
    fn record(&self, record: &AuditRecord) {
        self.0.lock().unwrap().push(record.clone());
    }
}

fn grant(collection: &str) -> StorageGrant {
    StorageGrant {
        collections: vec![CollectionSpec::new(collection)],
        stream_prefixes: vec!["log/".into()],
        blob_prefixes: vec!["files/".into()],
        secret_prefixes: vec!["token.".into()],
        writable: true,
    }
}

fn alpha() -> BusName {
    BusName::new(":1.1").unwrap()
}

fn beta() -> BusName {
    BusName::new(":1.2").unwrap()
}

fn stranger() -> BusName {
    BusName::new(":1.99").unwrap()
}

struct Rig {
    host: StorageHost,
    env: Arc<Env>,
    trail: Arc<Trail>,
    backend: Arc<dyn StorageBackend>,
}

fn rig() -> Rig {
    let backend: Arc<dyn StorageBackend> =
        Arc::new(super::super::storage_host_watching::WatchingStorage::new());
    let env = Arc::new(Env {
        backend: Mutex::new(Some(Arc::clone(&backend))),
        profile: Mutex::new(None),
        grants: HashMap::from([("alpha", grant("notes")), ("beta", grant("other"))]),
        fences: Mutex::new(None),
        keys_ready: AtomicBool::new(true),
    });
    let directory = Directory(HashMap::from([
        (":1.1".into(), "alpha".into()),
        (":1.2".into(), "beta".into()),
    ]));
    let trail = Arc::new(Trail::default());
    Rig {
        host: StorageHost::new(
            Arc::new(directory),
            Arc::clone(&env) as Arc<dyn Environment>,
            Arc::clone(&trail) as Arc<dyn AuditSink>,
        ),
        env,
        trail,
        backend,
    }
}

/// A rig where only `module` (bus name `:1.1`) declares `grant`.
fn rig_with(module: &'static str, grant: StorageGrant) -> Rig {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let env = Arc::new(Env {
        backend: Mutex::new(Some(Arc::clone(&backend))),
        profile: Mutex::new(None),
        grants: HashMap::from([(module, grant)]),
        fences: Mutex::new(None),
        keys_ready: AtomicBool::new(true),
    });
    let trail = Arc::new(Trail::default());
    Rig {
        host: StorageHost::new(
            Arc::new(Directory(HashMap::from([(":1.1".into(), module.into())]))),
            Arc::clone(&env) as Arc<dyn Environment>,
            Arc::clone(&trail) as Arc<dyn AuditSink>,
        ),
        env,
        trail,
        backend,
    }
}

fn put(collection: &str, id: &str, doc: Value) -> Value {
    json!({
        "collection": collection,
        "document_id": id,
        "document_json": doc.to_string(),
        "precondition": {"kind": "none"},
    })
}

fn get(collection: &str, id: &str) -> Value {
    json!({"collection": collection, "document_id": id})
}

async fn call(
    rig: &Rig,
    who: &BusName,
    object: Object,
    selector: &str,
    request: Value,
) -> Result<Value, HostError> {
    rig.host
        .handle(object, selector, request, Some(who), Admission::Private)
        .await
}

fn denied(label: &'static str) -> Result<Value, HostError> {
    Err(HostError::Denied(label))
}

#[tokio::test]
async fn a_module_stores_and_reads_its_own_documents() {
    let rig = rig();
    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"v": 1})),
    )
    .await
    .unwrap();
    let found = call(&rig, &alpha(), Object::Documents, "Get", get("notes", "a"))
        .await
        .unwrap();
    assert_eq!(found["document_id"], "a");
    assert_eq!(found["document_json"], json!({"v": 1}).to_string());
}

#[tokio::test]
async fn a_call_with_no_authenticated_sender_is_refused() {
    let rig = rig();
    let result = rig
        .host
        .handle(
            Object::Documents,
            "Get",
            get("notes", "a"),
            None,
            Admission::Private,
        )
        .await;
    assert_eq!(result, denied("unauthenticated-caller"));
}

#[tokio::test]
async fn a_peer_that_is_not_a_registry_module_is_refused() {
    let rig = rig();
    let result = call(
        &rig,
        &stranger(),
        Object::Documents,
        "Get",
        get("notes", "a"),
    )
    .await;
    assert_eq!(result, denied("unknown-caller"));
}

#[tokio::test]
async fn a_module_without_a_declared_grant_has_no_storage() {
    let rig = rig();
    let directory = Directory(HashMap::from([(":1.5".into(), "undeclared".into())]));
    let host = StorageHost::new(
        Arc::new(directory),
        Arc::clone(&rig.env) as Arc<dyn Environment>,
        Arc::clone(&rig.trail) as Arc<dyn AuditSink>,
    );
    let result = host
        .handle(
            Object::Documents,
            "Get",
            get("notes", "a"),
            Some(&BusName::new(":1.5").unwrap()),
            Admission::Private,
        )
        .await;
    assert_eq!(result, denied("no-grant-declared"));
}

#[tokio::test]
async fn a_module_cannot_name_another_modules_collection() {
    let rig = rig();
    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"v": 1})),
    )
    .await
    .unwrap();
    // beta is granted "other", not "notes".
    for selector in ["Get", "Put", "Delete"] {
        let request = if selector == "Put" {
            put("notes", "a", json!({"v": 2}))
        } else {
            get("notes", "a")
        };
        assert_eq!(
            call(&rig, &beta(), Object::Documents, selector, request).await,
            denied("target-not-granted"),
            "{selector}"
        );
    }
    // And the document is untouched.
    let kept = call(&rig, &alpha(), Object::Documents, "Get", get("notes", "a"))
        .await
        .unwrap();
    assert_eq!(kept["document_json"], json!({"v": 1}).to_string());
}

#[tokio::test]
async fn two_modules_granted_the_same_collection_name_do_not_share_documents() {
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let env = Arc::new(Env {
        backend: Mutex::new(Some(backend)),
        profile: Mutex::new(None),
        grants: HashMap::from([("alpha", grant("notes")), ("beta", grant("notes"))]),
        fences: Mutex::new(None),
        keys_ready: AtomicBool::new(true),
    });
    let directory = Directory(HashMap::from([
        (":1.1".into(), "alpha".into()),
        (":1.2".into(), "beta".into()),
    ]));
    let host = StorageHost::new(
        Arc::new(directory),
        env as Arc<dyn Environment>,
        Arc::new(Trail::default()),
    );
    let ask = |who: BusName, selector: &'static str, request: Value| {
        let host = &host;
        async move {
            host.handle(
                Object::Documents,
                selector,
                request,
                Some(&who),
                Admission::Private,
            )
            .await
        }
    };
    ask(alpha(), "Put", put("notes", "a", json!({"owner": "alpha"})))
        .await
        .unwrap();
    assert_eq!(
        ask(beta(), "Get", get("notes", "a")).await.unwrap(),
        Value::Null,
        "beta must not see alpha's document"
    );
    ask(beta(), "Put", put("notes", "a", json!({"owner": "beta"})))
        .await
        .unwrap();
    let alphas = ask(alpha(), "Get", get("notes", "a")).await.unwrap();
    assert_eq!(
        alphas["document_json"],
        json!({"owner": "alpha"}).to_string()
    );
}

#[tokio::test]
async fn a_module_in_another_scope_does_not_see_the_data() {
    let rig = rig();
    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"v": 1})),
    )
    .await
    .unwrap();
    // The same module, acting for a different tenant, is a different scope.
    *rig.env.profile.lock().unwrap() = Some("acme".into());
    let other = call(&rig, &alpha(), Object::Documents, "Get", get("notes", "a"))
        .await
        .unwrap();
    assert_eq!(other, Value::Null);
    *rig.env.profile.lock().unwrap() = None;
    let back = call(&rig, &alpha(), Object::Documents, "Get", get("notes", "a"))
        .await
        .unwrap();
    assert_eq!(back["document_id"], "a");
}

#[tokio::test]
async fn a_request_cannot_carry_a_scope_a_database_or_a_grant() {
    let rig = rig();
    for forged in ["scope", "database", "grant", "generation"] {
        let mut request = get("notes", "a");
        request[forged] = json!("profile:acme");
        let result = call(&rig, &alpha(), Object::Documents, "Get", request).await;
        assert_eq!(
            result,
            Err(HostError::Storage(StorageFailure {
                kind: StorageKind::InvalidInput
            })),
            "{forged}"
        );
    }
}

#[tokio::test]
async fn the_host_ensures_declared_schema_and_the_module_cannot_change_it() {
    // The grant declares a unique index; only the host's ensure_collection can
    // have created it, so a duplicate being refused proves the host ran it.
    let spec = CollectionSpec::new("notes")
        .index(tinystoragedrivers::IndexSpec::new("by_key", ["key"]).unique());
    let mut declared = grant("notes");
    declared.collections = vec![spec.clone()];
    let rig = rig_with("alpha", declared);
    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"key": "same"})),
    )
    .await
    .unwrap();
    let duplicate = call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "b", json!({"key": "same"})),
    )
    .await;
    assert!(
        matches!(duplicate, Err(HostError::Storage(_))),
        "the declared unique index must hold: {duplicate:?}"
    );

    // Repeating the declared spec is answered; changing it is the host's call.
    let same = serde_json::to_value(&spec).unwrap();
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Documents,
            "EnsureCollection",
            json!({"spec": same})
        )
        .await,
        Ok(Value::Null)
    );
    let wider = serde_json::to_value(spec.clone().ttl("t")).unwrap();
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Documents,
            "EnsureCollection",
            json!({"spec": wider})
        )
        .await,
        denied("schema-is-host-only")
    );
    let weaker = serde_json::to_value(CollectionSpec::new("notes")).unwrap();
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Documents,
            "EnsureCollection",
            json!({"spec": weaker})
        )
        .await,
        denied("schema-is-host-only")
    );
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Documents,
            "DropCollection",
            json!({"collection": "notes"})
        )
        .await,
        denied("schema-is-host-only")
    );
}

#[tokio::test]
async fn a_collection_listing_shows_only_the_modules_own_names() {
    let rig = rig();
    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({})),
    )
    .await
    .unwrap();
    // The host keeps something else in the module's scope.
    let scope = module_scope(None, "alpha").unwrap();
    let storage = rig.backend.for_scope(&scope).unwrap();
    storage
        .documents()
        .put(
            "host-private",
            "x",
            json!({}),
            crate::storage::Precondition::None,
        )
        .await
        .unwrap();
    let all = storage.documents().collections("").await.unwrap();
    assert!(all.contains(&"host-private".to_string()), "{all:?}");

    let listed = call(
        &rig,
        &alpha(),
        Object::Documents,
        "Collections",
        json!({"prefix": ""}),
    )
    .await
    .unwrap();
    assert_eq!(listed, json!(["notes"]));
}

#[tokio::test]
async fn streams_and_blobs_stay_inside_their_prefixes() {
    let rig = rig();
    call(
        &rig,
        &alpha(),
        Object::Streams,
        "Append",
        json!({"stream": "log/a", "record_json": json!({"n": 1}).to_string()}),
    )
    .await
    .unwrap();
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Streams,
            "Append",
            json!({"stream": "audit/a", "record_json": "{}"}),
        )
        .await,
        denied("target-not-granted")
    );
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Blobs,
            "Head",
            json!({"key": "elsewhere/x"})
        )
        .await,
        denied("target-not-granted")
    );
    assert_eq!(
        call(
            &rig,
            &alpha(),
            Object::Blobs,
            "PutStream",
            json!({"key": "elsewhere/x", "len": 1})
        )
        .await,
        denied("target-not-granted")
    );
}

#[tokio::test]
async fn secrets_need_a_confidential_caller_and_a_granted_prefix() {
    let rig = rig();
    let set = json!({"name": "token.a", "value": [1, 2, 3]});
    // The bus gates the route; the host still refuses a private admission.
    let private = call(&rig, &alpha(), Object::Secrets, "Set", set.clone()).await;
    assert_eq!(
        private,
        Err(HostError::Storage(StorageFailure {
            kind: StorageKind::Unsupported
        }))
    );
    let confidential = |request: Value, selector: &'static str, who: BusName| {
        let host = &rig.host;
        async move {
            host.handle(
                Object::Secrets,
                selector,
                request,
                Some(&who),
                Admission::Confidential,
            )
            .await
        }
    };
    confidential(set, "Set", alpha()).await.unwrap();
    let read = confidential(json!({"name": "token.a"}), "Get", alpha())
        .await
        .unwrap();
    assert_eq!(read["value"], json!([1, 2, 3]));
    assert_eq!(
        confidential(json!({"name": "other.a"}), "Get", alpha()).await,
        denied("target-not-granted")
    );
    // beta's secrets are a different scope even under the same name.
    assert_eq!(
        confidential(json!({"name": "token.a"}), "Get", beta())
            .await
            .unwrap(),
        Value::Null
    );
}

#[tokio::test]
async fn a_read_only_grant_cannot_write() {
    let rig = rig();
    let mut read_only = grant("notes");
    read_only.writable = false;
    let env = Arc::new(Env {
        backend: Mutex::new(Some(Arc::clone(&rig.backend))),
        profile: Mutex::new(None),
        grants: HashMap::from([("alpha", read_only)]),
        fences: Mutex::new(None),
        keys_ready: AtomicBool::new(true),
    });
    let host = StorageHost::new(
        Arc::new(Directory(HashMap::from([(":1.1".into(), "alpha".into())]))),
        env as Arc<dyn Environment>,
        Arc::new(Trail::default()),
    );
    let result = host
        .handle(
            Object::Documents,
            "Put",
            put("notes", "a", json!({})),
            Some(&alpha()),
            Admission::Private,
        )
        .await;
    assert_eq!(result, denied("read-only-grant"));
}

#[tokio::test]
async fn an_invalid_grant_is_never_bound() {
    let rig = rig();
    let mut bad = grant("notes");
    bad.collections = vec![CollectionSpec::new("secrets")];
    let env = Arc::new(Env {
        backend: Mutex::new(Some(Arc::clone(&rig.backend))),
        profile: Mutex::new(None),
        grants: HashMap::from([("alpha", bad)]),
        fences: Mutex::new(None),
        keys_ready: AtomicBool::new(true),
    });
    let host = StorageHost::new(
        Arc::new(Directory(HashMap::from([(":1.1".into(), "alpha".into())]))),
        env as Arc<dyn Environment>,
        Arc::new(Trail::default()),
    );
    let result = host
        .handle(
            Object::Documents,
            "Get",
            get("secrets", "a"),
            Some(&alpha()),
            Admission::Private,
        )
        .await;
    assert_eq!(result, denied("invalid-grant"));
}

#[tokio::test]
async fn no_installed_backend_answers_unavailable() {
    let rig = rig();
    *rig.env.backend.lock().unwrap() = None;
    let result = call(&rig, &alpha(), Object::Documents, "Get", get("notes", "a")).await;
    assert_eq!(
        result,
        Err(HostError::Storage(StorageFailure {
            kind: StorageKind::Unavailable
        }))
    );
}

#[tokio::test]
async fn every_call_is_audited_without_its_payload() {
    let rig = rig();
    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"secret-looking": "PAYLOAD-MARKER"})),
    )
    .await
    .unwrap();
    let _ = call(&rig, &beta(), Object::Documents, "Get", get("notes", "a")).await;
    let _ = call(
        &rig,
        &alpha(),
        Object::Documents,
        "DropCollection",
        json!({"collection": "notes"}),
    )
    .await;

    let trail = rig.trail.0.lock().unwrap().clone();
    assert_eq!(trail.len(), 3);
    assert_eq!(
        trail[0],
        AuditRecord {
            module: "alpha".into(),
            scope: "local/module:alpha".into(),
            object: "Documents",
            op: "Put".into(),
            target: "notes".into(),
            outcome: "ok",
        }
    );
    assert_eq!(trail[1].module, "beta");
    assert_eq!(trail[1].outcome, "target-not-granted");
    assert_eq!(trail[2].outcome, "schema-is-host-only");
    let rendered = format!("{trail:?}");
    assert!(!rendered.contains("PAYLOAD-MARKER"));
}

#[path = "storage_host_bus_tests.rs"]
mod bus;

#[path = "storage_host_cdylib_tests.rs"]
mod cdylib;
#[cfg(test)]
#[path = "storage_host_stream_tests.rs"]
mod streams;

async fn set_secret(rig: &Rig) -> Result<Value, HostError> {
    let who = alpha();
    rig.host
        .handle(
            Object::Secrets,
            "Set",
            json!({"name": "token.a", "value": [1]}),
            Some(&who),
            Admission::Confidential,
        )
        .await
}

#[tokio::test]
async fn a_key_provider_failure_at_bind_is_retried_not_cached() {
    let rig = rig();
    rig.env.keys_ready.store(false, Ordering::SeqCst);
    assert!(set_secret(&rig).await.is_err());
    rig.env.keys_ready.store(true, Ordering::SeqCst);
    set_secret(&rig).await.unwrap();
}
