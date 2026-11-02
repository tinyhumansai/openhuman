use super::*;
use crate::core::{runtime::DispatchCtx, FieldSchema, TypeSchema};
use std::sync::{
    atomic::{AtomicUsize, Ordering},
    Mutex,
};

static CALLS: AtomicUsize = AtomicUsize::new(0);
static TEST_LOCK: Mutex<()> = Mutex::new(());
static EVENTS: Mutex<Vec<&'static str>> = Mutex::new(Vec::new());

fn staged_bound_methods() -> Vec<&'static str> {
    let mut methods = vec!["app_state.snapshot", "app_state.update_local_state"];
    methods.push("composio.execute");
    #[cfg(feature = "mcp")]
    methods.push("mcp_clients.registry_get");
    methods.extend(["doctor.report", "doctor.models"]);
    methods.extend(["encrypt.secret", "decrypt.secret"]);
    methods.extend([
        "keyring_consent.status",
        "keyring_consent.decide",
        "keyring_consent.retry_probe",
        "keyring_consent.respond_operation",
    ]);
    methods.extend([
        "approval.list_pending",
        "approval.list_recent_decisions",
        "approval.list_tool_trust",
        "approval.revoke_tool_trust",
        "approval.decide",
        "approval.get_gate_state",
        "approval.preauthorize_flow",
    ]);
    methods.extend([
        "socket.connect",
        "socket.disconnect",
        "socket.state",
        "socket.emit",
        "socket.connect_with_session",
    ]);
    methods.extend([
        "tool_registry.list",
        "tool_registry.get",
        "tool_registry.diagnostics",
    ]);
    #[cfg(feature = "voice")]
    methods.extend([
        "voice.status",
        "voice.transcribe",
        "voice.transcribe_bytes",
        "voice.tts",
        "voice.reply_synthesize",
        "voice.stt_dispatch",
        "voice.tts_dispatch",
        "voice.set_providers",
        "voice.update_provider_settings",
        "voice.list_models",
        "voice.test_provider",
        "voice.live_providers",
        "voice.live_settings_get",
        "voice.live_settings_set",
        "voice.live_test_provider",
    ]);
    methods.push("threads.upsert");
    methods.extend(["devices.create_pairing", "devices.list", "devices.revoke"]);
    methods.extend([
        "profiles.provision",
        "profiles.deprovision",
        "profiles.list",
        "profiles.status",
        "profiles.set_credential",
        "profiles.clear_credential",
        "profiles.release",
    ]);
    #[cfg(feature = "modules")]
    methods.push("modules.computer_status");
    methods
}

#[test]
fn production_registry_binds_every_socket_controller_to_its_declared_parser() {
    let entries: Vec<_> = registry()
        .iter()
        .filter(|grouped| {
            grouped
                .controller
                .schema
                .method_name()
                .starts_with("socket.")
        })
        .map(|grouped| grouped.controller.clone())
        .collect();
    assert!(
        internal_registry().iter().all(|row| !row
            .controller
            .schema
            .method_name()
            .starts_with("socket.")),
        "socket methods remain public registry entries"
    );
    let contracts = crate::platform::socket::socket_parameter_contracts();
    assert_eq!(
        entries.len(),
        contracts.len(),
        "complete raw socket inventory"
    );
    assert!(entries
        .iter()
        .all(|entry| entry.schema.namespace == "socket"));
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.schema.method_name())
            .collect::<Vec<_>>(),
        contracts
            .iter()
            .map(|contract| contract.method)
            .collect::<Vec<_>>(),
        "independent socket inventory and registration order"
    );
    let grouped: Vec<_> = registry()
        .iter()
        .chain(internal_registry())
        .filter(|row| row.controller.schema.method_name().starts_with("socket."))
        .collect();
    assert!(grouped.iter().all(|row| row.group == DomainGroup::Platform));
    let rpc_methods = [
        "openhuman.socket_connect",
        "openhuman.socket_disconnect",
        "openhuman.socket_state",
        "openhuman.socket_emit",
        "openhuman.socket_connect_with_session",
    ];
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.rpc_method_name())
            .collect::<Vec<_>>(),
        rpc_methods,
        "socket RPC method names remain stable"
    );
    verify_parameter_contracts(&entries, &contracts)
        .expect("raw socket entries use their declared schemas and runtime decoders");
    for contract in &contracts {
        let entry = entries
            .iter()
            .find(|entry| entry.schema.method_name() == contract.method)
            .expect("independent fixture selects a raw socket entry");
        crate::core::all::verify_parameter_schema(&entry.schema, contract)
            .expect("positive socket contract matches the production schema");
        for &required in contract.required {
            for make_optional in [false, true] {
                let mut broken = entry.schema.clone();
                if make_optional {
                    broken
                        .inputs
                        .iter_mut()
                        .find(|field| field.name == required)
                        .expect("required field exists in schema")
                        .required = false;
                } else {
                    broken.inputs.retain(|field| field.name != required);
                }
                assert!(
                    crate::core::all::verify_parameter_schema(&broken, contract).is_err(),
                    "{} independent contract rejects removed/optional {required}",
                    contract.method
                );
            }
        }
    }
}

