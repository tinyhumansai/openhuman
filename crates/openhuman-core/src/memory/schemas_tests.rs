use super::*;
use crate::config::Config;
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DomainSet;
use crate::memory::test_fixtures::{bind_reference, config_in};
use serde_json::{json, Map, Value};

/// Every method of the spec's RPC table (`docs/specs/memory-v2.md`), exactly.
const SPEC_METHODS: [&str; 20] = [
    "openhuman.memory_engines_list",
    "openhuman.memory_engine_get",
    "openhuman.memory_engine_set",
    "openhuman.memory_recall",
    "openhuman.memory_fetch",
    "openhuman.memory_learn",
    "openhuman.memory_forget",
    "openhuman.memory_items_list",
    "openhuman.memory_conversations_get",
    "openhuman.memory_conversations_set",
    "openhuman.memory_sources_list",
    "openhuman.memory_sources_add",
    "openhuman.memory_sources_remove",
    "openhuman.memory_sources_sync",
    "openhuman.memory_context_get",
    "openhuman.memory_context_refresh",
    "openhuman.memory_context_set",
    "openhuman.memory_import_scan",
    "openhuman.memory_import_start",
    "openhuman.memory_import_status",
];

fn object(value: Value) -> Map<String, Value> {
    match value {
        Value::Object(map) => map,
        other => panic!("not an object: {other}"),
    }
}

/// Runs `function`'s handler against `config` (as the embedder-supplied
/// config, so no on-disk config is read).
async fn call(config: &Config, function: &str, params: Value) -> Result<Value, String> {
    let handler = handler_for(function);
    let ctx = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    CoreContext::scope(ctx, async move { handler(object(params)).await }).await
}

#[test]
fn every_spec_method_is_registered_with_its_exact_name() {
    let controllers = all_registered_controllers();
    let names: Vec<String> = controllers.iter().map(|c| c.rpc_method_name()).collect();
    assert_eq!(names, SPEC_METHODS, "registered methods, in spec order");
    assert_eq!(all_controller_schemas().len(), SPEC_METHODS.len());
    for controller in &controllers {
        assert_eq!(controller.schema.namespace, "memory");
        assert!(!controller.schema.description.is_empty());
        assert!(!controller.schema.outputs.is_empty());
    }
}

#[test]
fn unknown_function_gets_the_placeholder_schema() {
    let unknown = schema("no_such_function");
    assert_eq!(unknown.function, "unknown");
    assert!(!FUNCTIONS.contains(&"unknown"));
}

#[test]
fn required_inputs_match_the_spec() {
    let required = |function: &str| -> Vec<&'static str> {
        schema(function)
            .inputs
            .iter()
            .filter(|input| input.required)
            .map(|input| input.name)
            .collect()
    };
    let optional = |function: &str| -> Vec<&'static str> {
        schema(function)
            .inputs
            .iter()
            .filter(|input| !input.required)
            .map(|input| input.name)
            .collect()
    };
    assert_eq!(required("engine_set"), ["engine"]);
    assert_eq!(optional("engine_set"), ["endpoint", "api_key"]);
    assert_eq!(required("recall"), ["question"]);
    assert_eq!(required("fetch"), ["query"]);
    assert_eq!(required("learn"), ["text"]);
    assert_eq!(required("forget"), ["ids"]);
    assert_eq!(required("sources_add"), ["kind", "target"]);
    assert_eq!(required("sources_remove"), ["id"]);
    assert_eq!(optional("sources_remove"), ["forget_items"]);
    assert_eq!(optional("sources_sync"), ["id"]);
    assert_eq!(required("import_start"), ["consent"]);
    assert_eq!(
        optional("context_set"),
        ["enabled", "interval_mins", "budget_tokens"]
    );
    assert_eq!(
        optional("conversations_set"),
        ["enabled", "batch_turns", "idle_secs"]
    );
    for empty in [
        "engines_list",
        "engine_get",
        "conversations_get",
        "sources_list",
        "context_get",
        "context_refresh",
        "import_scan",
        "import_status",
    ] {
        assert!(schema(empty).inputs.is_empty(), "{empty} takes no params");
    }
}

