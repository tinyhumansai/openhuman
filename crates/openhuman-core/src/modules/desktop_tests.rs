//! Desktop host tests that avoid downloading a native module or reaching Jev.

use std::sync::{Arc, Mutex};

use serde_json::{json, Value};
use tinybus::broker::Broker;
use tinybus::service::Interface;
use tinybus::transport::memory::MemoryBus;
use tinybus::{Connection, InterfaceName, MemberName, ObjectPath};
use tinycomputer_bus::{names, DesktopError, DesktopResponse};

use super::{call, call_with_proxy, fingerprint, jev_ready, module_config, state, MODULE_ID};
use crate::config::Config;
use crate::security::credentials::{api_key, AuthService};

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

#[test]
fn backend_key_is_selected_ahead_of_direct_openrouter_and_rotates_privately() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = config_in(dir.path());
    config.resolved_endpoints.product_identity = Some("openhuman-test".into());
    AuthService::from_config(&config)
        .store_provider_token(
            "provider:openrouter",
            "default",
            "or_test_direct",
            Default::default(),
            true,
        )
        .expect("store direct key");

    let direct = module_config(&config);
    assert_eq!(direct["jev"]["provider"], "open_router");
    assert_eq!(direct["jev"]["api_key"], "or_test_direct");
    assert!(jev_ready(&config));

    api_key::store_api_key(&config, "th_test_backend").expect("store backend key");
    let hosted = module_config(&config);
    assert_eq!(hosted["jev"]["provider"], "tiny_humans_open_router");
    assert_eq!(hosted["jev"]["api_key"], "th_test_backend");
    assert!(hosted["jev"]["sdk_name"].is_string());
    assert_ne!(fingerprint(&hosted), fingerprint(&direct));

    api_key::store_api_key(&config, "th_test_rotated").expect("rotate key");
    let rotated = module_config(&config);
    assert_eq!(rotated["jev"]["api_key"], "th_test_rotated");
    assert_ne!(fingerprint(&rotated), fingerprint(&hosted));
    assert_eq!(module_config(&config), rotated);
}

#[tokio::test]
async fn disabled_modules_refuse_calls_before_loading_or_reinitializing() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut config = config_in(dir.path());
    config.modules.enabled = false;
    let (status, detail) = state(&config);
    assert_eq!(status, "unsupported");
    assert!(detail.as_deref().is_some_and(|s| s.contains("disabled")));

    let error = call(&config, names::methods::PERMISSIONS, json!({}))
        .await
        .expect_err("disabled host must refuse desktop call");
    assert!(error.contains("disabled"), "{error}");
}

struct MockDesktop {
    calls: Arc<Mutex<Vec<(String, Value)>>>,
}

#[async_trait::async_trait]
impl Interface for MockDesktop {
    fn name(&self) -> InterfaceName {
        InterfaceName::new(names::INTERFACE).expect("contract interface")
    }

    fn members(&self) -> Vec<MemberName> {
        [
            names::methods::LIST_APPS,
            names::methods::PERMISSIONS,
            names::methods::RUN_GOAL,
            names::methods::RESOLVE_INTENT,
        ]
        .into_iter()
        .map(|name| MemberName::new(name).expect("contract member"))
        .collect()
    }

    async fn call(&self, member: &MemberName, args: Value) -> tinybus::Result<Value> {
        self.calls
            .lock()
            .expect("calls lock")
            .push((member.as_str().to_owned(), args));
        if member.as_str() == names::methods::PERMISSIONS {
            return Err(tinybus::Error::MethodFailed {
                name: "ai.tinyhumans.tinycomputer.Desktop.Error.Permission".to_owned(),
                message: "permission denied".to_owned(),
            });
        }
        serde_json::to_value(DesktopResponse::err(
            "list-apps",
            DesktopError::new("PERM_DENIED", "Accessibility permission needed"),
        ))
        .map_err(tinybus::Error::from)
    }
}

#[tokio::test]
async fn bus_client_preserves_envelopes_and_names_transport_failures() {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let service = Connection::connect(bus.connect().await.expect("service transport"))
        .await
        .expect("service");
    let calls = Arc::new(Mutex::new(Vec::new()));
    service
        .serve_at(
            ObjectPath::new(names::OBJECT_PATH).expect("contract path"),
            MockDesktop {
                calls: Arc::clone(&calls),
            },
        )
        .await
        .expect("serve");
    service.request_name(names::INTERFACE).await.expect("name");
    let client = Connection::connect(bus.connect().await.expect("client transport"))
        .await
        .expect("client");
    let proxy = client
        .proxy(names::INTERFACE, names::OBJECT_PATH, names::INTERFACE)
        .expect("proxy");

    let response = call_with_proxy(&proxy, names::methods::LIST_APPS, json!({"limit": 2}))
        .await
        .expect("structured envelope is a successful bus delivery");
    assert!(!response.ok);
    assert_eq!(
        response.error.as_ref().map(|e| e.code.as_str()),
        Some("PERM_DENIED")
    );
    assert_eq!(
        calls.lock().expect("calls lock")[0],
        ("ListApps".to_owned(), json!([{"limit": 2}]))
    );

    let error = call_with_proxy(&proxy, names::methods::PERMISSIONS, json!({}))
        .await
        .expect_err("bus failure must remain a failure");
    assert!(error.contains("desktop Permissions failed"), "{error}");
    assert!(error.contains("permission denied"), "{error}");

    // A goal carries user intent to Jev. A peer merely claiming the contract
    // name has no module attestation, so TinyBus must refuse delivery.
    let error = call_with_proxy(&proxy, names::methods::RUN_GOAL, json!({"goal": "hello"}))
        .await
        .expect_err("unattested mock must not receive a confidential goal");
    assert!(error.contains("desktop RunGoal failed"), "{error}");
    assert_eq!(calls.lock().expect("calls lock").len(), 2);
}

#[test]
fn registry_and_contract_address_agree() {
    let record = crate::modules::registry::find(MODULE_ID).expect("desktop registry entry");
    assert_eq!(record.bus_name, names::INTERFACE);
    assert_eq!(record.object_path, names::OBJECT_PATH);
}
