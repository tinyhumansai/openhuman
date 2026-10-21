use super::*;
use std::sync::{Arc, Mutex};
use tinybus::service::Interface;
use tinybus::transport::memory::MemoryBus;

const DESTINATION: &str = "ai.tinyhumans.openhuman.DoctorConnectorFixture";
const OBJECT_PATH: &str = "/ai/tinyhumans/openhuman/DoctorConnectorFixture";
const INTERFACE: &str = "ai.tinyhumans.openhuman.DoctorConnectorFixture";

struct ReadOnlyFixture {
    calls: Arc<Mutex<Vec<(String, serde_json::Value)>>>,
    reply: serde_json::Value,
    delay: std::time::Duration,
}

struct FixtureReadyRuntime {
    proxy: tinybus::Proxy,
    proxy_requests: Arc<std::sync::atomic::AtomicUsize>,
}

impl ExistingRuntimeAccess for FixtureReadyRuntime {
    fn is_ready(&self) -> bool {
        true
    }

    fn proxy(&self) -> Result<tinybus::Proxy, ExistingCallError> {
        self.proxy_requests
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(self.proxy.clone())
    }
}

#[async_trait::async_trait]
impl Interface for ReadOnlyFixture {
    fn name(&self) -> tinybus::InterfaceName {
        tinybus::InterfaceName::new(INTERFACE).unwrap()
    }

    fn members(&self) -> Vec<tinybus::MemberName> {
        vec![tinybus::MemberName::new("ListConnectedIntegrations").unwrap()]
    }

    async fn call(
        &self,
        member: &tinybus::MemberName,
        args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        tokio::time::sleep(self.delay).await;
        self.calls
            .lock()
            .unwrap()
            .push((member.as_str().to_owned(), args));
        Ok(self.reply.clone())
    }
}

struct LinkedFixtureModule {
    tx: std::sync::mpsc::SyncSender<Vec<u8>>,
}

unsafe extern "C" fn linked_fixture_deliver(
    ctx: *mut std::ffi::c_void,
    ptr: *const u8,
    len: usize,
) -> i32 {
    if ctx.is_null() || ptr.is_null() {
        return tinybus::module::abi::TB_BAD_ARGUMENT;
    }
    let module = unsafe { &*ctx.cast::<LinkedFixtureModule>() };
    let bytes = unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec();
    match module.tx.try_send(bytes) {
        Ok(()) => tinybus::module::abi::TB_OK,
        Err(std::sync::mpsc::TrySendError::Full(_)) => tinybus::module::abi::TB_BACKPRESSURE,
        Err(std::sync::mpsc::TrySendError::Disconnected(_)) => tinybus::module::abi::TB_CLOSED,
    }
}

unsafe extern "C" fn linked_fixture_shutdown(ctx: *mut std::ffi::c_void, _: u64) -> i32 {
    if !ctx.is_null() {
        drop(unsafe { Box::from_raw(ctx.cast::<LinkedFixtureModule>()) });
    }
    tinybus::module::abi::TB_OK
}

unsafe extern "C" fn linked_fixture_reinitialize(
    _: *mut std::ffi::c_void,
    _: *const u8,
    _: usize,
) -> i32 {
    tinybus::module::abi::TB_BAD_ARGUMENT
}

unsafe extern "C" fn linked_fixture_init(
    host: *const tinybus::module::abi::TbHostVtable,
    out: *mut tinybus::module::abi::TbModuleVtable,
) -> i32 {
    if host.is_null() || out.is_null() {
        return tinybus::module::abi::TB_BAD_ARGUMENT;
    }
    let host = unsafe { *host };
    let config = if host.config.len == 0 {
        serde_json::Value::Null
    } else {
        let bytes = unsafe { std::slice::from_raw_parts(host.config.ptr, host.config.len) };
        match serde_json::from_slice::<serde_json::Value>(bytes) {
            Ok(config) => config,
            Err(_) => return tinybus::module::abi::TB_BAD_ARGUMENT,
        }
    };
    let reply = config
        .get("reply")
        .cloned()
        .unwrap_or(serde_json::Value::Null);
    let host_ctx = host.host_ctx as usize;
    let send = host.send;
    let ready = host.ready;
    let (tx, rx) = std::sync::mpsc::sync_channel::<Vec<u8>>(8);
    std::thread::spawn(move || {
        unsafe { ready(host_ctx as *mut std::ffi::c_void) };
        while let Ok(bytes) = rx.recv() {
            let Ok(message) = serde_json::from_slice::<tinybus::Message>(&bytes) else {
                continue;
            };
            if message.header.kind != tinybus::MessageKind::MethodCall {
                continue;
            }
            let member = message
                .header
                .member
                .as_ref()
                .map(ToString::to_string)
                .unwrap_or_default();
            let body = serde_json::json!({
                "response": reply,
                "observed_member": member,
                "observed_body": message.body,
            });
            let response = tinybus::Message::method_return(&message.header, body);
            let Ok(bytes) = serde_json::to_vec(&response) else {
                continue;
            };
            let _ = unsafe {
                send(
                    host_ctx as *mut std::ffi::c_void,
                    bytes.as_ptr(),
                    bytes.len(),
                )
            };
        }
    });
    unsafe {
        *out = tinybus::module::abi::TbModuleVtable {
            size: std::mem::size_of::<tinybus::module::abi::TbModuleVtable>() as u32,
            _reserved: 0,
            module_ctx: Box::into_raw(Box::new(LinkedFixtureModule { tx })).cast(),
            deliver: linked_fixture_deliver,
            shutdown: linked_fixture_shutdown,
            reinitialize: Some(linked_fixture_reinitialize),
        };
    }
    tinybus::module::abi::TB_OK
}

