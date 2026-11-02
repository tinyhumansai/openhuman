use super::*;
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use crate::memory::types::EngineStatus;
use tinymemory_api::{
    FetchMode, ItemKind, LearningKind, ListRequest, MemoryMeta, MetaFilter, SourceKind, SourceRef,
    StoreItem, ToolCallRef,
};

fn learn_params(text: &str) -> LearnParams {
    LearnParams {
        text: text.to_string(),
        kind: None,
        confidence: None,
        meta: None,
    }
}

#[cfg(feature = "channels")]
#[test]
fn corrupt_legacy_channel_threads_are_skipped_without_modifying_the_source() {
    let tmp = tempfile::tempdir().unwrap();
    let workspace = tmp.path().join("workspace");
    let memory_dir = workspace.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let legacy_file = memory_dir.join("channel_threads.json");
    let corrupt = br#"{"discord":["unfinished"}"#;
    std::fs::write(&legacy_file, corrupt).unwrap();

    let threads = legacy_channel_threads(&workspace, "discord");

    assert!(threads.is_empty(), "invalid legacy state is skipped");
    assert_eq!(std::fs::read(&legacy_file).unwrap(), corrupt);
    assert!(
        !legacy_file.with_extension("json.corrupt").exists(),
        "a state read must not move or quarantine the legacy file"
    );
}

#[cfg(feature = "channels")]
#[tokio::test]
async fn reserved_thread_matching_uses_module_identity_boundaries() {
    use tinychannels_bus::module_inbound::PrepareChannelIdentityRequest;

    let client = crate::channels::module_fixture::client().await;
    let base_for = |channel: &'static str| {
        let client = client.clone();
        async move {
            crate::modules::channel_controller::prepare_identity(
                &client,
                PrepareChannelIdentityRequest {
                    channel: channel.into(),
                    sender: None,
                    reply_target: None,
                    thread_ts: None,
                },
            )
            .await
            .expect("linked TinyChannels module prepares its canonical base")
            .thread_id
        }
    };
    let base = base_for("discord").await;
    let neighboring_channel = base_for("discordish").await;

    assert!(is_channel_thread_id(&base, &base));
    assert!(is_channel_thread_id(&format!("{base}/thread-1"), &base));
    assert!(is_channel_thread_id(
        &format!("{base}#thread:1700.001"),
        &base
    ));
    assert!(!is_channel_thread_id(&format!("{base}suffix"), &base));
    assert!(!is_channel_thread_id(&neighboring_channel, &base));
    assert!(!is_channel_thread_id(&format!("prefix-{base}"), &base));
}

