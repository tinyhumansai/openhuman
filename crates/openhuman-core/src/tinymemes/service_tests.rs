use super::*;
use serde_json::{json, Value};
use tinybus::{broker::Broker, transport::memory::MemoryBus};
use tinymemes_bus::{HostFailure, HostResult};

fn config_in(dir: &std::path::Path) -> Config {
    let mut config = Config::default();
    config.config_path = dir.join("config.toml");
    config.workspace_dir = dir.join("workspace");
    config.secrets.encrypt = false;
    config
}

/// A host connection serving the callbacks, the peer that owns the module's
/// bus name, and a peer that does not.
struct Bus {
    host: Connection,
    module: Connection,
    intruder: Connection,
}

async fn bus() -> Bus {
    let bus = MemoryBus::new();
    Broker::new().spawn(bus.clone());
    let connect = || async {
        Connection::connect(bus.connect().await.unwrap())
            .await
            .unwrap()
    };
    let (host, module, intruder) = (connect().await, connect().await, connect().await);
    install(&host).await.unwrap();
    module.request_name(STATE_INTERFACE).await.unwrap();
    Bus {
        host,
        module,
        intruder,
    }
}

impl Bus {
    async fn call(
        &self,
        from: &Connection,
        member: &str,
        payload: Value,
    ) -> tinybus::Result<Value> {
        let host = self.host.unique_name().unwrap();
        from.proxy(host.as_str(), HOST_PATH, HOST_INTERFACE)
            .unwrap()
            .call(member, (payload,))
            .await
    }
}

fn decode<T: serde::de::DeserializeOwned>(value: Value) -> HostResult<T> {
    serde_json::from_value(value).unwrap()
}

#[tokio::test]
async fn only_the_module_may_call_back_and_only_for_a_live_remix() {
    let dir = tempfile::tempdir().unwrap();
    let bus = bus().await;
    let registration = callbacks::register(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir.path()),
    );
    let correlation = registration.correlation().to_owned();
    let deliver = json!({ "correlation": correlation, "reply": "hi", "remixed": true });

    // The module, for its own live remix.
    let ok = bus
        .call(&bus.module, host_methods::DELIVER, deliver.clone())
        .await
        .unwrap();
    assert_eq!(decode::<()>(ok), Ok(()));

    // A peer that does not own the module's bus name, with a valid id.
    let refused = bus
        .call(&bus.intruder, host_methods::DELIVER, deliver)
        .await;
    assert!(refused.is_err(), "a stranger must not reach the callbacks");

    // The module, with an id nobody registered, or none at all.
    for payload in [
        json!({ "correlation": "not-a-live-remix", "reply": "hi", "remixed": true }),
        json!({ "reply": "hi", "remixed": true }),
        json!({ "correlation": "", "reply": "hi", "remixed": true }),
    ] {
        let refused = bus.call(&bus.module, host_methods::DELIVER, payload).await;
        assert!(refused.is_err(), "no live remix, no answer");
    }
}

#[tokio::test]
async fn a_direct_call_without_the_brokers_context_is_unauthenticated() {
    let bus = bus().await;
    let service = HostService {
        connection: bus.host.clone(),
    };
    let error = tinybus::Interface::call(
        &service,
        &MemberName::new(host_methods::DELIVER).unwrap(),
        json!([{}]),
    )
    .await
    .unwrap_err();
    assert!(error.to_string().contains("unauthenticated"), "{error}");
}