#[tokio::test]
async fn invalid_params_are_rejected_as_invalid_request() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    for (function, params) in [
        ("engine_set", json!({})),
        ("recall", json!({})),
        ("recall", json!({"question": 7})),
        ("fetch", json!({"mode": "keyword"})),
        ("learn", json!({"confidence": 0.5})),
        ("forget", json!({"ids": "not-a-list"})),
        ("sources_add", json!({"kind": "folder"})),
        ("sources_remove", json!({})),
        ("conversations_set", json!({"batch_turns": "many"})),
        ("context_set", json!({"enabled": "yes"})),
        ("import_start", json!({"consent": "true"})),
    ] {
        let error = call(&config, function, params.clone())
            .await
            .expect_err(&format!("{function} {params}"));
        assert!(error.contains("INVALID_REQUEST"), "{function}: {error}");
    }
}

#[tokio::test]
async fn memory_off_surfaces_the_memory_off_code() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    for (function, params) in [
        ("recall", json!({"question": "q"})),
        ("fetch", json!({"query": "q"})),
        ("learn", json!({"text": "t"})),
        ("forget", json!({"ids": ["a"]})),
        ("items_list", json!({})),
        ("context_refresh", json!({})),
        ("sources_sync", json!({})),
    ] {
        let error = call(&config, function, params).await.unwrap_err();
        assert!(error.contains("MEMORY_OFF"), "{function}: {error}");
    }
    let engine = call(&config, "engine_get", json!({})).await.unwrap();
    let engine = engine.get("result").unwrap_or(&engine);
    assert_eq!(engine["status"], "off");
}

#[tokio::test]
async fn read_handlers_answer_over_a_bound_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);

    let learned = call(
        &config,
        "learn",
        json!({"text": "Likes oolong", "kind": "preference"}),
    )
    .await
    .unwrap();
    let learned = learned.get("result").unwrap_or(&learned).clone();
    let id = learned["id"].as_str().expect("id").to_string();

    let listed = call(&config, "items_list", json!({"limit": 5}))
        .await
        .unwrap();
    let listed = listed.get("result").unwrap_or(&listed);
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);

    let recalled = call(&config, "recall", json!({"question": "oolong"}))
        .await
        .unwrap();
    let recalled = recalled.get("result").unwrap_or(&recalled);
    assert!(!recalled["citations"].as_array().unwrap().is_empty());

    let fetched = call(&config, "fetch", json!({"query": "oolong"}))
        .await
        .unwrap();
    let fetched = fetched.get("result").unwrap_or(&fetched);
    assert_eq!(fetched["hits"].as_array().unwrap().len(), 1);

    let engines = call(&config, "engines_list", json!({})).await.unwrap();
    let engines = engines.get("result").unwrap_or(&engines);
    assert_eq!(engines["active"], "reference");

    let engine = call(&config, "engine_get", json!({})).await.unwrap();
    let engine = engine.get("result").unwrap_or(&engine);
    assert_eq!(engine["status"], "ok");

    let forgotten = call(&config, "forget", json!({"ids": [id]})).await.unwrap();
    let forgotten = forgotten.get("result").unwrap_or(&forgotten);
    assert_eq!(forgotten["forgotten"], 1);
}