#[cfg(feature = "channels")]
#[tokio::test]
async fn unavailable_channel_module_reports_error_and_queues_known_legacy_threads() {
    use tinychannels_bus::module_inbound::PrepareChannelIdentityRequest;

    let _linked_module = crate::channels::module_fixture::client().await;
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    config.modules.enabled = false;
    let legacy_thread = "legacy-discord-thread";
    let legacy_file = config.workspace_dir.join("memory/channel_threads.json");
    std::fs::create_dir_all(legacy_file.parent().unwrap()).unwrap();
    std::fs::write(&legacy_file, br#"{"discord":["legacy-discord-thread"]}"#).unwrap();

    let unavailable = crate::modules::channel_controller::prepare_identity(
        &crate::modules::client(config.clone()),
        PrepareChannelIdentityRequest {
            channel: "discord".into(),
            sender: None,
            reply_target: None,
            thread_ts: None,
        },
    )
    .await
    .err()
    .expect("explicitly disabled module must stay unavailable");
    assert_eq!(unavailable, tinybus::module::ModuleCallError::Unavailable);

    let error = forget_channel(&config, "discord")
        .await
        .expect_err("an unavailable identity module must be visible to the caller");
    assert_eq!(error.code(), "ENGINE");
    assert!(error.to_string().contains("channel identity preparation"));
    assert_eq!(
        crate::memory::deletion::pending(&config.workspace_dir),
        vec![crate::memory::deletion::PendingDeletion::Thread {
            thread_id: legacy_thread.into()
        }]
    );
}

#[tokio::test]
async fn every_engine_operation_reports_memory_off_without_an_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);

    let recall_error = recall(
        &config,
        RecallParams {
            refers_to: None,
            question: "anything?".into(),
            filter: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(recall_error.code(), "MEMORY_OFF");

    let fetch_error = fetch(
        &config,
        FetchParams {
            refers_to: None,
            query: "x".into(),
            mode: None,
            filter: None,
            limit: None,
            cursor: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(fetch_error.code(), "MEMORY_OFF");

    let learn_error = learn(&config, learn_params("a fact"), None)
        .await
        .unwrap_err();
    assert_eq!(learn_error.code(), "MEMORY_OFF");

    let forget_error = forget(
        &config,
        ForgetParams {
            ids: vec!["a".into()],
            reach: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(forget_error.code(), "MEMORY_OFF");

    let list_error = items_list(&config, ItemsListParams::default())
        .await
        .unwrap_err();
    assert_eq!(list_error.code(), "MEMORY_OFF");

    assert_eq!(engines_list(&config).await.active, None);
    assert!(!engines_list(&config).await.engines.is_empty());

    let view = engine_get(&config).await;
    assert_eq!(view.status, EngineStatus::Off);
    assert_eq!(view.engine.as_deref(), Some(TINYHUMANS_ENGINE));
    assert!(view.reason.is_some());
    assert!(view.fetch_modes.is_empty());
    assert!(!view.has_key);
}

#[tokio::test]
async fn learn_recall_fetch_list_and_forget_round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);

    let learned = learn(
        &config,
        learn_params("The user prefers dark roast coffee"),
        None,
    )
    .await
    .unwrap();
    assert!(!learned.id.is_empty());

    let answer = recall(
        &config,
        RecallParams {
            refers_to: None,
            question: "coffee".into(),
            filter: None,
            limit: Some(5),
        },
    )
    .await
    .unwrap();
    assert!(!answer.citations.is_empty());

    let page = fetch(
        &config,
        FetchParams {
            refers_to: None,
            query: "coffee".into(),
            mode: None,
            filter: Some(MetaFilter {
                kinds: vec![ItemKind::Learning],
                ..MetaFilter::default()
            }),
            limit: Some(1000),
            cursor: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(page.hits.len(), 1);

    let listed = items_list(&config, ItemsListParams::default())
        .await
        .unwrap();
    assert_eq!(listed.items.len(), 1);
    assert_eq!(listed.items[0].id.0, learned.id);
    // A preview listing names the same items (the reference engine's
    // preview is its listing).
    let previewed = items_list(
        &config,
        ItemsListParams {
            preview: true,
            ..ItemsListParams::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(
        previewed.items.iter().map(|h| &h.id).collect::<Vec<_>>(),
        listed.items.iter().map(|h| &h.id).collect::<Vec<_>>()
    );

    let view = engines_list(&config).await;
    assert_eq!(view.active.as_deref(), Some("reference"));
    let health = engine_get(&config).await;
    assert_eq!(health.status, EngineStatus::Ok);
    assert!(!health.fetch_modes.is_empty());

    let forgotten = forget(
        &config,
        ForgetParams {
            ids: vec![learned.id.clone(), "  ".into()],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(forgotten.forgotten, 1);
    assert!(engine.is_empty());
}

#[tokio::test]
async fn forget_needs_an_id() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    bind_reference(&config);
    let error = forget(
        &config,
        ForgetParams {
            ids: vec![" ".into()],
            reach: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "INVALID_REQUEST");
}

#[test]
fn host_meta_wins_over_caller_meta() {
    let caller = MemoryMeta {
        thread_id: Some("forged".into()),
        channel: Some("caller-channel".into()),
        repo: Some("owner/repo".into()),
        tags: vec!["mine".into()],
        ..MemoryMeta::default()
    };
    let host = MemoryMeta {
        thread_id: Some("t-real".into()),
        channel: Some("telegram".into()),
        agent_id: Some("orchestrator".into()),
        tool_call: Some(ToolCallRef {
            name: "memory".into(),
            id: Some("call-1".into()),
        }),
        source: SourceRef {
            kind: SourceKind::Agent,
            id: None,
        },
        tags: vec!["host".into(), "mine".into()],
        ..MemoryMeta::default()
    };
    let item = learning_item(
        LearnParams {
            text: "  likes tea  ".into(),
            kind: Some(LearningKind::Preference),
            confidence: Some(0.5),
            meta: Some(caller),
        },
        Some(host),
    )
    .unwrap();
    let meta = item.meta();
    assert_eq!(meta.thread_id.as_deref(), Some("t-real"));
    assert_eq!(meta.channel.as_deref(), Some("telegram"));
    assert_eq!(meta.agent_id.as_deref(), Some("orchestrator"));
    assert_eq!(
        meta.repo.as_deref(),
        Some("owner/repo"),
        "caller fields kept"
    );
    assert_eq!(meta.source.kind, SourceKind::Agent);
    assert_eq!(meta.tags, vec!["mine".to_string(), "host".to_string()]);
    assert!(meta.observed_at.is_some());
    match item {
        StoreItem::Learning {
            text,
            kind,
            confidence,
            ..
        } => {
            assert_eq!(text, "likes tea");
            assert_eq!(kind, LearningKind::Preference);
            assert!((confidence - 0.5).abs() < f32::EPSILON);
        }
        other => panic!("expected a learning, got {:?}", other.kind()),
    }
}

#[test]
fn host_metadata_none_preserves_caller_channel() {
    let item = learning_item(
        LearnParams {
            text: "likes tea".into(),
            kind: None,
            confidence: None,
            meta: Some(MemoryMeta {
                channel: Some("caller-channel".into()),
                ..MemoryMeta::default()
            }),
        },
        Some(MemoryMeta::default()),
    )
    .unwrap();

    assert_eq!(item.meta().channel.as_deref(), Some("caller-channel"));
}

#[test]
fn learning_confidence_must_be_a_probability() {
    let mut params = learn_params("x");
    params.confidence = Some(1.5);
    assert_eq!(
        learning_item(params, None).unwrap_err().code(),
        "INVALID_REQUEST"
    );
    assert_eq!(
        learning_item(learn_params("   "), None).unwrap_err().code(),
        "INVALID_REQUEST"
    );
}

#[tokio::test]
async fn stores_are_scrubbed_of_secrets() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    learn(
        &config,
        learn_params("deploy key sk-ant-api03-abcdefghijklmnopqrstuvwxyz0123456789 is set"),
        None,
    )
    .await
    .unwrap();
    let items = stored(&engine, MetaFilter::default()).await;
    assert_eq!(items.len(), 1);
    assert!(!items[0]
        .text
        .contains("abcdefghijklmnopqrstuvwxyz0123456789"));
}

#[tokio::test]
async fn engine_set_validates_and_rebinds() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);

    let unknown = EngineSetParams {
        engine: "nope".into(),
        endpoint: None,
        api_key: None,
    };
    assert_eq!(
        apply_engine_set(&mut config, &unknown)
            .await
            .unwrap_err()
            .code(),
        "INVALID_REQUEST"
    );

    for endpoint in ["not a url", "ftp://host", "file:///etc"] {
        let params = EngineSetParams {
            engine: CORTEXDB_ENGINE.into(),
            endpoint: Some(endpoint.into()),
            api_key: None,
        };
        assert_eq!(
            apply_engine_set(&mut config, &params)
                .await
                .unwrap_err()
                .code(),
            "INVALID_REQUEST",
            "{endpoint}"
        );
    }

    let keyed_tinyhumans = EngineSetParams {
        engine: TINYHUMANS_ENGINE.into(),
        endpoint: None,
        api_key: Some("k".into()),
    };
    assert_eq!(
        apply_engine_set(&mut config, &keyed_tinyhumans)
            .await
            .unwrap_err()
            .code(),
        "INVALID_REQUEST"
    );

    assert!(!engine::is_on(&config).await);
    let cortex = EngineSetParams {
        engine: CORTEXDB_ENGINE.into(),
        endpoint: Some("https://cortex.example.test".into()),
        api_key: Some("cdb-test-key".into()),
    };
    apply_engine_set(&mut config, &cortex).await.unwrap();
    assert_eq!(config.memory.engine, CORTEXDB_ENGINE);
    assert_eq!(
        config.memory.endpoint_for(CORTEXDB_ENGINE).as_deref(),
        Some("https://cortex.example.test")
    );
    let bound = engine::resolve(&config).await.engine().expect("rebuilt on");
    assert_eq!(bound.id, CORTEXDB_ENGINE);
    assert_eq!(bound.endpoint, "https://cortex.example.test");

    let clear = EngineSetParams {
        engine: CORTEXDB_ENGINE.into(),
        endpoint: Some(String::new()),
        api_key: Some(String::new()),
    };
    apply_engine_set(&mut config, &clear).await.unwrap();
    assert!(config.memory.endpoint_for(CORTEXDB_ENGINE).is_none());
    assert!(
        !engine::is_on(&config).await,
        "removing the key turns memory off"
    );
}

#[tokio::test]
async fn disabling_memory_turns_it_off_and_keeps_the_engine_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    apply_engine_set(
        &mut config,
        &EngineSetParams {
            engine: CORTEXDB_ENGINE.into(),
            endpoint: Some("https://cortex.example.test".into()),
            api_key: Some("cdb-test-key".into()),
        },
    )
    .await
    .unwrap();
    assert!(engine::is_on(&config).await);

    let keyed = EngineSetParams {
        engine: DISABLED_ENGINE.into(),
        endpoint: None,
        api_key: Some("k".into()),
    };
    assert_eq!(
        apply_engine_set(&mut config, &keyed)
            .await
            .unwrap_err()
            .code(),
        "INVALID_REQUEST"
    );
    assert!(
        engine::is_on(&config).await,
        "a refused disable changes nothing"
    );

    let disable = EngineSetParams {
        engine: DISABLED_ENGINE.into(),
        endpoint: None,
        api_key: None,
    };
    apply_engine_set(&mut config, &disable).await.unwrap();
    assert_eq!(config.memory.engine, DISABLED_ENGINE);
    assert!(!engine::is_on(&config).await);
    assert_eq!(
        config.memory.endpoint_for(CORTEXDB_ENGINE).as_deref(),
        Some("https://cortex.example.test"),
        "the cortexdb endpoint survives disabling"
    );

    let view = engine_get(&config).await;
    assert_eq!(view.engine.as_deref(), Some(DISABLED_ENGINE));
    assert_eq!(view.status, EngineStatus::Off);
    assert_eq!(view.reason.as_deref(), Some("memory is disabled"));
    assert!(engines_list(&config).await.active.is_none());

    // Selecting cortexdb again needs no endpoint or key re-entry.
    apply_engine_set(
        &mut config,
        &EngineSetParams {
            engine: CORTEXDB_ENGINE.into(),
            endpoint: None,
            api_key: None,
        },
    )
    .await
    .unwrap();
    assert!(engine::is_on(&config).await);
}

#[tokio::test]
async fn fetch_refuses_a_mode_the_engine_does_not_declare() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    apply_engine_set(
        &mut config,
        &EngineSetParams {
            engine: CORTEXDB_ENGINE.into(),
            endpoint: Some("https://cortex.example.test".into()),
            api_key: Some("cdb-test-key".into()),
        },
    )
    .await
    .unwrap();
    let error = fetch(
        &config,
        FetchParams {
            refers_to: None,
            query: "x".into(),
            mode: Some(FetchMode::Keyword),
            filter: None,
            limit: None,
            cursor: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "UNSUPPORTED");
}

#[tokio::test]
async fn erase_all_needs_its_confirmation() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    learn(&config, learn_params("a fact"), None).await.unwrap();
    let error = erase_all(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        EraseAllParams { confirm: false },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "INVALID_REQUEST");
    assert_eq!(stored(&engine, MetaFilter::default()).await.len(), 1);
}

#[tokio::test]
async fn erase_all_erases_everything_the_engine_holds() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    learn(&config, learn_params("a fact"), None).await.unwrap();
    learn(&config, learn_params("another fact"), None)
        .await
        .unwrap();
    let view = erase_all(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        EraseAllParams { confirm: true },
    )
    .await
    .unwrap();
    assert!(view.erased_scopes >= 1, "{view:?}");
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
}

#[tokio::test]
async fn erase_all_reports_memory_off_without_an_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let error = erase_all(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        EraseAllParams { confirm: true },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), "MEMORY_OFF");
}

#[tokio::test]
async fn erasing_memory_cancels_queued_learning_so_restart_cannot_restore_it() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    let facts = crate::memory::tools::CallFacts::of(
        &config,
        &tinymemory_tools::layout::identity::Identity::agent("orchestrator"),
    );
    let queued = crate::memory::tool_writes::tests::enqueue(
        &config,
        &serde_json::json!({"action":"learn","text":"Prefers tea"}),
        &facts,
    )
    .unwrap();
    assert!(queued["status"].as_str().unwrap().starts_with("queued"));
    erase_all(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        EraseAllParams { confirm: true },
    )
    .await
    .unwrap();
    let restarted = config.clone();
    crate::memory::tool_writes::drain(&restarted).await;
    assert!(stored(&engine, MetaFilter::default()).await.is_empty());
}