fn record_event(event: &'static str) {
    EVENTS
        .lock()
        .expect("event lock is not poisoned")
        .push(event);
}

fn clear_events() {
    EVENTS.lock().expect("event lock is not poisoned").clear();
}

fn events() -> Vec<&'static str> {
    EVENTS.lock().expect("event lock is not poisoned").clone()
}

#[derive(Debug)]
struct Parsed(String);

fn parse_value(params: Map<String, Value>) -> Result<Parsed, String> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    record_event("parse");
    params
        .get("value")
        .and_then(Value::as_str)
        .map(|value| Parsed(value.to_owned()))
        .ok_or_else(|| "`value` is required".to_owned())
}

fn run_value(_ctx: DispatchCtx, parsed: Parsed) -> ControllerFuture {
    CALLS.fetch_add(1, Ordering::SeqCst);
    record_event("run");
    Box::pin(async move { Ok(Value::String(parsed.0)) })
}

fn gate(_ctx: DispatchCtx) -> ControllerTask<()> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    record_event("gate");
    Box::pin(async { Ok(()) })
}

fn failing_gate(_ctx: DispatchCtx) -> ControllerTask<()> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    record_event("gate");
    Box::pin(async { Err("gate rejected".to_owned()) })
}

fn failing_parse(_params: Map<String, Value>) -> Result<Parsed, String> {
    CALLS.fetch_add(1, Ordering::SeqCst);
    record_event("parse");
    Err("parser rejected".to_owned())
}

fn schema() -> ControllerSchema {
    ControllerSchema {
        namespace: "bound_test",
        function: "echo",
        description: "Echo one value through a bound parser.",
        inputs: vec![FieldSchema {
            name: "value",
            ty: TypeSchema::String,
            comment: "Value to echo.",
            required: true,
        }],
        outputs: vec![FieldSchema {
            name: "value",
            ty: TypeSchema::String,
            comment: "Echoed value.",
            required: true,
        }],
    }
}

fn params() -> Map<String, Value> {
    serde_json::from_value(serde_json::json!({"value":"fixture"})).expect("fixture is an object")
}

#[test]
fn bound_controller_uses_its_parser_for_decode_and_runtime_invocation() {
    let _lock = TEST_LOCK.lock().expect("test lock is not poisoned");
    CALLS.store(0, Ordering::SeqCst);
    clear_events();
    let controller = BoundRegisteredController::typed(schema(), parse_value, run_value);

    controller
        .decode_params(params())
        .expect("the bound parser accepts the fixture");
    assert_eq!(CALLS.load(Ordering::SeqCst), 1, "decoder only parses");
    assert_eq!(events(), ["parse"]);

    let output = futures::executor::block_on(controller.invoke(DispatchCtx::unbound(), params()))
        .expect("runtime invocation succeeds");
    assert_eq!(output, Value::String("fixture".to_owned()));
    assert_eq!(
        CALLS.load(Ordering::SeqCst),
        3,
        "invocation parses then runs"
    );
    assert_eq!(events(), ["parse", "parse", "run"]);
}

