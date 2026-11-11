use super::*;
use crate::config::Config;
use crate::core::runtime::context::CoreContext;
use crate::core::runtime::DispatchCtx;
use crate::core::runtime::DomainSet;
use crate::memory::test_fixtures::{bind_reference, config_in};
use serde_json::{json, Map, Value};
use tinymemory_api::MemoryEngine;

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
async fn call(
    config: &Config,
    function: &str,
    params: Value,
) -> std::result::Result<Value, String> {
    let handler = handler_for(function).expect("test function has a handler");
    let ctx = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    CoreContext::scope(ctx.clone(), async move {
        handler(DispatchCtx::new(ctx.clone()), object(params)).await
    })
    .await
    .map(|response| response.get("result").cloned().unwrap_or(response))
}

#[test]
fn every_spec_method_is_registered_with_its_exact_name() {
    let controllers = all_registered_controllers();
    let names: Vec<String> = controllers.iter().map(|c| c.rpc_method_name()).collect();
    assert_eq!(names, SPEC_METHODS, "registered methods, in spec order");
    assert_eq!(all_controller_schemas().len(), SPEC_METHODS.len());
    for controller in &controllers {
        assert_eq!(controller.schema.namespace, "memory");
    }
}

#[test]
fn policy_set_schema_exposes_all_recall_policy_fields() {
    let schema = schema("policy_set");
    let fields: Vec<_> = schema.inputs.iter().map(|field| field.name).collect();
    assert!(fields.contains(&"date_hint"));
    assert!(fields.contains(&"compaction_timeout_ms"));
    assert!(fields.contains(&"build_delay_secs"));
}

#[test]
fn every_function_resolves_through_its_handler_entry() {
    let expected_handlers: [(&str, crate::core::all::ControllerHandler); 35] = [
        ("engines_list", handlers::engines_list),
        ("engine_get", handlers::engine_get),
        ("engine_set", handlers::engine_set),
        ("policy_get", handlers::policy_get),
        ("policy_set", handlers::policy_set),
        ("pack_preview", handlers::pack_preview),
        ("recall", handlers::recall),
        ("fetch", handlers::fetch),
        ("learn", handlers::learn),
        ("forget", handlers::forget),
        ("erase_all", handlers::erase_all),
        ("items_list", handlers::items_list),
        ("explore", handlers::explore),
        ("items_get", handlers::items_get),
        ("agents_list", handlers::agents_list),
        (
            "conversations_backfill_status",
            handlers::conversations_backfill_status,
        ),
        (
            "conversations_backfill_start",
            handlers::conversations_backfill_start,
        ),
        ("brain_sources", handlers::brain_sources),
        ("brain_search", handlers::brain_search),
        ("brain_ingest", handlers::brain_ingest),
        ("brain_forget", handlers::brain_forget),
        ("sources_list", handlers::sources_list),
        ("sources_add", handlers::sources_add),
        ("sources_remove", handlers::sources_remove),
        ("sources_sync", handlers::sources_sync),
        ("jobs_list", handlers::jobs_list),
        ("jobs_run", handlers::jobs_run),
        ("import_scan", handlers::import_scan),
        ("import_start", handlers::import_start),
        ("import_status", handlers::import_status),
        ("import_retry_failed", handlers::import_retry_failed),
        ("migration_scan", handlers::migration_scan),
        ("migration_start", handlers::migration_start),
        ("migration_status", handlers::migration_status),
        ("migration_retry", handlers::migration_retry),
    ];

    assert_eq!(FUNCTIONS.len(), expected_handlers.len());
    for (function, (expected_name, expected_handler)) in FUNCTIONS.iter().zip(expected_handlers) {
        assert_eq!(*function, expected_name);
        assert!(std::ptr::fn_addr_eq(
            handler_for(function).expect("registered function has a handler"),
            expected_handler
        ));
    }

    assert!(handler_for("no_such_function").is_none());
}

#[test]
fn unknown_function_gets_the_placeholder_schema() {
    let unknown = schema("no_such_function");
    assert_eq!(unknown.function, "unknown");
    assert!(!FUNCTIONS.contains(&"unknown"));
}