/// With a reach, forget by id goes to the engine's `forget_within` (through
/// the scrubbing wrapper every bound engine sits in), so the engine never
/// sweeps the tree for the ids; without one it stays a plain forget by id.
#[tokio::test]
async fn forget_with_a_reach_forgets_within_it() {
    use tinymemory_api::MemoryEngine as _;

    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = std::sync::Arc::new(crate::memory::test_fixtures::RecordingEngine::new());
    crate::memory::test_fixtures::RecordingEngine::bind(&engine, &config);

    let mine = learn(&config, learn_params("tea in the morning"), None)
        .await
        .unwrap();
    let other = learn(&config, learn_params("coffee at night"), None)
        .await
        .unwrap();
    let namespace = engine
        .list(ListRequest {
            filter: MetaFilter::default(),
            limit: 10,
            cursor: None,
        })
        .await
        .unwrap()
        .items
        .into_iter()
        .find(|hit| hit.id.0 == mine.id)
        .expect("learned item listed")
        .meta
        .namespace;

    let forgotten = forget(
        &config,
        ForgetParams {
            ids: vec![mine.id.clone()],
            reach: Some(tinymemory_api::Reach::exact(namespace)),
        },
    )
    .await
    .unwrap();
    assert_eq!(forgotten.forgotten, 1);
    assert_eq!(engine.calls(), vec!["forget_within"]);

    let forgotten = forget(
        &config,
        ForgetParams {
            ids: vec![other.id.clone()],
            reach: None,
        },
    )
    .await
    .unwrap();
    assert_eq!(forgotten.forgotten, 1);
    assert_eq!(engine.calls(), vec!["forget_within", "forget"]);
}

#[path = "ops_compat_tests.rs"]
mod compat_tests;

#[tokio::test]
async fn disabled_engine_rejects_an_api_key_without_mutating_memory_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);
    let before = config.memory.clone();

    let error = apply_engine_set(
        &mut config,
        &EngineSetParams {
            engine: DISABLED_ENGINE.into(),
            endpoint: None,
            api_key: Some("not-a-real-key".into()),
        },
    )
    .await
    .unwrap_err();

    assert_eq!(error.code(), "INVALID_REQUEST");
    assert_eq!(config.memory, before);
}
