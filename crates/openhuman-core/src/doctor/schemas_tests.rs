use super::*;
use crate::core::all::assert_schema_controller_parity;

#[test]
fn legacy_handlers_keep_their_identity_and_order() {
    use crate::core::all::ControllerHandler;

    let handlers: [(&str, ControllerHandler); 2] =
        [("report", handle_report), ("models", handle_models)];
    let controllers = all_registered_controllers();
    assert_eq!(controllers.len(), handlers.len());
    assert_eq!(all_controller_schemas().len(), handlers.len());
    assert_schema_controller_parity(&all_controller_schemas(), &controllers, "report");

    for ((function, expected_handler), controller) in handlers.iter().zip(&controllers) {
        assert_eq!(controller.schema.namespace, "doctor");
        assert_eq!(controller.schema.function, *function);
        assert!(std::ptr::fn_addr_eq(controller.handler, *expected_handler));
    }
}

#[test]
fn report_schema() {
    let s = schemas("report");
    assert_eq!(s.namespace, "doctor");
    assert_eq!(s.function, "report");
    assert_eq!(
        s.inputs.iter().map(|field| field.name).collect::<Vec<_>>(),
        ["tier", "only", "skip", "profile"]
    );
    assert!(s.inputs.iter().all(|field| !field.required));
}

#[test]
fn models_schema_has_optional_use_cache() {
    let s = schemas("models");
    assert_eq!(s.function, "models");
    let use_cache = s.inputs.iter().find(|f| f.name == "use_cache");
    assert!(use_cache.is_some_and(|f| !f.required));
}

#[test]
fn optional_cache_flag_preserves_missing_null_and_boolean_values() {
    for (case, input, expected) in [
        ("missing", None, true),
        ("null", Some(Value::Null), true),
        ("true", Some(Value::Bool(true)), true),
        ("false", Some(Value::Bool(false)), false),
    ] {
        let mut params = Map::new();
        if let Some(input) = input {
            params.insert("use_cache".into(), input);
        }
        assert_eq!(parse_models(params).unwrap(), expected, "{case}");
    }
}

#[test]
fn model_parser_rejects_non_boolean_cache_flags() {
    for value in [
        Value::String("true".into()),
        Value::Number(1.into()),
        Value::Array(vec![]),
        Value::Object(Map::new()),
    ] {
        let params = Map::from_iter([("use_cache".into(), value)]);
        let error = parse_models(params).unwrap_err();
        assert!(
            error.starts_with("invalid 'use_cache':"),
            "wrong type error keeps its field prefix: {error}"
        );
    }
}

#[test]
fn report_parser_applies_defaults_and_ignores_unknown_fields() {
    let params = Map::from_iter([
        ("use_cache".into(), Value::String("wrong type".into())),
        (
            "unknown_nested".into(),
            serde_json::json!({"ignored":[1, false]}),
        ),
    ]);
    let options = parse_report(params).expect("unknown fields are ignored");
    assert_eq!(options.tier, crate::doctor::DoctorTier::Connect);
    assert!(options.only.is_empty());
    assert!(options.skip.is_empty());
    assert!(options.profile.is_none());
}

#[test]
fn report_parser_preserves_typed_options_and_null_defaults() {
    let params = Map::from_iter([
        ("tier".into(), serde_json::json!("live")),
        ("only".into(), serde_json::json!(["memory", "config"])),
        ("skip".into(), Value::Null),
        ("profile".into(), Value::Null),
    ]);
    let options = parse_report(params).unwrap();
    assert_eq!(options.tier, crate::doctor::DoctorTier::Live);
    assert_eq!(options.only, ["memory", "config"]);
    assert!(options.skip.is_empty());
    assert!(options.profile.is_none());
}

#[test]
fn report_parser_rejects_invalid_declared_options() {
    for (key, value) in [
        ("tier", serde_json::json!("unknown")),
        ("only", serde_json::json!("memory")),
        ("skip", serde_json::json!([false])),
        ("profile", serde_json::json!(false)),
    ] {
        let error = parse_report(Map::from_iter([(key.into(), value)])).unwrap_err();
        assert!(error.starts_with(&format!("invalid '{key}':")), "{error}");
    }
}

async fn runtime_doctor_entries() -> Vec<crate::core::all::ControllerEntry> {
    use crate::core::runtime::{CoreContext, DomainSet};

    CoreContext::scope(CoreContext::for_test(DomainSet::full(), None), async {
        crate::core::all::all_registered_controllers()
            .into_iter()
            .filter(|entry| entry.schema.namespace == "doctor")
            .collect()
    })
    .await
}

#[tokio::test]
async fn report_options_parse_before_dispatch_config_and_models_gate_before_parsing() {
    use crate::config::test_env::EnvVarGuard;
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};
    use serde_json::json;

    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("config.toml")).unwrap();
    let _env = EnvVarGuard::workspace_async(tmp.path()).await;
    let entries = runtime_doctor_entries().await;
    assert_eq!(entries.len(), 2, "both runtime Doctor entries exist");
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));

    let report = entries
        .iter()
        .find(|entry| entry.schema.method_name() == "doctor.report")
        .expect("runtime doctor.report entry");
    let report_error = report
        .invoke(
            ctx.clone(),
            Map::from_iter([("tier".into(), json!("invalid"))]),
        )
        .await
        .expect_err("invalid options fail before the report resolves config");
    assert!(
        report_error.starts_with("invalid 'tier':"),
        "{report_error}"
    );

    let models = entries
        .iter()
        .find(|entry| entry.schema.method_name() == "doctor.models")
        .expect("runtime doctor.models entry");
    let malformed = Map::from_iter([("use_cache".into(), json!("false"))]);
    let gate_error = models
        .invoke(ctx, malformed.clone())
        .await
        .expect_err("config gate runs before model parameter parsing");
    assert_eq!(
        gate_error,
        "The selected profile config is missing or invalid."
    );
    assert!(!gate_error.starts_with("invalid 'use_cache':"));

    let valid_ctx = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        crate::config::Config::default(),
    ));
    let parse_error = models
        .invoke(valid_ctx, malformed)
        .await
        .expect_err("invalid cache flag stops before the model probe runner");
    assert!(
        parse_error.starts_with("invalid 'use_cache':"),
        "models parser error: {parse_error}"
    );
}

pub(crate) fn doctor_parameter_contracts() -> Vec<crate::core::all::ControllerParamContract> {
    use serde_json::json;

    vec![
        crate::core::all::ControllerParamContract {
            method: "doctor.report",
            required: &[],
            optional: &["tier", "only", "skip", "profile"],
            examples: vec![
                json!({}),
                json!({"tier":"static"}),
                json!({"only":["config"]}),
                json!({"skip":["memory"]}),
                json!({"profile":"work"}),
            ],
        },
        crate::core::all::ControllerParamContract {
            method: "doctor.models",
            required: &[],
            optional: &["use_cache"],
            examples: vec![
                json!({}),
                json!({"use_cache":null}),
                json!({"use_cache":true}),
                json!({"use_cache":false}),
            ],
        },
    ]
}
