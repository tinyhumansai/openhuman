use super::*;
use crate::core::all::{
    assert_schema_controller_parity, expect_schema_fields, expect_schema_identity,
    expect_schema_output_names, expect_schema_required_inputs,
};
// Additional names from the extracted schema handler modules.
use super::handlers::{read_optional_u64, read_required};
use crate::core::TypeSchema;
use serde_json::{json, Map, Value};
use tempfile::TempDir;

async fn cron_test_runtime(config: crate::config::Config) -> crate::core::runtime::CoreRuntime {
    use crate::core::runtime::{CoreBuilder, DomainSet, ServiceSet};
    use crate::core::types::HostKind;

    CoreBuilder::new(HostKind::Cli)
        .domains(DomainSet::full())
        .services(ServiceSet::none())
        .config(config)
        .build()
        .await
        .unwrap()
}

fn cron_test_config(tmp: &TempDir) -> crate::config::Config {
    let config = crate::config::Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Default::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

#[test]
fn cron_schema_wire_contracts_are_preserved() {
    assert_schema_controller_parity(
        &all_controller_schemas(),
        &all_registered_controllers(),
        "add",
    );
    let functions: Vec<_> = all_controller_schemas()
        .iter()
        .map(|schema| schema.function)
        .collect();
    assert_eq!(
        functions,
        ["add", "list", "update", "remove", "run", "runs"]
    );

    for (function, required) in [
        ("add", &["schedule"][..]),
        ("update", &["job_id", "patch"][..]),
        ("remove", &["job_id"][..]),
    ] {
        let schema = schemas(function);
        expect_schema_identity(&schema, "cron", function);
        expect_schema_fields(
            &schema,
            &required
                .iter()
                .map(|name| (*name, Some(true)))
                .collect::<Vec<_>>(),
        );
        if function == "update" {
            assert!(schema.inputs.iter().all(|field| field.required));
        }
    }
    expect_schema_required_inputs(&schemas("add"), &["schedule"]);
    let list = schemas("list");
    expect_schema_identity(&list, "cron", "list");
    assert!(list.inputs.is_empty());
    expect_schema_output_names(&list, &["jobs"]);
    let remove = schemas("remove");
    assert_eq!(remove.inputs.len(), 1);
    assert_eq!(remove.outputs[0].name, "result");
    assert_eq!(schemas("add").outputs[0].name, "job");
    expect_schema_fields(&schemas("runs"), &[("limit", Some(false))]);

    let s = schemas("run");
    // `cron_run` spawns the execution and returns immediately, so the declared
    // output is the enqueue acknowledgement — `job_id` plus a `status` that is
    // always "queued". `duration_ms` / `output` belong to the terminal record
    // in `cron_runs` and must not be promised here.
    let TypeSchema::Object { fields } = &s.outputs[0].ty else {
        panic!("expected object output type");
    };
    let names: Vec<_> = fields.iter().map(|f| f.name).collect();
    assert_eq!(names, vec!["job_id", "status"]);

    let status = fields.iter().find(|f| f.name == "status").unwrap();
    assert!(
        matches!(&status.ty, TypeSchema::Enum { variants } if variants.as_slice() == ["queued"]),
        "status must declare exactly the variant the handler emits, got {:?}",
        status.ty
    );
}

#[test]
fn schemas_run_description_points_at_cron_runs_for_the_terminal_record() {
    let s = schemas("run");
    assert!(
        s.description.contains("cron_runs"),
        "the description must tell a client where the terminal outcome lives, got: {}",
        s.description
    );
}

#[test]
fn schemas_runs_limit_is_optional() {
    let s = schemas("runs");
    let limit = s.inputs.iter().find(|f| f.name == "limit").unwrap();
    assert!(!limit.required);
}

// ── registry helpers ────────────────────────────────────────────

#[test]
fn schemas_add_requires_schedule_and_returns_job() {
    let s = schemas("add");
    assert_eq!(s.namespace, "cron");
    assert_eq!(s.function, "add");
    let required: Vec<_> = s
        .inputs
        .iter()
        .filter(|f| f.required)
        .map(|f| f.name)
        .collect();
    assert_eq!(required, vec!["schedule"]);
    assert_eq!(s.outputs[0].name, "job");
}

#[tokio::test]
async fn rpc_add_rejects_disallowed_target_even_with_request_origin_spoof() {
    let tmp = TempDir::new().unwrap();
    let mut config = cron_test_config(&tmp);
    config.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: "test-token".into(),
        chat_id: None,
        allowed_users: vec!["alice".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    let runtime = cron_test_runtime(config.clone()).await;
    let result = runtime
        .invoke(
            "openhuman.cron_add",
            json!({
                "job_type": "agent",
                "schedule": {"kind":"every", "every_ms":300000},
                "prompt": "check inbox",
                "session_target": "isolated",
                "delivery": {"mode":"announce", "channel":"telegram", "to":"42", "best_effort":true},
                "origin": {"kind":"channel", "channel":"telegram", "reply_target":"42", "history_key":"telegram_alice_42", "sender":"alice"}
            }),
        )
        .await;
    assert!(
        result.unwrap_err().contains("allowed"),
        "RPC-supplied origin must not bypass the configured recipient allowlist"
    );
    assert!(crate::cron::list_jobs(&config).unwrap().is_empty());
}

#[tokio::test]
async fn rpc_origin_delivery_accepts_web_origin_without_trusting_channel_origin() {
    let tmp = TempDir::new().unwrap();
    let config = cron_test_config(&tmp);
    let runtime = cron_test_runtime(config.clone()).await;
    let result = runtime
        .invoke(
            "openhuman.cron_add",
            json!({
                "job_type": "agent",
                "schedule": {"kind":"every", "every_ms":300000},
                "prompt": "check inbox",
                "session_target": "current",
                "delivery": {"mode":"origin"},
                "origin": {"kind":"web", "thread_id":"thread-1"}
            }),
        )
        .await
        .expect("a caller-provided Web origin is structurally valid for RPC origin delivery");
    assert_eq!(result["logs"][0], "cron job created");
    let jobs = crate::cron::list_jobs(&config).unwrap();
    assert_eq!(jobs.len(), 1);
    assert_eq!(
        jobs[0].origin,
        Some(tinyflows_bus::schedule::types::JobOrigin::Web {
            thread_id: "thread-1".into(),
            agent_id: None,
        })
    );
}

#[tokio::test]
async fn rpc_origin_delivery_rejects_channel_origin_outside_allowed_users() {
    let tmp = TempDir::new().unwrap();
    let mut config = cron_test_config(&tmp);
    config.channels_config.telegram = Some(crate::config::TelegramConfig {
        bot_token: "test-token".into(),
        chat_id: None,
        allowed_users: vec!["alice".into()],
        stream_mode: Default::default(),
        draft_update_interval_ms: 1000,
        silent_streaming: true,
        mention_only: false,
    });
    let runtime = cron_test_runtime(config.clone()).await;
    let result = runtime
        .invoke(
            "openhuman.cron_add",
            json!({
                "job_type": "agent",
                "schedule": {"kind":"every", "every_ms":300000},
                "prompt": "check inbox",
                "session_target": "isolated",
                "delivery": {"mode":"origin"},
                "origin": {"kind":"channel", "channel":"telegram", "reply_target":"999", "history_key":"telegram-eve-999", "sender":"eve"}
            }),
        )
        .await
        .unwrap_err();
    assert!(result.contains("allowed_users"), "{result}");
    assert!(crate::cron::list_jobs(&config).unwrap().is_empty());
}

#[tokio::test]
async fn rpc_add_refuses_when_cron_is_disabled_without_creating_job() {
    let tmp = TempDir::new().unwrap();
    let mut config = cron_test_config(&tmp);
    config.cron.enabled = false;
    let runtime = cron_test_runtime(config.clone()).await;
    let result = runtime
        .invoke(
            "openhuman.cron_add",
            json!({
                "job_type": "shell",
                "schedule": {"kind":"every", "every_ms":300000},
                "command": "echo harmless"
            }),
        )
        .await;
    assert!(result.unwrap_err().contains("cron is disabled"));
    assert!(crate::cron::list_jobs(&config).unwrap().is_empty());
}

#[tokio::test]
async fn rpc_add_cannot_create_a_memory_system_flow_from_parameters() {
    let tmp = TempDir::new().unwrap();
    let mut config = cron_test_config(&tmp);
    config.cron.agent_jobs_only = true;
    let runtime = cron_test_runtime(config.clone()).await;
    let result = runtime
        .invoke(
            "openhuman.cron_add",
            json!({
                "job_type": "flow",
                "schedule": {"kind":"every", "every_ms":300000},
                "command": "system:memory_background"
            }),
        )
        .await
        .unwrap_err();

    assert!(result.contains("allowed enum variants"), "{result}");
    assert!(crate::cron::list_jobs(&config).unwrap().is_empty());
}

#[test]
fn all_controller_schemas_covers_every_supported_function() {
    let names: Vec<_> = all_controller_schemas()
        .into_iter()
        .map(|s| s.function)
        .collect();
    assert_eq!(
        names,
        vec!["add", "list", "update", "remove", "run", "runs"]
    );
}

// ── read_required ───────────────────────────────────────────────

#[test]
fn read_required_returns_value_for_present_key() {
    let mut params = Map::new();
    params.insert("job_id".into(), json!("abc"));
    let got: String = read_required(&params, "job_id").unwrap();
    assert_eq!(got, "abc");
}

#[test]
fn read_required_errors_when_key_missing() {
    let params = Map::new();
    let err = read_required::<String>(&params, "job_id").unwrap_err();
    assert!(err.contains("missing required param 'job_id'"));
}

#[test]
fn read_required_errors_when_deserialization_fails() {
    let mut params = Map::new();
    params.insert("job_id".into(), json!(42));
    let err = read_required::<String>(&params, "job_id").unwrap_err();
    assert!(err.contains("invalid 'job_id'"));
}

// ── read_optional_u64 ───────────────────────────────────────────

#[test]
fn read_optional_u64_handles_absent_null_values_and_errors() {
    assert_eq!(read_optional_u64(&Map::new(), "limit").unwrap(), None);
    let mut params = Map::new();
    params.insert("limit".into(), Value::Null);
    assert_eq!(read_optional_u64(&params, "limit").unwrap(), None);
    params.clear();
    params.insert("limit".into(), json!(42));
    assert_eq!(read_optional_u64(&params, "limit").unwrap(), Some(42));

    for (tag, value) in [
        ("negative", json!(-1)),
        ("string", json!("ten")),
        ("bool", json!(true)),
        ("array", json!([1, 2])),
        ("object", json!({"k": 1})),
    ] {
        let mut params = Map::new();
        params.insert("limit".into(), value);
        let err = read_optional_u64(&params, "limit").unwrap_err();
        assert!(
            err.contains("expected unsigned integer"),
            "tag={tag} err={err}"
        );
    }
}

// ── type_name ───────────────────────────────────────────────────