#[tokio::test]
async fn settings_handlers_validate_persist_and_report() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);

    let conversations = call(
        &config,
        "conversations_set",
        json!({"enabled": false, "batch_turns": 3, "idle_secs": 90}),
    )
    .await
    .unwrap();
    let conversations = conversations.get("result").unwrap_or(&conversations);
    assert_eq!(conversations["enabled"], false);
    assert_eq!(conversations["batch_turns"], 3);
    assert_eq!(conversations["idle_secs"], 90);
    let saved = std::fs::read_to_string(&config.config_path).expect("config persisted");
    assert!(saved.contains("batch_turns"), "{saved}");

    let bad = call(&config, "conversations_set", json!({"batch_turns": 0}))
        .await
        .unwrap_err();
    assert!(bad.contains("INVALID_REQUEST"));

    let context = call(
        &config,
        "context_set",
        json!({"interval_mins": 30, "budget_tokens": 500}),
    )
    .await
    .unwrap();
    let context = context.get("result").unwrap_or(&context);
    assert_eq!(context["interval_mins"], 30);
    assert_eq!(context["budget_tokens"], 500);
    let jobs = crate::cron::list_jobs(&config).unwrap();
    assert!(
        jobs.iter()
            .any(|job| job.command == "system:memory_context_refresh"),
        "context_set (re)seeds the cron jobs"
    );

    let got = call(&config, "context_get", json!({})).await.unwrap();
    assert!(got.get("result").unwrap_or(&got).get("markdown").is_some());
    let convo = call(&config, "conversations_get", json!({})).await.unwrap();
    assert!(convo
        .get("result")
        .unwrap_or(&convo)
        .get("recent")
        .is_some());
}

#[tokio::test]
async fn engine_set_persists_the_selection_and_rejects_unknown_engines() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let error = call(&config, "engine_set", json!({"engine": "nope"}))
        .await
        .unwrap_err();
    assert!(error.contains("INVALID_REQUEST"));

    let view = call(
        &config,
        "engine_set",
        json!({
            "engine": "cortexdb",
            "endpoint": "https://cortex.example.test",
            "api_key": "cdb-test-key",
        }),
    )
    .await
    .unwrap();
    let view = view.get("result").unwrap_or(&view);
    assert_eq!(view["engine"], "cortexdb");
    assert_eq!(view["endpoint"], "https://cortex.example.test");
    let saved = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(saved.contains("cortexdb"), "{saved}");
    assert!(
        !saved.contains("cdb-test-key"),
        "the key never lands in config"
    );
}

#[tokio::test]
async fn source_handlers_add_remove_and_sync() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let folder = tmp.path().join("docs");
    std::fs::create_dir_all(&folder).unwrap();

    let added = call(
        &config,
        "sources_add",
        json!({"kind": "folder", "target": folder.display().to_string(), "label": "Docs"}),
    )
    .await
    .unwrap();
    let added = added.get("result").unwrap_or(&added);
    assert_eq!(added["source"]["kind"], "folder");
    assert_eq!(added["source"]["label"], "Docs");
    assert_eq!(added["source"]["status"], "idle");
    let saved = std::fs::read_to_string(&config.config_path).unwrap();
    assert!(saved.contains("Docs"), "{saved}");

    let bad_kind = call(
        &config,
        "sources_add",
        json!({"kind": "twitter", "target": "x"}),
    )
    .await
    .unwrap_err();
    assert!(bad_kind.contains("INVALID_REQUEST"));

    let none = call(&config, "sources_list", json!({})).await.unwrap();
    assert!(none.get("result").unwrap_or(&none)["sources"].is_array());

    let missing = call(&config, "sources_remove", json!({"id": "src-missing"}))
        .await
        .unwrap();
    assert_eq!(missing.get("result").unwrap_or(&missing)["removed"], false);

    let unknown_sync = {
        bind_reference(&config);
        call(&config, "sources_sync", json!({"id": "src-missing"}))
            .await
            .unwrap_err()
    };
    assert!(unknown_sync.contains("INVALID_REQUEST"));
}

#[tokio::test]
async fn import_handlers_report_scan_and_refuse_without_consent() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let scan = call(&config, "import_scan", json!({})).await.unwrap();
    assert_eq!(scan.get("result").unwrap_or(&scan)["found"], false);

    let status = call(&config, "import_status", json!({})).await.unwrap();
    assert_eq!(
        status.get("result").unwrap_or(&status)["state"]["phase"],
        "idle"
    );

    let refused = call(&config, "import_start", json!({"consent": false}))
        .await
        .unwrap_err();
    assert!(refused.contains("INVALID_REQUEST"));
    let no_param = call(&config, "import_start", json!({})).await.unwrap_err();
    assert!(no_param.contains("INVALID_REQUEST"));
}
