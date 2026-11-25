use super::*;
use crate::core::all::{
    assert_schema_controller_parity, ControllerHandler, ControllerParamContract,
};

#[test]
fn legacy_registered_handlers_keep_their_function_identity() {
    let controllers = all_registered_controllers();
    let schemas = all_controller_schemas();
    let expected_handlers: [ControllerHandler; 2] = [handle_encrypt_secret, handle_decrypt_secret];
    assert_eq!(controllers.len(), 2);
    assert_eq!(controllers.len(), expected_handlers.len());
    assert_eq!(schemas.len(), 2);
    assert_eq!(
        schemas
            .iter()
            .map(|schema| schema.method_name())
            .collect::<Vec<_>>(),
        ["encrypt.secret", "decrypt.secret"]
    );
    assert_schema_controller_parity(&schemas, &controllers, "secret");
    for (controller, expected) in controllers.iter().zip(expected_handlers) {
        assert!(std::ptr::fn_addr_eq(controller.handler, expected));
    }
}

pub(super) fn parameter_contracts() -> Vec<ControllerParamContract> {
    use serde_json::json;

    vec![
        ControllerParamContract {
            method: "encrypt.secret",
            required: &["plaintext"],
            optional: &[],
            examples: vec![
                json!({"plaintext":""}),
                json!({"plaintext":"  keep whitespace  "}),
            ],
        },
        ControllerParamContract {
            method: "decrypt.secret",
            required: &["ciphertext"],
            optional: &[],
            examples: vec![
                json!({"ciphertext":""}),
                json!({"ciphertext":"  keep whitespace  "}),
            ],
        },
    ]
}

#[test]
fn secret_parsers_preserve_strings_and_required_value_errors() {
    use serde_json::json;

    type SecretParser = fn(Map<String, Value>) -> Result<String, String>;
    let parsers: [(&str, SecretParser); 2] = [
        ("plaintext", parse_encrypt_secret),
        ("ciphertext", parse_decrypt_secret),
    ];

    for (field, parse) in parsers {
        for expected in ["", "  keep whitespace  ", "secret payload"] {
            let params = Map::from_iter([(field.into(), json!(expected))]);
            assert_eq!(parse(params).unwrap(), expected, "{field} parser value");
        }

        assert_eq!(
            parse(Map::new()).unwrap_err(),
            format!("missing required param '{field}'")
        );
        for value in [json!(null), json!(true), json!(42)] {
            let params = Map::from_iter([(field.into(), value)]);
            let error = parse(params).expect_err("secret input must be a string");
            assert!(
                error.starts_with(&format!("invalid '{field}':")),
                "{field} invalid input error: {error}"
            );
        }
    }
}

async fn runtime_encryption_entries() -> Vec<crate::core::all::ControllerEntry> {
    use crate::core::runtime::{CoreContext, DomainSet};

    CoreContext::scope(CoreContext::for_test(DomainSet::full(), None), async {
        crate::core::all::all_registered_controllers()
            .into_iter()
            .filter(|entry| matches!(entry.schema.namespace, "encrypt" | "decrypt"))
            .collect()
    })
    .await
}

#[tokio::test]
async fn runtime_secret_entries_load_config_before_parsing_and_parse_after_success() {
    use crate::config::test_env::EnvVarGuard;
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};

    let tmp = tempfile::tempdir().unwrap();
    std::fs::create_dir(tmp.path().join("config.toml")).unwrap();
    let _env = EnvVarGuard::workspace_async(tmp.path()).await;

    let entries = runtime_encryption_entries().await;
    assert_eq!(entries.len(), 2, "both runtime encryption entries exist");
    let ctx = DispatchCtx::new(CoreContext::for_test(DomainSet::full(), None));
    for (method, field) in [
        ("encrypt.secret", "plaintext"),
        ("decrypt.secret", "ciphertext"),
    ] {
        let entry = entries
            .iter()
            .find(|entry| entry.schema.method_name() == method)
            .expect("runtime entry exists");
        let error = entry
            .invoke(ctx.clone(), Map::new())
            .await
            .expect_err("malformed params remain blocked by a config gate error");
        assert!(
            error.contains("is a directory") || error.contains("not a file"),
            "{method} must report the config gate error first: {error}"
        );
        assert!(
            !error.contains(&format!("missing required param '{field}'")),
            "{method} must not parse before its config gate: {error}"
        );
    }

    let valid_ctx = DispatchCtx::new(CoreContext::for_test_with_config(
        DomainSet::full(),
        crate::config::Config::default(),
    ));
    for (method, field) in [
        ("encrypt.secret", "plaintext"),
        ("decrypt.secret", "ciphertext"),
    ] {
        let entry = entries
            .iter()
            .find(|entry| entry.schema.method_name() == method)
            .expect("runtime entry exists");
        let error = entry
            .invoke(valid_ctx.clone(), Map::new())
            .await
            .expect_err("missing input is rejected before any encryption operation");
        assert_eq!(error, format!("missing required param '{field}'"));
    }
}
