use super::*;

#[cfg(feature = "modules")]
struct RegistrationProbe {
    dropped: std::sync::Arc<tokio::sync::Notify>,
    context: Option<std::sync::Arc<()>>,
}

#[cfg(feature = "modules")]
impl Drop for RegistrationProbe {
    fn drop(&mut self) {
        // Wake the waiter only after the callback releases its context.
        drop(self.context.take());
        self.dropped.notify_one();
    }
}

#[cfg(feature = "modules")]
#[async_trait::async_trait]
impl tinybus::service::Interface for RegistrationProbe {
    fn name(&self) -> tinybus::InterfaceName {
        tinybus::InterfaceName::new("ai.tinyhumans.openhuman.TestRegistration")
            .expect("interface name")
    }

    fn members(&self) -> Vec<tinybus::MemberName> {
        vec![tinybus::MemberName::new("Ping").expect("member name")]
    }

    async fn call(
        &self,
        _member: &tinybus::MemberName,
        _args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        let _keep_callback_context_alive = &self.context;
        Ok(serde_json::Value::Null)
    }
}

#[cfg(feature = "modules")]
struct SecondRegistrationProbe;

#[cfg(feature = "modules")]
#[async_trait::async_trait]
impl tinybus::service::Interface for SecondRegistrationProbe {
    fn name(&self) -> tinybus::InterfaceName {
        tinybus::InterfaceName::new("ai.tinyhumans.openhuman.TestRegistrationSecond")
            .expect("interface name")
    }

    fn members(&self) -> Vec<tinybus::MemberName> {
        vec![tinybus::MemberName::new("Ping").expect("member name")]
    }