#[test]
fn bound_controller_decode_does_not_run_gate_or_business_handler() {
    let _lock = TEST_LOCK.lock().expect("test lock is not poisoned");
    CALLS.store(0, Ordering::SeqCst);
    clear_events();
    let controller =
        BoundRegisteredController::after_gate(schema(), gate, parse_value, |_ctx, (), parsed| {
            run_value(DispatchCtx::unbound(), parsed)
        });

    controller
        .decode_params(params())
        .expect("the bound parser accepts the fixture");
    assert_eq!(CALLS.load(Ordering::SeqCst), 1, "only the parser ran");
    assert_eq!(events(), ["parse"]);

    let output = futures::executor::block_on(controller.invoke(DispatchCtx::unbound(), params()))
        .expect("gated runtime invocation succeeds");
    assert_eq!(output, Value::String("fixture".to_owned()));
    assert_eq!(CALLS.load(Ordering::SeqCst), 4, "gate, parser, then runner");
    assert_eq!(events(), ["parse", "gate", "parse", "run"]);
}

#[test]
fn bound_controller_runs_gate_before_parser() {
    let _lock = TEST_LOCK.lock().expect("test lock is not poisoned");
    CALLS.store(0, Ordering::SeqCst);
    clear_events();
    let controller =
        BoundRegisteredController::after_gate(schema(), gate, parse_value, run_value_after_gate);

    let output = futures::executor::block_on(controller.invoke(DispatchCtx::unbound(), params()))
        .expect("gated runtime invocation succeeds");
    assert_eq!(output, Value::String("fixture".to_owned()));
    assert_eq!(CALLS.load(Ordering::SeqCst), 3);
    assert_eq!(events(), ["gate", "parse", "run"]);
}

fn run_value_after_gate(_ctx: DispatchCtx, _gate: (), parsed: Parsed) -> ControllerFuture {
    run_value(DispatchCtx::unbound(), parsed)
}

#[test]
fn bound_controller_preserves_parser_errors() {
    let _lock = TEST_LOCK.lock().expect("test lock is not poisoned");
    clear_events();
    let controller = BoundRegisteredController::typed(schema(), parse_value, run_value);
    let error = controller
        .decode_params(Map::new())
        .expect_err("missing value is rejected");
    assert_eq!(error, "`value` is required");
}

#[test]
fn bound_controller_gate_error_short_circuits_parser_and_runner() {
    let _lock = TEST_LOCK.lock().expect("test lock is not poisoned");
    CALLS.store(0, Ordering::SeqCst);
    clear_events();
    let controller = BoundRegisteredController::after_gate(
        schema(),
        failing_gate,
        parse_value,
        run_value_after_gate,
    );

    let error = futures::executor::block_on(controller.invoke(DispatchCtx::unbound(), params()))
        .expect_err("gate failure is returned");
    assert_eq!(error, "gate rejected");
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(events(), ["gate"]);
}

#[test]
fn bound_controller_runtime_parser_error_short_circuits_runner() {
    let _lock = TEST_LOCK.lock().expect("test lock is not poisoned");
    CALLS.store(0, Ordering::SeqCst);
    clear_events();
    let controller = BoundRegisteredController::typed(schema(), failing_parse, run_value);

    let error = futures::executor::block_on(controller.invoke(DispatchCtx::unbound(), params()))
        .expect_err("parser failure is returned");
    assert_eq!(error, "parser rejected");
    assert_eq!(CALLS.load(Ordering::SeqCst), 1);
    assert_eq!(events(), ["parse"]);
}