fn linked_fixture_manifest() -> tinybus::module::manifest::ModuleManifest {
    use tinybus::module::manifest::{
        ModuleIdentity, ModuleManifest, PanicPolicy, ProvidedInterface, MANIFEST_SCHEMA,
    };
    use tinybus::name::{BusName, InterfaceName, MemberName, ObjectPath};
    use tinybus::version::{InterfaceVersion, Version};

    ModuleManifest {
        schema: MANIFEST_SCHEMA,
        module: ModuleIdentity {
            name: "doctor-connector-fixture".to_owned(),
            version: Version::new(1, 0, 0),
            description: "Test-only attested connector fixture".to_owned(),
            homepage: None,
            license: String::new(),
        },
        bus_name: BusName::new(DESTINATION).unwrap(),
        object_path: ObjectPath::new(OBJECT_PATH).unwrap(),
        provides: vec![ProvidedInterface {
            version: InterfaceVersion::provided(
                InterfaceName::new(INTERFACE).unwrap(),
                Version::new(1, 0, 0),
            ),
            methods: vec![MemberName::new("ListConnectedIntegrations").unwrap()],
            signals: Vec::new(),
        }],
        requires: Vec::new(),
        environment: Vec::new(),
        capabilities: Vec::new(),
        lazy_init: false,
        worker_threads: 1,
        on_panic: PanicPolicy::Detach,
    }
}

#[tokio::test]
async fn a_matching_config_binding_is_pinned_to_workspace_policy_and_credential() {
    let _guard = route_lock().lock().await;
    let scratch = tempfile::tempdir().expect("create isolated binding fixture");
    let mut selected = Config {
        config_path: scratch.path().join("config.toml"),
        workspace_dir: scratch.path().join("workspace-a"),
        ..Config::default()
    };
    selected.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_owned();
    selected.composio.api_key = Some("doctor-binding-key-a".to_owned());
    selected.composio.offered_toolkits_allow = vec!["github".to_owned()];
    seed_loaded_scope_for_test(&selected);
    let route = serde_json::json!({
        "route": "direct",
        "api_key": "doctor-binding-key-a",
        "entity_id": "default",
        "state_dir": selected.workspace_dir.join("state"),
        "offered_toolkits": super::super::offered_toolkits(&selected),
    });

    record_configured_route_for_test(&selected, &route);
    assert!(binding_matches_config(&selected));

    let mut other_workspace = selected.clone();
    other_workspace.workspace_dir = scratch.path().join("workspace-b");
    assert!(!binding_matches_config(&other_workspace));

    let mut other_credential = selected.clone();
    other_credential.composio.api_key = Some("doctor-binding-key-b".to_owned());
    assert!(!binding_matches_config(&other_credential));

    let mut other_policy = selected.clone();
    other_policy
        .composio
        .offered_toolkits_allow
        .push("slack".to_owned());
    assert!(!binding_matches_config(&other_policy));

    let mut reconfigured_b = selected.clone();
    reconfigured_b.workspace_dir = scratch.path().join("workspace-b");
    reconfigured_b.composio.api_key = Some("doctor-binding-key-b".to_owned());
    let route_b = serde_json::json!({
        "route": "direct",
        "api_key": "doctor-binding-key-b",
        "entity_id": "default",
        "state_dir": reconfigured_b.workspace_dir.join("state"),
        "offered_toolkits": super::super::offered_toolkits(&reconfigured_b),
    });
    record_configured_route_for_test(&reconfigured_b, &route_b);
    assert!(
        !binding_matches_config(&reconfigured_b),
        "reconfiguring a module loaded under A cannot relabel its state/policy as B"
    );

    *configured_binding()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
}

