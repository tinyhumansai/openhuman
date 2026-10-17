use super::*;
use crate::config::Config;
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DispatchCtx;
use crate::core::runtime::DomainSet;
use crate::memory::test_fixtures::{bind_reference, config_in};
use serde_json::{json, Map, Value};

/// Every method of the spec's RPC table (`docs/specs/memory-v2.md`), exactly.
const SPEC_METHODS: [&str; 35] = [
    "openhuman.memory_engines_list",
    "openhuman.memory_engine_get",
    "openhuman.memory_engine_set",
    "openhuman.memory_policy_get",
    "openhuman.memory_policy_set",
    "openhuman.memory_pack_preview",
    "openhuman.memory_recall",
    "openhuman.memory_fetch",
    "openhuman.memory_learn",
    "openhuman.memory_forget",
    "openhuman.memory_erase_all",
    "openhuman.memory_items_list",
    "openhuman.memory_explore",
    "openhuman.memory_items_get",
    "openhuman.memory_agents_list",
    "openhuman.memory_conversations_backfill_status",
    "openhuman.memory_conversations_backfill_start",
    "openhuman.memory_brain_sources",
    "openhuman.memory_brain_search",
    "openhuman.memory_brain_ingest",
    "openhuman.memory_brain_forget",
    "openhuman.memory_sources_list",
    "openhuman.memory_sources_add",
    "openhuman.memory_sources_remove",
    "openhuman.memory_sources_sync",
    "openhuman.memory_jobs_list",
    "openhuman.memory_jobs_run",
    "openhuman.memory_import_scan",
    "openhuman.memory_import_start",
    "openhuman.memory_import_status",
    "openhuman.memory_import_retry_failed",
    "openhuman.memory_migration_scan",
    "openhuman.memory_migration_start",
    "openhuman.memory_migration_status",
    "openhuman.memory_migration_retry",
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
    CoreContext::scope(ctx.clone(), async move {
        handler(DispatchCtx::new(ctx.clone()), object(params)).await
    })
    .await
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
    assert_eq!(optional("migration_start"), ["takeover"]);
    assert_eq!(optional("pack_preview"), ["query", "thread_id", "agent_id"]);
    assert_eq!(required("brain_search"), ["query"]);
    assert_eq!(
        optional("brain_ingest"),
        ["path", "text", "source", "title"]
    );
    assert_eq!(required("brain_forget"), ["source"]);
    assert_eq!(optional("jobs_run"), ["id"]);
    assert!(required("policy_set").is_empty());
    for empty in [
        "engines_list",
        "engine_get",
        "policy_get",
        "agents_list",
        "brain_sources",
        "jobs_list",
        "sources_list",
        "import_scan",
        "import_status",
        "import_retry_failed",
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
        ("erase_all", json!({"confirm": "yes"})),
        ("erase_all", json!({"confirm": false})),
        ("sources_add", json!({"kind": "folder"})),
        ("sources_remove", json!({})),
        ("policy_set", json!({"budget_tokens": "many"})),
        ("policy_set", json!({"no_such_setting": 1})),
        ("brain_search", json!({})),
        ("brain_forget", json!({})),
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
        ("erase_all", json!({"confirm": true})),
        ("items_list", json!({})),
        ("explore", json!({"facet": "kind"})),
        ("items_get", json!({"ids": ["a"]})),
        ("pack_preview", json!({})),
        ("agents_list", json!({})),
        ("brain_sources", json!({})),
        ("brain_search", json!({"query": "q"})),
        ("brain_ingest", json!({"text": "t"})),
        ("brain_forget", json!({"source": "pdf"})),
        ("jobs_run", json!({})),
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

    call(&config, "learn", json!({"text": "Drinks tea"}))
        .await
        .unwrap();
    let erased = call(&config, "erase_all", json!({"confirm": true}))
        .await
        .unwrap();
    let erased = erased.get("result").unwrap_or(&erased);
    assert_eq!(erased["erased_scopes"], 1);
    let listed = call(&config, "items_list", json!({"limit": 5}))
        .await
        .unwrap();
    let listed = listed.get("result").unwrap_or(&listed);
    assert!(listed["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn policy_handlers_validate_persist_and_report() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);

    let policy = call(
        &config,
        "policy_set",
        json!({"log_conversations": false, "budget_tokens": 900, "team_limit": 0}),
    )
    .await
    .unwrap();
    let policy = policy.get("result").unwrap_or(&policy);
    assert_eq!(policy["log_conversations"], false);
    assert_eq!(policy["recall"]["budget_tokens"], 900);
    assert_eq!(policy["recall"]["team_limit"], 0);
    let saved = std::fs::read_to_string(&config.config_path).expect("config persisted");
    assert!(saved.contains("budget_tokens = 900"), "{saved}");

    let bad = call(&config, "policy_set", json!({"budget_tokens": 1}))
        .await
        .unwrap_err();
    assert!(bad.contains("INVALID_REQUEST"));

    let got = call(&config, "policy_get", json!({})).await.unwrap();
    assert_eq!(got.get("result").unwrap_or(&got)["root"], "root");
    let jobs = call(&config, "jobs_list", json!({})).await.unwrap();
    assert!(jobs.get("result").unwrap_or(&jobs)["pending"].is_array());
}

#[tokio::test]
async fn lifecycle_handlers_answer_over_a_bound_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);

    let filed = call(
        &config,
        "brain_ingest",
        json!({"text": "Standups are at 9:30", "source": "notion", "title": "Rituals"}),
    )
    .await
    .unwrap();
    assert_eq!(filed.get("result").unwrap_or(&filed)["source"], "notion");

    let sources = call(&config, "brain_sources", json!({})).await.unwrap();
    let sources = sources.get("result").unwrap_or(&sources);
    assert_eq!(sources["sources"][0]["source"], "notion");

    let preview = call(
        &config,
        "pack_preview",
        json!({"query": "when are standups?"}),
    )
    .await
    .unwrap();
    let preview = preview.get("result").unwrap_or(&preview);
    assert_eq!(preview["mode"], "turn");
    assert!(preview["pack"]["markdown"]
        .as_str()
        .unwrap()
        .contains("9:30"));

    let ran = call(&config, "jobs_run", json!({})).await.unwrap();
    assert_eq!(
        ran.get("result").unwrap_or(&ran)["runs"][0]["outcome"],
        "done"
    );

    let agents = call(&config, "agents_list", json!({})).await.unwrap();
    assert!(agents.get("result").unwrap_or(&agents)["agents"].is_array());
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