#[test]
fn production_registry_binds_threads_upsert_to_its_declared_parser() {
    use serde_json::json;

    // Read the unfiltered production registry directly. The domain catalog is
    // useful for wire-shape checks, but it can remain correct if production
    // registration accidentally falls back to a legacy handler.
    let entries: Vec<_> = registry()
        .iter()
        .chain(internal_registry())
        .map(|grouped| &grouped.controller)
        .filter(|entry| entry.schema.method_name() == "threads.upsert")
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "threads.upsert has one raw registry entry"
    );
    let entry = entries[0];
    let bound_methods: Vec<_> = registry()
        .iter()
        .chain(internal_registry())
        .filter(|grouped| grouped.controller.is_bound())
        .map(|grouped| grouped.controller.schema.method_name())
        .collect();
    assert_eq!(
        bound_methods,
        staged_bound_methods(),
        "staged bound inventory"
    );
    assert!(
        entry.is_bound(),
        "threads.upsert must be bound at the production registration site"
    );

    let required = ["id", "title", "created_at"];
    let declared: Vec<_> = entry
        .schema
        .inputs
        .iter()
        .filter(|field| field.required)
        .map(|field| field.name)
        .collect();
    assert_eq!(
        declared, required,
        "required fields are independent contract"
    );

    for example in [
        json!({"id":"t1", "title":"Hello", "created_at":"2026-04-22T00:00:00Z"}),
        json!({"id":"t2", "title":"Thread", "created_at":"2026-04-22T00:00:00Z", "labels":["work"]}),
    ] {
        let params = example.as_object().expect("example is an object").clone();
        validate_params(&entry.schema, &params)
            .expect("actual registered schema accepts the example");
        entry
            .decode_params(params)
            .expect("actual raw registry entry uses the runtime parser");
    }

    for missing in ["id", "title", "created_at"] {
        let mut params: Map<String, Value> = serde_json::from_value(json!({
            "id":"t1", "title":"Hello", "created_at":"2026-04-22T00:00:00Z"
        }))
        .expect("valid upsert params");
        params.remove(missing);
        assert!(
            validate_params(&entry.schema, &params).is_err(),
            "registered schema requires `{missing}`"
        );
        assert!(
            entry.decode_params(params).is_err(),
            "runtime parser rejects missing `{missing}`"
        );
    }
}

#[test]
#[cfg(feature = "mcp")]
fn production_registry_binds_mcp_registry_get_to_its_declared_parser() {
    let entries: Vec<_> = registry()
        .iter()
        .filter(|grouped| grouped.controller.schema.method_name() == "mcp_clients.registry_get")
        .collect();
    assert_eq!(entries.len(), 1, "registry_get has one public raw entry");
    assert!(!internal_registry()
        .iter()
        .any(|grouped| grouped.controller.schema.method_name() == "mcp_clients.registry_get"));
    let grouped = entries[0];
    assert_eq!(grouped.group, DomainGroup::Mcp);
    let entry = &grouped.controller;
    assert!(
        entry.is_bound(),
        "registry_get must bind at production registration"
    );
    assert_eq!(
        entry.rpc_method_name(),
        "openhuman.mcp_clients_registry_get"
    );
    let contract = crate::mcp::registry::registry_get_parameter_contract();
    crate::core::all::verify_parameter_contracts(
        std::slice::from_ref(entry),
        std::slice::from_ref(&contract),
    )
    .expect("raw MCP entry matches its independent contract and bound decoder");
    assert!(entry.schema.inputs.iter().all(|field| field.required));

    for remove in [true, false] {
        let mut broken = entry.schema.clone();
        if remove {
            broken
                .inputs
                .retain(|field| field.name != contract.required[0]);
        } else {
            broken
                .inputs
                .iter_mut()
                .find(|field| field.name == contract.required[0])
                .expect("positive contract includes its required input")
                .required = false;
        }
        assert!(required_input_contract(&broken, contract.required)
            .unwrap_err()
            .contains("required inputs mismatch"));
    }
}