#[tokio::test]
async fn a_session_from_another_issuer_is_not_a_matching_connector_credential() {
    let scratch = tempfile::tempdir().expect("create isolated issuer fixture");
    let mut config = Config::default();
    config.config_path = scratch.path().join("config.toml");
    config.workspace_dir = scratch.path().join("workspace");
    config.secrets.encrypt = false;
    config.resolved_endpoints.control_plane_url = Some("https://account-b.example".to_owned());
    let profiles = scratch.path().join("auth-profiles.json");
    let raw = serde_json::json!({
        "schema_version": 1,
        "active_profiles": {},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "token": "issuer-mismatch-test-token",
                "metadata": { "session_issuing_backend": "https://account-a.example" }
            }
        }
    })
    .to_string();
    std::fs::write(&profiles, &raw).expect("seed selected issuer profile");

    assert!(selected_credential(&config).is_none());
    assert_eq!(std::fs::read_to_string(&profiles).unwrap(), raw);
    assert!(!scratch.path().join("auth-profiles.lock").exists());
}

#[tokio::test]
async fn a_ready_connector_never_probes_after_the_selected_session_expires() {
    let _route_guard = route_lock().lock().await;
    let scratch = tempfile::tempdir().expect("create isolated expired-session fixture");
    let mut config = Config::default();
    config.config_path = scratch.path().join("config.toml");
    config.workspace_dir = scratch.path().join("workspace");
    config.secrets.encrypt = false;
    let profiles = scratch.path().join("auth-profiles.json");
    let raw = serde_json::json!({
        "schema_version": 1,
        "active_profiles": {},
        "profiles": {
            "app-session:default": {
                "provider": "app-session",
                "profile_name": "default",
                "kind": "token",
                "token": "expired-connector-test-token",
                "metadata": { "session_expires_at": "2000-01-01T00:00:00Z" }
            }
        }
    })
    .to_string();
    std::fs::write(&profiles, &raw).expect("seed isolated expired credential");
    seed_loaded_scope_for_test(&config);

    let bus = MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let client = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let proxy = client.proxy(DESTINATION, OBJECT_PATH, INTERFACE).unwrap();
    let proxy_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runtime = FixtureReadyRuntime {
        proxy,
        proxy_requests: Arc::clone(&proxy_requests),
    };

    let result = call_existing_readonly_with_runtime::<_, serde_json::Value, _>(
        &config,
        "ListConnectedIntegrations",
        (),
        &runtime,
    )
    .await
    .expect("expired credentials make the inspection unavailable");

    assert!(result.is_none());
    assert_eq!(proxy_requests.load(std::sync::atomic::Ordering::SeqCst), 0);
    assert_eq!(std::fs::read_to_string(&profiles).unwrap(), raw);
    assert!(!scratch.path().join("auth-profiles.lock").exists());
    *configured_binding().lock().unwrap() = None;
    *loaded_scope().lock().unwrap() = None;
}

#[tokio::test]
async fn load_scope_is_captured_only_when_the_module_receives_selected_paths_and_policy() {
    let _guard = route_lock().lock().await;
    let scratch = tempfile::tempdir().expect("create isolated load-scope fixture");
    let mut config = Config::default();
    config.config_path = scratch.path().join("config.toml");
    config.workspace_dir = scratch.path().join("workspace-a");
    config.composio.offered_toolkits_allow = vec!["github".to_owned()];
    let policy = super::super::offered_toolkits(&config);

    note_module_config_for_load(&config, &serde_json::json!({ "offered_toolkits": policy }));
    assert!(
        pending_load_scope().lock().unwrap().is_none(),
        "capability-only fallback does not attest a workspace"
    );

    let module_config = serde_json::json!({
        "route": "proxy",
        "state_dir": config.workspace_dir.join("state"),
        "offered_toolkits": super::super::offered_toolkits(&config),
    });
    note_module_config_for_load(&config, &module_config);
    assert_eq!(
        *pending_load_scope().lock().unwrap(),
        load_scope_fingerprint(&config)
    );
    *pending_load_scope().lock().unwrap() = None;
}

