use super::*;
use crate::core::all::{
    assert_schema_controller_parity, ControllerHandler, ControllerParamContract,
};
use serde_json::json;

#[test]
fn legacy_handlers_keep_their_identity_and_order() {
    let expected: [(&str, ControllerHandler); 4] = [
        ("create_pairing", handle_create_pairing),
        ("import_legacy", handle_import_legacy),
        ("list", handle_list),
        ("revoke", handle_revoke),
    ];
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), expected.len());
    for ((function, handler), controller) in expected.iter().zip(&controllers) {
        assert_eq!(controller.schema.namespace, "devices");
        assert_eq!(controller.schema.function, *function);
        assert!(std::ptr::fn_addr_eq(controller.handler, *handler));
    }
}

pub(super) fn parameter_contracts() -> Vec<ControllerParamContract> {
    vec![
        ControllerParamContract {
            method: "devices.create_pairing",
            required: &[],
            optional: &["label"],
            examples: vec![
                json!({}),
                json!({"label": null}),
                json!({"label": ""}),
                json!({"label": "  preserve label  "}),
            ],
        },
        ControllerParamContract {
            method: "devices.list",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "devices.import_legacy",
            required: &[],
            optional: &[],
            examples: vec![json!({})],
        },
        ControllerParamContract {
            method: "devices.revoke",
            required: &["channel_id"],
            optional: &[],
            examples: vec![
                json!({"channel_id": ""}),
                json!({"channel_id": "  raw channel id  "}),
            ],
        },
    ]
}

#[test]
fn device_schema_catalog_matches_legacy_handler_order() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "create_pairing",
    );
}

#[test]
fn create_pairing_parser_preserves_optional_labels_and_strict_type_errors() {
    for (params, expected) in [
        (Map::new(), None),
        (Map::from_iter([("label".into(), json!(null))]), None),
        (
            Map::from_iter([("label".into(), json!(""))]),
            Some(String::new()),
        ),
        (
            Map::from_iter([("label".into(), json!("  preserve label  "))]),
            Some("  preserve label  ".to_owned()),
        ),
    ] {
        assert_eq!(parse_create_pairing(params).unwrap(), expected);
    }
    assert_eq!(
        parse_create_pairing(Map::from_iter([
            ("label".into(), json!("  parser label  ")),
            ("unknown_scalar".into(), json!(true)),
            ("unknown_nested".into(), json!({"ignored": [1, null]})),
        ]))
        .unwrap(),
        Some("  parser label  ".to_owned()),
        "the actual parser keeps its value while ignoring unknown fields"
    );

    for (value, kind) in [
        (json!(true), "bool"),
        (json!(7), "number"),
        (json!([]), "array"),
        (json!({}), "object"),
    ] {
        assert_eq!(
            parse_create_pairing(Map::from_iter([("label".into(), value)])),
            Err(format!("invalid 'label': expected string, got {kind}"))
        );
    }
}

#[test]
fn list_parser_ignores_the_entire_parameter_map() {
    parse_list(Map::from_iter([
        ("unexpected".into(), json!({"nested": [true, null]})),
        ("channel_id".into(), json!(false)),
    ]))
    .expect("devices.list ignores all parameters");
}

#[test]
fn revoke_parser_requires_and_preserves_a_string_channel_id() {
    assert_eq!(
        parse_revoke(Map::new()).unwrap_err(),
        "missing required param 'channel_id'"
    );
    for (value, serde_error) in [
        (json!(null), "invalid type: null, expected a string"),
        (
            json!(true),
            "invalid type: boolean `true`, expected a string",
        ),
        (json!(7), "invalid type: integer `7`, expected a string"),
        (json!([]), "invalid type: sequence, expected a string"),
        (json!({}), "invalid type: map, expected a string"),
    ] {
        assert_eq!(
            parse_revoke(Map::from_iter([("channel_id".into(), value)])),
            Err(format!("invalid 'channel_id': {serde_error}"))
        );
    }
    for value in ["", "ordinary-channel", "  raw channel id  "] {
        assert_eq!(
            parse_revoke(Map::from_iter([("channel_id".into(), json!(value))])).unwrap(),
            value
        );
    }
    assert_eq!(
        parse_revoke(Map::from_iter([
            ("channel_id".into(), json!("  parser channel id  ")),
            ("extra_scalar".into(), json!(true)),
            ("extra_nested".into(), json!({"ignored": [false, null]})),
        ]))
        .unwrap(),
        "  parser channel id  ",
        "the actual parser preserves the channel id while ignoring extras"
    );
}

async fn runtime_device_entries() -> Vec<crate::core::all::ControllerEntry> {
    use crate::core::runtime::{CoreContext, DomainSet};

    CoreContext::scope(CoreContext::for_test(DomainSet::full(), None), async {
        crate::core::all::all_registered_controllers()
            .into_iter()
            .filter(|entry| entry.schema.namespace == "devices")
            .collect()
    })
    .await
}

#[tokio::test]
async fn device_handlers_load_config_before_parsing_or_running() {
    use crate::config::test_env::EnvVarGuard;
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};

    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("config.toml")).unwrap();
    let _env = EnvVarGuard::workspace_async(tmp.path()).await;
    let entries = runtime_device_entries().await;
    assert_eq!(entries.len(), 4, "all runtime device entries are present");
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));

    for (method, params) in [
        (
            "devices.create_pairing",
            Map::from_iter([("label".into(), json!(false))]),
        ),
        (
            "devices.list",
            Map::from_iter([("unexpected".into(), json!({"ignored": true}))]),
        ),
        ("devices.import_legacy", Map::new()),
        (
            "devices.revoke",
            Map::from_iter([("channel_id".into(), json!(false))]),
        ),
    ] {
        let entry = entries
            .iter()
            .find(|entry| entry.schema.method_name() == method)
            .expect("runtime device method is registered");
        let error = entry
            .invoke(ctx.clone(), params)
            .await
            .expect_err("invalid config prevents parser/runner effects");
        assert!(
            error.contains("is a directory") || error.contains("not a file"),
            "{method} config load precedes its parser and operation: {error}"
        );
        assert!(!error.starts_with("invalid 'label':"));
        assert!(!error.starts_with("invalid 'channel_id':"));
    }
}