#[test]
#[cfg(feature = "modules")]
fn production_registry_binds_computer_status_to_its_declared_parser() {
    use serde_json::json;

    let entries: Vec<_> = internal_registry()
        .iter()
        .filter(|grouped| grouped.controller.schema.method_name() == "modules.computer_status")
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "computer_status has one internal raw registry entry"
    );
    assert!(!registry()
        .iter()
        .any(|grouped| grouped.controller.schema.method_name() == "modules.computer_status"));
    let grouped = entries[0];
    assert_eq!(grouped.group, DomainGroup::Modules);
    let entry = &grouped.controller;
    assert!(
        entry.is_bound(),
        "computer_status must bind at production registration"
    );
    assert_eq!(entry.rpc_method_name(), "openhuman.modules_computer_status");
    let contract = crate::modules::schemas::computer_status_parameter_contract();
    crate::core::all::verify_parameter_contracts(
        std::slice::from_ref(entry),
        std::slice::from_ref(&contract),
    )
    .expect("raw Modules entry matches its independent contract and bound decoder");

    // Runtime parsing intentionally keeps its permissive false default for
    // non-boolean inputs; these maps are parser cases, not schema examples.
    for example in [
        json!({"load":null}),
        json!({"load":"true"}),
        json!({"load":1}),
    ] {
        let params = example.as_object().expect("parser case object").clone();
        entry
            .decode_params(params)
            .expect("actual bound parser preserves permissive false defaults");
    }
}

#[test]
fn production_registry_binds_composio_execute_to_its_declared_parser() {
    use serde_json::json;

    // Inspect the unfiltered registry so a legacy row cannot hide behind the
    // domain schema catalog or live surface filters.
    let entries: Vec<_> = registry()
        .iter()
        .chain(internal_registry())
        .map(|grouped| &grouped.controller)
        .filter(|entry| entry.schema.method_name() == "composio.execute")
        .collect();
    assert_eq!(
        entries.len(),
        1,
        "composio.execute has one raw registry entry"
    );
    let entry = entries[0];
    let bound_methods: Vec<_> = registry()
        .iter()
        .chain(internal_registry())
        .filter(|grouped| grouped.controller.is_bound())
        .map(|grouped| grouped.controller.schema.method_name())
        .collect();
    assert_eq!(
        bound_methods,
        staged_bound_methods(),
        "staged bound inventory"
    );
    assert!(
        entry.is_bound(),
        "composio.execute must be bound at registration"
    );
    assert_eq!(entry.rpc_method_name(), "openhuman.composio_execute");

    for name in ["arguments", "connection_id"] {
        assert!(
            entry
                .schema
                .inputs
                .iter()
                .any(|field| field.name == name && !field.required),
            "{name} must remain optional"
        );
    }

    required_input_contract(&entry.schema, &["tool"])
        .expect("independent required-field contract matches production schema");
    let mut missing_required = entry.schema.clone();
    missing_required.inputs.retain(|field| field.name != "tool");
    assert!(required_input_contract(&missing_required, &["tool"])
        .unwrap_err()
        .contains("required inputs mismatch"));
    let mut optional_required = entry.schema.clone();
    optional_required
        .inputs
        .iter_mut()
        .find(|field| field.name == "tool")
        .expect("tool schema field")
        .required = false;
    assert!(required_input_contract(&optional_required, &["tool"])
        .unwrap_err()
        .contains("required inputs mismatch"));

    for example in [
        json!({"tool":"GMAIL_SEND_EMAIL"}),
        json!({"tool":" GMAIL_SEND_EMAIL ","arguments":{"to":"test@example.invalid"},"connection_id":"conn-1"}),
        json!({"tool":"GMAIL_SEND_EMAIL","arguments":null,"connection_id":null}),
        json!({"tool":"GMAIL_SEND_EMAIL","arguments":42,"connection_id":""}),
    ] {
        let params = example.as_object().expect("example is an object").clone();
        validate_params(&entry.schema, &params)
            .expect("actual registered schema accepts the example");
        entry
            .decode_params(params)
            .expect("actual raw registry entry uses the runtime parser");
    }

    let mut missing_tool: Map<String, Value> = serde_json::from_value(json!({
        "arguments":{"to":"test@example.invalid"}
    }))
    .expect("object params");
    assert!(validate_params(&entry.schema, &missing_tool).is_err());
    assert!(entry.decode_params(missing_tool.clone()).is_err());
    missing_tool.insert("tool".into(), json!("GMAIL_SEND_EMAIL"));
    assert!(validate_params(&entry.schema, &missing_tool).is_ok());
    assert!(entry.decode_params(missing_tool).is_ok());
}