#[tokio::test]
async fn existing_readonly_call_returns_a_matching_ready_response_with_a_bound() {
    let _route_guard = route_lock().lock().await;
    let scratch = tempfile::tempdir().expect("create selected connector workspace");
    let mut config = Config::default();
    config.config_path = scratch.path().join("config.toml");
    config.workspace_dir = scratch.path().join("workspace");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_DIRECT.to_owned();
    config.composio.api_key = Some("doctor-boundary-fixture-key".to_owned());
    seed_loaded_scope_for_test(&config);
    let accepted_route = serde_json::json!({
        "route": "direct",
        "api_key": "doctor-boundary-fixture-key",
        "entity_id": "default",
        "state_dir": config.workspace_dir.join("state"),
        "offered_toolkits": super::super::offered_toolkits(&config),
    });
    record_configured_route_for_test(&config, &accepted_route);

    let bus = MemoryBus::new();
    let broker = tinybus::broker::Broker::new();
    let broker_task = broker.clone().spawn(bus.clone());
    let host = tinybus::module::ModuleHost::new(broker);
    host.attach_linked_with_config(
        tinybus::module::LinkedModule {
            descriptor: tinybus::module::abi::TbAbiDescriptor::current(
                "doctor-connector-fixture",
                "1.0.0",
            ),
            manifest: linked_fixture_manifest(),
            init: linked_fixture_init,
        },
        serde_json::json!({
            "reply": { "integrations": [{ "toolkit": "github", "connected": true }] }
        }),
    )
    .expect("admit the fixture through the trusted linked-module host");
    let client = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let reply = serde_json::json!({ "integrations": [{ "toolkit": "github", "connected": true }] });
    let proxy = client.proxy(DESTINATION, OBJECT_PATH, INTERFACE).unwrap();
    let proxy_requests = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let runtime = FixtureReadyRuntime {
        proxy,
        proxy_requests: Arc::clone(&proxy_requests),
    };

    let request = serde_json::json!({ "mode": "backend" });
    let response = call_existing_readonly_with_runtime::<_, serde_json::Value, _>(
        &config,
        "ListConnectedIntegrations",
        request.clone(),
        &runtime,
    )
    .await
    .expect("read-only fixture responds")
    .expect("the exactly matching ready binding is probeable");

    assert_eq!(response["response"], reply);
    assert_eq!(response["observed_member"], "ListConnectedIntegrations");
    assert_eq!(
        response["observed_body"],
        serde_json::json!([request.clone()])
    );
    let mut other_workspace = config.clone();
    other_workspace.workspace_dir = scratch.path().join("workspace-b");
    assert!(
        call_existing_readonly_with_runtime::<_, serde_json::Value, _>(
            &other_workspace,
            "ListConnectedIntegrations",
            request.clone(),
            &runtime,
        )
        .await
        .expect("mismatched binding is unavailable")
        .is_none()
    );
    assert_eq!(proxy_requests.load(std::sync::atomic::Ordering::SeqCst), 1);
    *configured_binding()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = None;
    drop(host);
    broker_task.abort();
}

#[tokio::test]
async fn existing_proxy_failures_are_sanitized_and_time_bounded() {
    let bus = MemoryBus::new();
    tinybus::broker::Broker::new().spawn(bus.clone());
    let service = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    let client = tinybus::Connection::connect(bus.connect().await.unwrap())
        .await
        .unwrap();
    service.request_name(DESTINATION).await.unwrap();
    let calls = Arc::new(Mutex::new(Vec::new()));
    service
        .serve_at(
            tinybus::ObjectPath::new(OBJECT_PATH).unwrap(),
            ReadOnlyFixture {
                calls,
                reply: serde_json::json!({ "private": "raw connector error fixture" }),
                delay: std::time::Duration::from_millis(40),
            },
        )
        .await
        .unwrap();
    let proxy = client.proxy(DESTINATION, OBJECT_PATH, INTERFACE).unwrap();

    let error = call_proxy_readonly_with_timeout::<_, String>(
        &proxy,
        "ListConnectedIntegrations",
        (),
        std::time::Duration::from_millis(1),
    )
    .await
    .expect_err("a slow connector status call must time out");

    assert_eq!(error, ExistingCallError::Failed);
    assert!(!format!("{error:?}").contains("raw connector error fixture"));

    let malformed = call_proxy_readonly_with_timeout::<_, String>(
        &proxy,
        "ListConnectedIntegrations",
        (),
        std::time::Duration::from_secs(1),
    )
    .await
    .expect_err("a malformed response must be surfaced as a safe failure");
    assert_eq!(malformed, ExistingCallError::Failed);
    assert!(!format!("{malformed:?}").contains("raw connector error fixture"));
}

#[tokio::test]
async fn an_absent_connector_does_not_start_runtime_or_load_the_module() {
    if crate::util::module_test_process::run(
        module_path!(),
        "an_absent_connector_does_not_start_runtime_or_load_the_module",
    ) {
        return;
    }
    assert!(!crate::modules::runtime::is_started());
    let scratch = tempfile::tempdir().expect("create isolated missing connector config");
    let mut config = Config::default();
    config.config_path = scratch.path().join("config.toml");
    config.workspace_dir = scratch.path().join("workspace");
    config.composio.mode = crate::config::schema::COMPOSIO_MODE_BACKEND.to_owned();
    let before = crate::modules::ops::state_of(super::super::MODULE_ID);

    let result =
        call_existing_readonly::<_, serde_json::Value>(&config, "ListConnectedIntegrations", ())
            .await
            .expect("absence is an unavailable observation, not a call error");

    assert!(result.is_none());
    assert_eq!(
        crate::modules::ops::state_of(super::super::MODULE_ID),
        before
    );
    assert!(!crate::modules::runtime::is_started());
}