#[test]
fn required_inputs_match_the_spec() {
    let optional = |function: &str| -> Vec<&'static str> {
        schema(function)
            .inputs
            .iter()
            .filter(|input| !input.required)
            .map(|input| input.name)
            .collect()
    };
    assert_eq!(optional("engine_set"), ["endpoint", "api_key"]);
    assert_eq!(optional("sources_remove"), ["forget_items"]);
    assert_eq!(optional("sources_sync"), ["id"]);
    assert_eq!(optional("migration_start"), ["takeover"]);
    assert_eq!(optional("pack_preview"), ["query", "thread_id", "agent_id"]);
    assert_eq!(
        optional("brain_ingest"),
        ["path", "text", "source", "title"]
    );
    assert_eq!(optional("jobs_run"), ["id"]);
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

#[test]
fn sources_add_schema_lists_only_supported_kinds_and_target_formats() {
    let schema = schema("sources_add");
    let field = |name: &str| {
        schema
            .inputs
            .iter()
            .find(|input| input.name == name)
            .unwrap_or_else(|| panic!("missing sources_add input {name}"))
    };

    assert_eq!(
        field("kind").comment,
        "folder | file | link | github | rss."
    );
    assert_eq!(
        field("target").comment,
        "Path, URL, owner/repo or feed URL."
    );
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
    let id = learned["id"].as_str().expect("id").to_string();

    let listed = call(&config, "items_list", json!({"limit": 5}))
        .await
        .unwrap();
    assert_eq!(listed["items"].as_array().unwrap().len(), 1);

    let recalled = call(&config, "recall", json!({"question": "oolong"}))
        .await
        .unwrap();
    assert!(!recalled["citations"].as_array().unwrap().is_empty());

    let fetched = call(&config, "fetch", json!({"query": "oolong"}))
        .await
        .unwrap();
    assert_eq!(fetched["hits"].as_array().unwrap().len(), 1);

    let engines = call(&config, "engines_list", json!({})).await.unwrap();
    assert_eq!(engines["active"], "reference");

    let engine = call(&config, "engine_get", json!({})).await.unwrap();
    assert_eq!(engine["status"], "ok");

    let forgotten = call(&config, "forget", json!({"ids": [id]})).await.unwrap();
    assert_eq!(forgotten["forgotten"], 1);

    call(&config, "learn", json!({"text": "Drinks tea"}))
        .await
        .unwrap();
    let erased = call(&config, "erase_all", json!({"confirm": true}))
        .await
        .unwrap();
    assert_eq!(erased["erased_scopes"], 1);
    let listed = call(&config, "items_list", json!({"limit": 5}))
        .await
        .unwrap();
    assert!(listed["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn rpc_memory_handlers_apply_the_refuse_boundary_before_operations() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.root = Some("team:acme".into());
    bind_reference(&config);
    let outside = serde_json::to_value(tinymemory_api::Reach::subtree(
        "team:other".parse().unwrap(),
    ))
    .unwrap();

    for (function, params) in [
        (
            "recall",
            json!({"question": "x", "filter": {"reach": outside.clone()}}),
        ),
        (
            "fetch",
            json!({"query": "x", "filter": {"reach": outside.clone()}}),
        ),
        (
            "learn",
            json!({"text": "outside", "meta": {"namespace": "team:other"}}),
        ),
        (
            "forget",
            json!({"ids": ["not-present"], "reach": outside.clone()}),
        ),
        ("items_list", json!({"filter": {"reach": outside.clone()}})),
        (
            "explore",
            json!({
                "facet": "kind",
                "path": [{"facet": "namespace", "value": "team:other"}]
            }),
        ),
        (
            "items_get",
            json!({"ids": ["not-present"], "reach": outside.clone()}),
        ),
    ] {
        let error = call(&config, function, params).await.unwrap_err();
        assert!(error.contains("INVALID_REQUEST"), "{function}: {error}");
        assert!(
            error.contains("outside this caller's memory")
                && error.contains("team:other")
                && error.contains("team:acme"),
            "{function}: unexpected boundary error: {error}"
        );
    }
}

#[tokio::test]
async fn rpc_default_reads_stay_within_the_configured_identity_root() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.memory.root = Some("team:acme".into());
    let engine = bind_reference(&config);
    for (text, root) in [
        ("acme memory", "team:acme"),
        ("other team secret", "team:other"),
    ] {
        engine
            .store(tinymemory_api::StoreItem::learning(
                text,
                tinymemory_api::LearningKind::Fact,
                0.9,
                tinymemory_api::MemoryMeta {
                    namespace: root.parse().unwrap(),
                    ..tinymemory_api::MemoryMeta::default()
                },
            ))
            .await
            .unwrap();
    }

    let listed = call(&config, "items_list", json!({})).await.unwrap();
    let listed = listed.get("result").unwrap_or(&listed);
    let items = listed["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["meta"]["namespace"], "team:acme");
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
    assert_eq!(got["root"], "root");
    let jobs = call(&config, "jobs_list", json!({})).await.unwrap();
    assert!(jobs["pending"].is_array());
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
    assert_eq!(filed["source"], "notion");

    let sources = call(&config, "brain_sources", json!({})).await.unwrap();
    assert_eq!(sources["sources"][0]["source"], "notion");

    let preview = call(
        &config,
        "pack_preview",
        json!({"query": "when are standups?"}),
    )
    .await
    .unwrap();
    assert_eq!(preview["mode"], "turn");
    assert!(preview["pack"]["markdown"]
        .as_str()
        .unwrap()
        .contains("9:30"));

    let ran = call(&config, "jobs_run", json!({})).await.unwrap();
    assert_eq!(ran["runs"][0]["outcome"], "done");

    let agents = call(&config, "agents_list", json!({})).await.unwrap();
    assert!(agents["agents"].is_array());
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
async fn rpc_learn_invalidates_a_cached_prefetch_pack() {
    use std::sync::Arc;
    use tinymemory_api::{LearningKind, MemoryEngine, MemoryMeta};

    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    engine
        .store(tinymemory_api::StoreItem::learning(
            "The user prefers dark roast coffee",
            LearningKind::Preference,
            0.9,
            MemoryMeta::default(),
        ))
        .await
        .unwrap();

    let ctx = CoreContext::for_test_with_config(DomainSet::full(), config.clone());
    let dispatch = DispatchCtx::new(ctx.clone());
    let identity = tinymemory_tools::layout::identity::Identity::root()
        .resolve(&crate::memory::scope::identity_pins(&config));
    let thread = "rpc-learn-invalidates-prefetch";
    let input = || {
        (
            thread.to_string(),
            0,
            "What coffee do I like?".to_string(),
            0,
            chrono::Utc::now(),
            false,
            None,
        )
    };

    CoreContext::scope(ctx.clone(), async {
        let config = Arc::new(config.clone());
        let (thread_id, turn_index, user_text, in_prompt_from, at, resumed, observed_actor) =
            input();
        assert!(crate::memory::lifecycle::prefetch::pre_turn(
            &dispatch,
            config.clone(),
            identity.clone(),
            thread_id,
            turn_index,
            user_text,
            in_prompt_from,
            at,
            resumed,
            observed_actor,
            None,
            None,
        )
        .await
        .is_none());
        crate::memory::lifecycle::prefetch::settle_for_test(
            &dispatch, &config, &identity, thread, false,
        )
        .await;
        let (thread_id, turn_index, user_text, in_prompt_from, at, resumed, observed_actor) =
            input();
        assert!(
            crate::memory::lifecycle::prefetch::pre_turn(
                &dispatch,
                config.clone(),
                identity.clone(),
                thread_id,
                turn_index,
                user_text,
                in_prompt_from,
                at,
                resumed,
                observed_actor,
                None,
                None,
            )
            .await
            .is_some(),
            "the initial turn should consume its completed pack"
        );

        call(
            &config,
            "learn",
            json!({"text": "The user enjoys espresso"}),
        )
        .await
        .unwrap();

        let (thread_id, turn_index, user_text, in_prompt_from, at, resumed, observed_actor) =
            input();
        assert!(
            crate::memory::lifecycle::prefetch::pre_turn(
                &dispatch,
                config,
                identity,
                thread_id,
                turn_index,
                user_text,
                in_prompt_from,
                at,
                resumed,
                observed_actor,
                None,
                None,
            )
            .await
            .is_none(),
            "a later turn must not consume a pack cached before RPC learn"
        );
    })
    .await;
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
    assert!(none["sources"].is_array());

    let missing = call(&config, "sources_remove", json!({"id": "src-missing"}))
        .await
        .unwrap();
    assert_eq!(missing["removed"], false);

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
    assert_eq!(scan["found"], false);

    let status = call(&config, "import_status", json!({})).await.unwrap();
    assert_eq!(status["state"]["phase"], "idle");

    let refused = call(&config, "import_start", json!({"consent": false}))
        .await
        .unwrap_err();
    assert!(refused.contains("INVALID_REQUEST"));
    let no_param = call(&config, "import_start", json!({})).await.unwrap_err();
    assert!(no_param.contains("INVALID_REQUEST"));
}
