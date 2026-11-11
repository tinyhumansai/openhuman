use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_identity, ControllerHandler,
    ControllerParamContract,
};
use serde_json::json;

#[test]
fn legacy_handlers_keep_their_identity_and_order() {
    let expected: [(&str, ControllerHandler); 2] = [
        ("snapshot", handle_snapshot),
        ("update_local_state", handle_update_local_state),
    ];
    let controllers = all_app_state_registered_controllers();
    assert_eq!(controllers.len(), expected.len());
    for ((function, handler), controller) in expected.iter().zip(&controllers) {
        assert_eq!(controller.schema.namespace, "app_state");
        assert_eq!(controller.schema.function, *function);
        assert!(std::ptr::fn_addr_eq(controller.handler, *handler));
    }
}

pub(super) fn parameter_contracts() -> Vec<ControllerParamContract> {
    use serde_json::json;

    vec![
        ControllerParamContract {
            method: "app_state.snapshot",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "app_state.update_local_state",
            required: &[],
            optional: &["encryptionKey", "onboardingTasks", "keyringConsent"],
            examples: vec![
                json!({}),
                json!({
                    "encryptionKey": null,
                    "onboardingTasks": null,
                    "keyringConsent": null
                }),
                json!({
                    "encryptionKey": "  keep whitespace  ",
                    "onboardingTasks": {
                        "accessibilityPermissionGranted": false,
                        "enabledTools": [],
                        "connectedSources": [],
                        "updatedAtMs": null
                    },
                    "keyringConsent": {
                        "storageMode": "",
                        "consentedAtMs": null
                    }
                }),
                json!({"onboardingTasks": [], "keyringConsent": []}),
            ],
        },
    ]
}

#[test]
fn app_state_schema_contracts_and_registration_are_preserved() {
    crate::core::all::expect_schema_namespace(&all_app_state_controller_schemas(), "app_state");
    assert_eq!(all_app_state_controller_schemas().len(), 2);
    assert_schema_controller_parity(
        &all_app_state_controller_schemas(),
        &all_app_state_registered_controllers(),
        "snapshot",
    );
    let s = app_state_schemas("snapshot");
    expect_schema_identity(&s, "app_state", "snapshot");
    assert!(s.inputs.is_empty());
    assert!(!s.outputs.is_empty());
    let s = app_state_schemas("update_local_state");
    expect_schema_identity(&s, "app_state", "update_local_state");
    assert_eq!(s.inputs.len(), 3);
    for input in &s.inputs {
        assert!(!input.required, "input '{}' should be optional", input.name);
    }
}

#[test]
fn deserialize_update_local_state_params_with_null_clears_value() {
    let patch = parse_update(json!({
        "encryptionKey": null,
        "onboardingTasks": null,
        "keyringConsent": null
    }))
    .unwrap();
    assert_eq!(patch.encryption_key, Some(None));
    assert!(matches!(patch.onboarding_tasks, Some(None)));
    assert_eq!(patch.keyring_consent, Some(None));
}