    async fn call(
        &self,
        _member: &tinybus::MemberName,
        _args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

#[test]
fn the_disabled_message_names_the_component_and_the_switch() {
    let message = disabled_message();
    assert!(
        message.starts_with("COMPONENT_DISABLED:tinyflows"),
        "{message}"
    );
    assert!(message.contains("[components] tinyflows = true"));
}

#[test]
fn the_module_is_available_only_while_its_component_is_on() {
    let mut config = Config::default();
    assert_eq!(available(&config), cfg!(feature = "modules"));
    config.components.tinyflows = false;
    assert!(!available(&config));
}

#[tokio::test]
async fn a_switched_off_component_never_loads_the_module() {
    let mut config = Config::default();
    config.components.tinyflows = false;
    let error = proxy(&config).await.unwrap_err();
    assert!(error.starts_with("COMPONENT_DISABLED:tinyflows"), "{error}");
    let error = run_proxy(&config).await.unwrap_err();
    assert!(error.starts_with("COMPONENT_DISABLED:tinyflows"), "{error}");
    let error = copilot_proxy(&config).await.unwrap_err();
    assert!(error.starts_with("COMPONENT_DISABLED:tinyflows"), "{error}");
}

#[tokio::test]
async fn the_schedule_object_answers_on_the_contracts_names() {
    let config = Config::default();
    let proxy = proxy(&config).await.expect("module attaches");
    let millis: i64 = proxy
        .call(
            tinyflows_bus::schedule::members::PARSE_HUMAN_DELAY,
            ("30m",),
        )
        .await
        .unwrap();
    assert_eq!(millis, 30 * 60 * 1000);
}

#[tokio::test]
async fn the_copilot_object_answers_tool_catalog_over_its_bus_contract() {
    let config = Config::default();
    let proxy = copilot_proxy(&config)
        .await
        .expect("module copilot interface attaches");
    let tools: Vec<tinyflows_bus::copilot::ToolDescriptor> = proxy
        .call(
            "ToolCatalog",
            (tinyflows_bus::copilot::ToolCatalogRequest::default(),),
        )
        .await
        .expect("copilot catalogue call succeeds");
    let names: Vec<_> = tools.iter().map(|tool| tool.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "propose_workflow",
            "revise_workflow",
            "edit_workflow",
            "validate_workflow",
            "get_flow_history",
            "list_flow_runs",
            "resume_flow_run",
            "cancel_flow_run",
            "create_workflow",
            "duplicate_flow",
            "list_flows",
            "get_flow",
            "get_flow_run",
            "list_agent_definitions",
            "list_node_kinds",
            "get_node_kind_contract",
            "dry_run_workflow",
            "run_flow",
            "save_workflow",
            "suggest_workflows",
        ]
    );
    let run_flow = tools.iter().find(|tool| tool.name == "run_flow").unwrap();
    assert_eq!(run_flow.permission, "Write");
    assert!(run_flow.external_effect);
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn cancelling_partial_callback_registration_unserves_and_releases_its_context() {
    use std::sync::Arc;

    let path = format!(
        "/ai/tinyhumans/openhuman/TestRegistration/{}",
        uuid::Uuid::new_v4().simple()
    );
    let dropped = Arc::new(tokio::sync::Notify::new());
    let context = Arc::new(());
    let weak_context = Arc::downgrade(&context);
    let (registered_tx, registered_rx) = tokio::sync::oneshot::channel();
    let (continue_tx, continue_rx) = tokio::sync::oneshot::channel();
    let task_path = path.clone();
    let task_dropped = Arc::clone(&dropped);
    let task_context = Arc::clone(&context);
    let registration = tokio::spawn(async move {
        serve_pair_after_first(
            task_path,
            RegistrationProbe {
                dropped: task_dropped,
                context: Some(task_context),
            },
            SecondRegistrationProbe,
            async move {
                let _ = registered_tx.send(());
                let _ = continue_rx.await;
            },
        )
        .await
    });
    registered_rx.await.expect("first interface is served");

    let connected = super::super::runtime::runtime()
        .await
        .expect("module runtime exists");
    let proxy = connected
        .connection()
        .proxy(
            HOST_BUS_NAME,
            &path,
            "ai.tinyhumans.openhuman.TestRegistration",
        )
        .expect("probe proxy");
    assert!(proxy.call::<serde_json::Value>("Ping", ()).await.is_ok());

    registration.abort();
    let cleanup = dropped.notified();
    let _ = continue_tx.send(());
    tokio::time::timeout(std::time::Duration::from_secs(5), cleanup)
        .await
        .expect("detached registration settles and unregisters its interface");
    drop(context);
    assert!(
        weak_context.upgrade().is_none(),
        "callback context is released"
    );
    assert!(proxy.call::<serde_json::Value>("Ping", ()).await.is_err());
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn single_callback_registration_unserves_when_its_guard_drops() {
    use std::sync::Arc;

    let path = format!(
        "/ai/tinyhumans/openhuman/TestRegistration/{}",
        uuid::Uuid::new_v4().simple()
    );
    let dropped = Arc::new(tokio::sync::Notify::new());
    let context = Arc::new(());
    let weak_context = Arc::downgrade(&context);
    let registration = serve_registered(
        path.clone(),
        RegistrationProbe {
            dropped: Arc::clone(&dropped),
            context: Some(Arc::clone(&context)),
        },
    )
    .await
    .expect("single callback registration succeeds");

    let runtime = super::super::runtime::runtime()
        .await
        .expect("module runtime exists");
    let proxy = runtime
        .connection()
        .proxy(
            HOST_BUS_NAME,
            &path,
            "ai.tinyhumans.openhuman.TestRegistration",
        )
        .expect("probe proxy");
    assert!(proxy.call::<serde_json::Value>("Ping", ()).await.is_ok());

    drop(registration);
    let cleanup = dropped.notified();
    tokio::time::timeout(std::time::Duration::from_secs(5), cleanup)
        .await
        .expect("registration guard drop removes its interface");
    drop(context);
    assert!(
        weak_context.upgrade().is_none(),
        "callback context is released"
    );
    assert!(proxy.call::<serde_json::Value>("Ping", ()).await.is_err());
}

#[cfg(feature = "modules")]
struct CapsHostManifestProbe;

#[cfg(feature = "modules")]
#[async_trait::async_trait]
impl tinybus::service::Interface for CapsHostManifestProbe {
    fn name(&self) -> tinybus::InterfaceName {
        tinybus::InterfaceName::new(tinyflows_bus::runs::CAPS_INTERFACE)
            .expect("CapsHost interface name")
    }

    fn members(&self) -> Vec<tinybus::MemberName> {
        vec![tinybus::MemberName::new("AgentCancel").expect("member name")]
    }

    async fn call(
        &self,
        _member: &tinybus::MemberName,
        _args: serde_json::Value,
    ) -> tinybus::Result<serde_json::Value> {
        Ok(serde_json::Value::Null)
    }
}

#[cfg(feature = "modules")]
#[tokio::test]
async fn serving_caps_host_advertises_v2_without_replacing_other_peer_manifest_entries() {
    let runtime = super::super::runtime::runtime()
        .await
        .expect("module runtime exists");
    let connection = runtime.connection();
    let other_interface = tinybus::InterfaceName::new("ai.tinyhumans.openhuman.TestPreserved")
        .expect("interface name");
    let caps_interface = tinybus::InterfaceName::new(tinyflows_bus::runs::CAPS_INTERFACE)
        .expect("CapsHost interface name");
    let mut original = connection
        .manifest_of(HOST_BUS_NAME)
        .await
        .expect("manifest lookup succeeds")
        .unwrap_or_else(|| tinybus::PeerManifest::new("openhuman-module-host"));
    original
        .provides
        .retain(|item| item.interface != caps_interface);
    original.provides.push(tinybus::InterfaceVersion::provided(
        caps_interface.clone(),
        tinybus::Version::new(1, 0, 0),
    ));
    original.provides.push(tinybus::InterfaceVersion::provided(
        other_interface,
        tinybus::Version::new(4, 2, 0),
    ));
    let prior_name = original.name.clone();
    let prior_version = original.version.clone();
    let prior_consumes = original.consumes.clone();
    connection
        .announce(&original)
        .await
        .expect("prior manifest can be announced");

    let path = format!(
        "/ai/tinyhumans/openhuman/CapsHost/{}",
        uuid::Uuid::new_v4().simple()
    );
    let registration = serve_registered(path, CapsHostManifestProbe)
        .await
        .expect("CapsHost registration succeeds");
    let announced = connection
        .manifest_of(HOST_BUS_NAME)
        .await
        .expect("manifest lookup succeeds")
        .expect("the module runtime peer is announced");

    let caps_host = announced
        .provided(&caps_interface)
        .expect("CapsHost is advertised");
    assert_eq!(caps_host.speaks, tinybus::Version::new(2, 0, 0));
    assert_eq!(announced.name, prior_name);
    assert_eq!(announced.version, prior_version);
    assert!(announced
        .provides
        .iter()
        .any(|item| item.interface.as_str() == "ai.tinyhumans.openhuman.TestPreserved"));
    assert_eq!(announced.consumes, prior_consumes);

    let proxy = connection
        .proxy(
            HOST_BUS_NAME,
            &registration.0,
            tinyflows_bus::runs::CAPS_INTERFACE,
        )
        .expect("CapsHost proxy resolves");
    proxy
        .call::<serde_json::Value>(
            "AgentCancel",
            (serde_json::json!({"client_request_id":"probe"}),),
        )
        .await
        .expect("the advertised member is actually served");
}
