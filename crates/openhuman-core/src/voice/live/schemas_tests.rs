use super::*;
use crate::core::runtime::DispatchCtx;

#[test]
fn registers_four_live_controllers_in_the_voice_namespace() {
    let controllers = live_registered_controllers();
    let functions: Vec<_> = controllers.iter().map(|c| c.schema.function).collect();
    assert_eq!(
        functions,
        vec![
            "live_providers",
            "live_settings_get",
            "live_settings_set",
            "live_test_provider"
        ]
    );
    assert!(controllers.iter().all(|c| c.schema.namespace == "voice"));
    assert_eq!(live_controller_schemas().len(), 4);
    assert_eq!(live_schemas("missing").function, "unknown");
    let test = live_schemas("voice_live_test_provider");
    assert!(test
        .inputs
        .iter()
        .any(|f| f.name == "provider" && f.required));
}

#[test]
fn live_legacy_catalog_keeps_private_handler_identities_and_schema_fixtures() {
    use crate::core::all::ControllerHandler;

    let controllers = live_registered_controllers();
    let expected: [(&str, ControllerHandler); 4] = [
        ("voice.live_providers", handle_live_providers),
        ("voice.live_settings_get", handle_live_settings_get),
        ("voice.live_settings_set", handle_live_settings_set),
        ("voice.live_test_provider", handle_live_test_provider),
    ];
    assert_eq!(controllers.len(), expected.len());
    for (controller, (method, handler)) in controllers.iter().zip(expected) {
        assert_eq!(controller.schema.method_name(), method);
        assert!(std::ptr::fn_addr_eq(controller.handler, handler));
    }

    let contracts = crate::voice::parameter_contracts();
    for contract in contracts
        .iter()
        .filter(|contract| contract.method.contains(".live_"))
    {
        let controller = controllers
            .iter()
            .find(|controller| controller.schema.method_name() == contract.method)
            .expect("each live voice schema fixture has a legacy row");
        crate::core::all::verify_parameter_schema(&controller.schema, contract)
            .expect("legacy live voice schema retains its independent fixture");
    }
}

#[tokio::test]
async fn handlers_reject_bad_params() {
    let bad = |v: serde_json::Value| v.as_object().unwrap().clone();
    assert!(
        handle_live_test_provider(DispatchCtx::process_default(), bad(serde_json::json!({})))
            .await
            .is_err()
    );
    assert!(handle_live_settings_set(
        DispatchCtx::process_default(),
        bad(serde_json::json!({"gemini": 5}))
    )
    .await
    .is_err());
}
