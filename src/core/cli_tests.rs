use super::{grouped_schemas, load_dotenv_for_cli, parse_function_params, parse_input_value};
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};
use std::sync::{Mutex, OnceLock};
use tempfile::tempdir;

static CLI_ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    CLI_ENV_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn grouped_schemas_contains_migrated_namespaces() {
    let grouped = grouped_schemas();
    assert!(grouped.contains_key("health"));
    assert!(grouped.contains_key("doctor"));
    assert!(grouped.contains_key("encrypt"));
    assert!(grouped.contains_key("decrypt"));
    assert!(grouped.contains_key("autocomplete"));
    assert!(grouped.contains_key("config"));
    assert!(grouped.contains_key("auth"));
    assert!(grouped.contains_key("service"));
    assert!(grouped.contains_key("migrate"));
    assert!(grouped.contains_key("local_ai"));
}

#[test]
fn parse_function_params_rejects_unknown_param() {
    let schema = ControllerSchema {
        namespace: "test",
        function: "echo",
        description: "test schema",
        inputs: vec![FieldSchema {
            name: "message",
            ty: TypeSchema::String,
            required: true,
            comment: "message text",
        }],
        outputs: vec![FieldSchema {
            name: "result",
            ty: TypeSchema::String,
            required: true,
            comment: "echo response",
        }],
    };
    let args = vec!["--unknown".to_string(), "value".to_string()];
    let err = parse_function_params(&schema, &args).expect_err("unknown param should fail");
    assert!(err.contains("unknown param"));
}

#[test]
fn parse_input_value_rejects_invalid_bool() {
    let err =
        parse_input_value(&TypeSchema::Bool, "not-a-bool").expect_err("invalid bool should fail");
    assert!(err.contains("expected bool"));
}

#[test]
fn load_dotenv_for_cli_reads_cwd_dotenv_without_overwriting_existing_env() {
    let _guard = env_lock();
    let tmp = tempdir().expect("tempdir");
    let env_path = tmp.path().join(".env");
    std::fs::write(
        &env_path,
        "BACKEND_URL=https://staging-api.example.test\nOPENHUMAN_APP_ENV=staging\n",
    )
    .expect("write .env");

    let original_dir = std::env::current_dir().expect("current dir");
    let prior_backend = std::env::var("BACKEND_URL").ok();
    let prior_app_env = std::env::var("OPENHUMAN_APP_ENV").ok();
    let prior_dotenv_path = std::env::var("OPENHUMAN_DOTENV_PATH").ok();

    unsafe {
        std::env::remove_var("BACKEND_URL");
        std::env::set_var("OPENHUMAN_APP_ENV", "production");
        std::env::remove_var("OPENHUMAN_DOTENV_PATH");
    }
    std::env::set_current_dir(tmp.path()).expect("set current dir");

    let result = load_dotenv_for_cli();

    let loaded_backend = std::env::var("BACKEND_URL").ok();
    let loaded_app_env = std::env::var("OPENHUMAN_APP_ENV").ok();

    std::env::set_current_dir(&original_dir).expect("restore current dir");
    unsafe {
        match prior_backend {
            Some(value) => std::env::set_var("BACKEND_URL", value),
            None => std::env::remove_var("BACKEND_URL"),
        }
        match prior_app_env {
            Some(value) => std::env::set_var("OPENHUMAN_APP_ENV", value),
            None => std::env::remove_var("OPENHUMAN_APP_ENV"),
        }
        match prior_dotenv_path {
            Some(value) => std::env::set_var("OPENHUMAN_DOTENV_PATH", value),
            None => std::env::remove_var("OPENHUMAN_DOTENV_PATH"),
        }
    }

    result.expect("dotenv load should succeed");
    assert_eq!(
        loaded_backend.as_deref(),
        Some("https://staging-api.example.test")
    );
    assert_eq!(loaded_app_env.as_deref(), Some("production"));
}