#[tokio::test]
async fn remixes_for_different_workspaces_are_served_at_once_each_for_its_own() {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let bus = bus().await;
    let a = callbacks::register(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir_a.path()),
    );
    let b = callbacks::register(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir_b.path()),
    );
    assert_ne!(a.correlation(), b.correlation());

    // Both are live together: neither waits for the other.
    let save = |registration: &callbacks::Registration, json: &str| {
        let payload =
            json!({ "correlation": registration.correlation(), "kind": "slang", "json": json });
        bus.call(&bus.module, host_methods::SAVE_INDEX, payload)
    };
    let (saved_a, saved_b) = tokio::join!(save(&a, "{\"a\":1}"), save(&b, "{\"b\":1}"));
    assert_eq!(decode::<()>(saved_a.unwrap()), Ok(()));
    assert_eq!(decode::<()>(saved_b.unwrap()), Ok(()));

    let load = |registration: &callbacks::Registration| {
        let payload = json!({ "correlation": registration.correlation(), "kind": "slang" });
        bus.call(&bus.module, host_methods::LOAD_INDEX, payload)
    };
    assert_eq!(
        decode::<Option<String>>(load(&a).await.unwrap()),
        Ok(Some("{\"a\":1}".into()))
    );
    assert_eq!(
        decode::<Option<String>>(load(&b).await.unwrap()),
        Ok(Some("{\"b\":1}".into()))
    );
}

#[tokio::test]
async fn end_retires_the_remix_and_is_harmless_when_repeated_or_unknown() {
    let dir = tempfile::tempdir().unwrap();
    let bus = bus().await;
    let registration = callbacks::register(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir.path()),
    );
    let correlation = registration.correlation().to_owned();
    registration.keep_for_background();

    let ended = bus
        .call(
            &bus.module,
            host_methods::END,
            json!({ "correlation": correlation }),
        )
        .await
        .unwrap();
    assert_eq!(decode::<()>(ended), Ok(()));
    let after = bus
        .call(
            &bus.module,
            host_methods::EVALUATE,
            json!({ "correlation": correlation, "body": {} }),
        )
        .await;
    assert!(after.is_err(), "an ended remix answers nothing");

    for correlation in [correlation.as_str(), "never-registered"] {
        let again = bus
            .call(
                &bus.module,
                host_methods::END,
                json!({ "correlation": correlation }),
            )
            .await
            .unwrap();
        assert_eq!(decode::<()>(again), Ok(()));
    }
    // A stranger cannot end somebody else's remix.
    let dir2 = tempfile::tempdir().unwrap();
    let live = callbacks::register(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir2.path()),
    );
    let refused = bus
        .call(
            &bus.intruder,
            host_methods::END,
            json!({ "correlation": live.correlation() }),
        )
        .await;
    assert!(refused.is_err());
    assert!(callbacks::session(live.correlation()).is_some());
}

#[tokio::test]
async fn an_unavailable_capability_is_reported_not_faked() {
    let dir = tempfile::tempdir().unwrap();
    let bus = bus().await;
    let registration = callbacks::register(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config_in(dir.path()),
    );
    let reply = bus
        .call(
            &bus.module,
            host_methods::EVALUATE,
            json!({ "correlation": registration.correlation(), "body": { "questions": {} } }),
        )
        .await
        .unwrap();
    assert_eq!(
        decode::<tinymemes_bus::EvaluateEnvelope>(reply),
        Err(HostFailure::Unsupported)
    );
}

#[tokio::test]
async fn component_off_refuses_before_any_module_load() {
    let mut config = Config::default();
    config.components.memes = false;
    let request: RemixRequest =
        serde_json::from_str(r#"{"conversation":[],"reply":"hi"}"#).unwrap();
    let error = remix(
        &crate::core::runtime::DispatchCtx::host_default(),
        &config,
        request,
    )
    .await
    .unwrap_err();
    assert!(error.contains("component is off"), "{error}");
}

#[test]
fn foreground_remixes_share_a_lock_per_workspace_and_never_across_workspaces() {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (config_in(dir_a.path()), config_in(dir_b.path()));
    assert!(std::sync::Arc::ptr_eq(&scope_lock(&a), &scope_lock(&a)));
    assert!(!std::sync::Arc::ptr_eq(&scope_lock(&a), &scope_lock(&b)));
}

#[tokio::test]
async fn a_second_remix_for_one_workspace_waits_for_the_first_but_another_workspace_does_not() {
    let (dir_a, dir_b) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let (a, b) = (config_in(dir_a.path()), config_in(dir_b.path()));
    let held = scope_lock(&a);
    let _first = held.lock().await;
    assert!(scope_lock(&a).try_lock().is_err(), "same workspace waits");
    assert!(
        scope_lock(&b).try_lock().is_ok(),
        "another workspace does not"
    );
}