#[test]
fn update_local_state_patch_preserves_camel_case_tristate_and_nested_defaults() {
    let missing = parse_update(json!({})).unwrap();
    assert_eq!(missing.encryption_key, None);
    assert!(missing.onboarding_tasks.is_none());
    assert_eq!(missing.keyring_consent, None);

    for expected in ["", "ordinary", "  keep whitespace  "] {
        let patch = parse_update(json!({"encryptionKey": expected})).unwrap();
        assert_eq!(
            patch.encryption_key.as_ref().map(|value| value.as_deref()),
            Some(Some(expected)),
            "encryptionKey remains verbatim"
        );
    }

    let patch = parse_update(json!({
        "encryptionKey": "  keep whitespace  ",
        "onboardingTasks": {
            "accessibilityPermissionGranted": false,
            "enabledTools": [],
            "connectedSources": [],
            "updatedAtMs": null,
            "unknownNestedField": true
        },
        "keyringConsent": {
            "storageMode": "",
            "consentedAtMs": 17,
            "unknownNestedField": true
        },
        "unknownOuterField": {"ignored": true},
        "encryption_key": "snake_case_is_not_an_alias"
    }))
    .unwrap();

    let snake_case_only = parse_update(json!({
        "encryption_key": "ignored",
        "onboarding_tasks": {"accessibility_permission_granted": true},
        "keyring_consent": {"storage_mode": "ignored"}
    }))
    .unwrap();
    assert!(snake_case_only.encryption_key.is_none());
    assert!(snake_case_only.onboarding_tasks.is_none());
    assert!(snake_case_only.keyring_consent.is_none());

    assert_eq!(
        patch.encryption_key.as_ref().map(|value| value.as_deref()),
        Some(Some("  keep whitespace  "))
    );
    let tasks = patch.onboarding_tasks.as_ref().unwrap().as_ref().unwrap();
    assert!(!tasks.accessibility_permission_granted);
    assert!(tasks.enabled_tools.is_empty());
    assert!(tasks.connected_sources.is_empty());
    assert_eq!(tasks.updated_at_ms, None);
    let consent = patch.keyring_consent.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(consent.storage_mode, "");
    assert_eq!(consent.consented_at_ms, Some(17));

    let defaults = parse_update(json!({
        "onboardingTasks": {},
        "keyringConsent": {}
    }))
    .unwrap();
    let tasks = defaults
        .onboarding_tasks
        .as_ref()
        .unwrap()
        .as_ref()
        .unwrap();
    assert!(!tasks.accessibility_permission_granted);
    assert!(tasks.enabled_tools.is_empty());
    assert!(tasks.connected_sources.is_empty());
    assert_eq!(tasks.updated_at_ms, None);
    let consent = defaults.keyring_consent.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(consent.storage_mode, "");
    assert_eq!(consent.consented_at_ms, None);
}

#[test]
fn update_local_state_patch_rejects_wrong_nested_and_outer_types() {
    for value in [
        serde_json::json!({"encryptionKey": false}),
        serde_json::json!({"onboardingTasks": "not-an-object"}),
        serde_json::json!({"keyringConsent": {"consentedAtMs": "17"}}),
    ] {
        let error = parse_update(value).expect_err("malformed patch types are rejected");
        assert!(
            error.starts_with("invalid params:"),
            "invalid update input keeps its parse error prefix: {error}"
        );
    }
}

#[test]
fn update_local_state_patch_preserves_array_to_struct_defaults() {
    let patch = parse_update(json!({
        "onboardingTasks": [],
        "keyringConsent": []
    }))
    .unwrap();
    let tasks = patch.onboarding_tasks.as_ref().unwrap().as_ref().unwrap();
    assert!(!tasks.accessibility_permission_granted);
    assert!(tasks.enabled_tools.is_empty());
    assert!(tasks.connected_sources.is_empty());
    assert_eq!(tasks.updated_at_ms, None);
    let consent = patch.keyring_consent.as_ref().unwrap().as_ref().unwrap();
    assert_eq!(consent.storage_mode, "");
    assert_eq!(consent.consented_at_ms, None);
}

async fn runtime_app_state_entries() -> Vec<crate::core::all::ControllerEntry> {
    use crate::core::runtime::{CoreContext, DomainSet};

    CoreContext::scope(CoreContext::for_test(DomainSet::full(), None), async {
        crate::core::all::all_registered_controllers()
            .into_iter()
            .filter(|entry| entry.schema.namespace == "app_state")
            .collect()
    })
    .await
}

#[tokio::test]
async fn update_local_state_rejects_malformed_params_before_loading_config() {
    use crate::config::test_env::EnvVarGuard;
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};

    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("config.toml")).unwrap();
    let _env = EnvVarGuard::workspace_async(tmp.path()).await;
    let entries = runtime_app_state_entries().await;
    let update = entries
        .iter()
        .find(|entry| entry.schema.method_name() == "app_state.update_local_state")
        .expect("runtime app_state.update_local_state entry");

    let error = update
        .invoke(
            DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None)),
            Map::from_iter([("encryptionKey".into(), json!(false))]),
        )
        .await
        .expect_err("malformed parameters stop before app-state persistence loads config");
    assert!(
        error.starts_with("invalid params:"),
        "update parser error takes precedence over config loading: {error}"
    );
    assert!(
        !error.contains("config.toml"),
        "the operation config loader was not reached: {error}"
    );
}

fn parse_update(value: serde_json::Value) -> Result<StoredAppStatePatch, String> {
    parse_update_local_state(
        value
            .as_object()
            .expect("test patch is a JSON object")
            .clone(),
    )
}
