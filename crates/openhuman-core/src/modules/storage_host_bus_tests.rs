use super::*;
use serde_json::json;
use tinystoragedrivers_bus::STORAGE_INTERFACE;
use tinystoragedrivers_busproxy::{ERROR_DENIED, OBJECT_ROOT};

#[test]
fn module_scopes_are_distinct_per_module_and_per_tenant() {
    let a = module_scope(None, "alpha").unwrap();
    let b = module_scope(None, "beta").unwrap();
    assert_ne!(a, b);
    assert_eq!(a.as_str(), "local/module:alpha");
    let acme = module_scope(Some("acme"), "alpha").unwrap();
    assert_ne!(a, acme);
    assert_eq!(acme.as_str(), "u:acme/module:alpha");
    let other = module_scope(Some("globex"), "alpha").unwrap();
    assert_ne!(acme, other);
}

#[test]
fn a_profiles_module_scope_sits_under_its_lease_fence() {
    for profile in ["acme", "user-1_x", "h-0123456789abcdef0123456789abcdef"] {
        let scope = module_scope(Some(profile), "tinymcp").unwrap();
        let covered =
            crate::profiles::fence::scopes_of(&crate::profiles::ProfileId::parse(profile).unwrap());
        assert!(
            covered.iter().any(|matcher| match matcher {
                crate::storage::fence::ScopeMatch::Prefix(prefix) =>
                    scope.as_str().starts_with(prefix.as_str()),
                crate::storage::fence::ScopeMatch::Exact(exact) => scope.as_str() == exact,
            }),
            "{profile}: {scope:?}"
        );
    }
}

#[test]
fn an_unusual_module_id_is_hashed_inside_its_tenant() {
    let hashed = module_scope(Some("acme"), "a@b c").unwrap();
    assert!(hashed.as_str().starts_with("u:acme/module:~"), "{hashed}");
    assert_ne!(hashed, module_scope(Some("acme"), "a@b d").unwrap());
    // Hashing keeps the scope under the fence.
    assert_eq!(
        hashed.tenant_scope(),
        Some(crate::storage::scope_for_profile("acme"))
    );
}

#[tokio::test]
async fn a_module_write_under_a_stale_lease_is_fenced_by_the_host() {
    use crate::storage::fence::{FenceRegistry, LeaseFence};
    use crate::storage::fenced_backend::FencedBackend;
    use crate::storage::lease::{DocumentLeases, LeaseStore};
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::Duration;

    let shared: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());
    let clock = Arc::new(AtomicU64::new(0));
    let reading = Arc::clone(&clock);
    let fences = Arc::new(FenceRegistry::new(move || reading.load(Ordering::SeqCst)));
    let ttl = Duration::from_secs(30);
    let leases: Arc<dyn LeaseStore> =
        Arc::new(DocumentLeases::cluster(shared.as_ref(), "node-a", None, ttl).unwrap());
    let grant_a = leases.acquire("acme", 0).await.unwrap();
    fences.register(Arc::new(LeaseFence::new(
        Arc::clone(&leases),
        "acme",
        "node-a",
        grant_a.epoch,
        grant_a.expires_at_ms,
        5_000,
        crate::profiles::fence::scopes_of(&crate::profiles::ProfileId::parse("acme").unwrap()),
    )));
    let fenced: Arc<dyn StorageBackend> = Arc::new(FencedBackend::new(shared, Arc::clone(&fences)));

    let rig = rig();
    *rig.env.backend.lock().unwrap() = Some(fenced);
    *rig.env.fences.lock().unwrap() = Some(fences);
    *rig.env.profile.lock().unwrap() = Some("acme".into());

    call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"v": 1})),
    )
    .await
    .unwrap();

    // The lease runs out; the module's next write is refused by the host's fence,
    // and a read is not.
    clock.store(10 * 60 * 1000, Ordering::SeqCst);
    let result = call(
        &rig,
        &alpha(),
        Object::Documents,
        "Put",
        put("notes", "a", json!({"v": 2})),
    )
    .await;
    assert_eq!(
        result,
        Err(HostError::Storage(StorageFailure {
            kind: StorageKind::Fenced
        }))
    );
    let fenced_audit = rig.trail.0.lock().unwrap().last().cloned().unwrap();
    assert_eq!(fenced_audit.outcome, "fenced");
    assert!(
        call(&rig, &alpha(), Object::Documents, "Get", get("notes", "a"))
            .await
            .is_ok()
    );
}

#[tokio::test]
async fn the_bus_hands_the_host_the_brokers_sender_and_nothing_else() {
    use tinybus::broker::Broker;
    use tinybus::transport::memory::MemoryBus;
    use tinybus::Connection;

    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let module = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let outsider = Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();

    let rig = rig();
    let directory = Directory(HashMap::from([(
        module.unique_name().unwrap().as_str().to_string(),
        "alpha".to_string(),
    )]));
    let host = Arc::new(StorageHost::new(
        Arc::new(directory),
        Arc::clone(&rig.env) as Arc<dyn Environment>,
        Arc::clone(&rig.trail) as Arc<dyn AuditSink>,
    ));
    let _served = serve_objects(&service, &host, None).await.unwrap();

    let documents = |from: &Connection| {
        from.proxy(
            STORAGE_INTERFACE,
            format!("{OBJECT_ROOT}/Documents"),
            STORAGE_INTERFACE,
        )
        .unwrap()
    };

    // The registered module is served...
    let stored: Value = documents(&module)
        .call("Put", put("notes", "a", json!({"v": 1})))
        .await
        .unwrap();
    assert_eq!(stored, json!(1));
    let found: Value = documents(&module)
        .call("Get", get("notes", "a"))
        .await
        .unwrap();
    assert_eq!(found["document_id"], "a");

    // ...a peer that is not a module is refused with the denial name, not data.
    let refused = documents(&outsider)
        .call::<Value>("Get", get("notes", "a"))
        .await
        .unwrap_err();
    match refused {
        tinybus::Error::MethodFailed { name, message } => {
            assert_eq!(name, ERROR_DENIED);
            assert_eq!(message, "unknown-caller");
        }
        other => panic!("unexpected error: {other:?}"),
    }

    // A forged scope is a refused request, not a different scope.
    let mut forged = get("notes", "a");
    forged["scope"] = json!("local");
    assert!(documents(&module)
        .call::<Value>("Get", forged)
        .await
        .is_err());

    // Secrets are on a confidential route: an ordinary call never reaches the host.
    let secrets = module
        .proxy(
            STORAGE_INTERFACE,
            format!("{OBJECT_ROOT}/Secrets"),
            STORAGE_INTERFACE,
        )
        .unwrap();
    assert!(secrets
        .call::<Value>("Get", json!({"name": "token.a"}))
        .await
        .is_err());
    assert!(rig
        .trail
        .0
        .lock()
        .unwrap()
        .iter()
        .all(|record| record.object != "Secrets"));
}