#[cfg(feature = "voice")]
#[test]
fn production_registry_binds_every_core_voice_controller_to_its_parser() {
    // Discover the namespace from raw registries independently of the fixture
    // rows; surface/domain filters must not hide a missing binding.
    let grouped: Vec<_> = registry()
        .iter()
        .chain(internal_registry())
        .filter(|row| row.controller.schema.method_name().starts_with("voice."))
        .collect();
    let entries: Vec<_> = grouped.iter().map(|row| row.controller.clone()).collect();
    let contracts = crate::voice::parameter_contracts();
    let expected_methods = [
        "voice.status",
        "voice.transcribe",
        "voice.transcribe_bytes",
        "voice.tts",
        "voice.reply_synthesize",
        "voice.stt_dispatch",
        "voice.tts_dispatch",
        "voice.set_providers",
        "voice.update_provider_settings",
        "voice.list_models",
        "voice.test_provider",
        "voice.live_providers",
        "voice.live_settings_get",
        "voice.live_settings_set",
        "voice.live_test_provider",
    ];
    assert_eq!(
        entries.len(),
        expected_methods.len(),
        "complete core voice inventory"
    );
    assert!(grouped.iter().all(|row| row.group == DomainGroup::Voice));
    assert_eq!(
        entries
            .iter()
            .map(|entry| entry.schema.method_name())
            .collect::<Vec<_>>(),
        expected_methods,
        "independent core voice inventory and registry order"
    );
    assert_eq!(
        contracts
            .iter()
            .map(|contract| contract.method)
            .collect::<Vec<_>>(),
        expected_methods,
        "authored parameter fixtures cover the complete ordered voice inventory"
    );
    crate::core::all::verify_parameter_contracts(&entries, &contracts)
        .expect("raw voice entries use their declared schemas and runtime decoders");

    for (entry, contract) in entries.into_iter().zip(contracts) {
        for &name in contract.required {
            for remove in [true, false] {
                let mut broken = entry.schema.clone();
                if remove {
                    broken.inputs.retain(|input| input.name != name);
                } else {
                    broken
                        .inputs
                        .iter_mut()
                        .find(|input| input.name == name)
                        .expect("independent required name is present")
                        .required = false;
                }
                assert!(
                    required_input_contract(&broken, contract.required).is_err(),
                    "{} independent required contract rejects {name} removal/optional mutation",
                    contract.method
                );
            }
        }
    }
}
#[path = "all_profiles_controller_tests.rs"]
mod profiles_controller_tests;

#[path = "all_encryption_controller_tests.rs"]
mod encryption_controller_tests;

#[path = "all_doctor_controller_tests.rs"]
mod doctor_controller_tests;

#[path = "all_app_state_controller_tests.rs"]
mod app_state_controller_tests;

#[path = "all_devices_controller_tests.rs"]
mod devices_controller_tests;

#[path = "all_approval_controller_tests.rs"]
mod approval_controller_tests;

#[path = "all_bound_registry_test_support.rs"]
mod registry_test_support;

#[path = "all_bound_registry_contract_tests.rs"]
mod registry_contract_tests;

#[path = "all_keyring_consent_controller_tests.rs"]
mod keyring_consent_controller_tests;

#[path = "all_tool_registry_controller_tests.rs"]
mod tool_registry_controller_tests;
