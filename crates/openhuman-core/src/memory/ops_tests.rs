use super::*;
use crate::memory::error::{INVALID_REQUEST, MEMORY_OFF, UNSUPPORTED};
use crate::memory::test_fixtures::{bind_reference, config_in, stored};
use crate::memory::types::EngineStatus;
use tinymemory::{FetchMode, ItemKind, MetaFilter, SourceKind, SourceRef, ToolCallRef};

fn learn_params(text: &str) -> LearnParams {
    LearnParams {
        text: text.to_string(),
        kind: None,
        confidence: None,
        meta: None,
    }
}

#[tokio::test]
async fn every_engine_operation_reports_memory_off_without_an_engine() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);

    let recall_error = recall(
        &config,
        RecallParams {
            question: "anything?".into(),
            filter: None,
            limit: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(recall_error.code(), MEMORY_OFF);

    let fetch_error = fetch(
        &config,
        FetchParams {
            query: "x".into(),
            mode: None,
            filter: None,
            limit: None,
            cursor: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(fetch_error.code(), MEMORY_OFF);

    let learn_error = learn(&config, learn_params("a fact"), None)
        .await
        .unwrap_err();
    assert_eq!(learn_error.code(), MEMORY_OFF);

    let forget_error = forget(
        &config,
        ForgetParams {
            ids: vec!["a".into()],
        },
    )
    .await
    .unwrap_err();
    assert_eq!(forget_error.code(), MEMORY_OFF);

    let list_error = items_list(&config, ItemsListParams::default())
        .await
        .unwrap_err();
    assert_eq!(list_error.code(), MEMORY_OFF);

    assert_eq!(engines_list(&config).active, None);
    assert!(!engines_list(&config).engines.is_empty());

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

    let view = engines_list(&config);
    assert_eq!(view.active.as_deref(), Some("reference"));
    let health = engine_get(&config).await;
    assert_eq!(health.status, EngineStatus::Ok);
    assert!(!health.fetch_modes.is_empty());

    let forgotten = forget(
        &config,
        ForgetParams {
            ids: vec![learned.id.clone(), "  ".into()],
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
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), INVALID_REQUEST);
}

#[test]
fn host_meta_wins_over_caller_meta() {
    let caller = MemoryMeta {
        thread_id: Some("forged".into()),
        repo: Some("owner/repo".into()),
        tags: vec!["mine".into()],
        ..MemoryMeta::default()
    };
    let host = MemoryMeta {
        thread_id: Some("t-real".into()),
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
fn learning_confidence_must_be_a_probability() {
    let mut params = learn_params("x");
    params.confidence = Some(1.5);
    assert_eq!(
        learning_item(params, None).unwrap_err().code(),
        INVALID_REQUEST
    );
    assert_eq!(
        learning_item(learn_params("   "), None).unwrap_err().code(),
        INVALID_REQUEST
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

#[test]
fn engine_set_validates_and_rebinds() {
    let tmp = tempfile::tempdir().unwrap();
    let mut config = config_in(&tmp);

    let unknown = EngineSetParams {
        engine: "nope".into(),
        endpoint: None,
        api_key: None,
    };
    assert_eq!(
        apply_engine_set(&mut config, &unknown).unwrap_err().code(),
        INVALID_REQUEST
    );

    for endpoint in ["not a url", "ftp://host", "file:///etc"] {
        let params = EngineSetParams {
            engine: CORTEXDB_ENGINE.into(),
            endpoint: Some(endpoint.into()),
            api_key: None,
        };
        assert_eq!(
            apply_engine_set(&mut config, &params).unwrap_err().code(),
            INVALID_REQUEST,
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
            .unwrap_err()
            .code(),
        INVALID_REQUEST
    );

    assert!(!engine::is_on(&config));
    let cortex = EngineSetParams {
        engine: CORTEXDB_ENGINE.into(),
        endpoint: Some("https://cortex.example.test".into()),
        api_key: Some("cdb-test-key".into()),
    };
    apply_engine_set(&mut config, &cortex).unwrap();
    assert_eq!(config.memory.engine, CORTEXDB_ENGINE);
    assert_eq!(
        config.memory.endpoint_for(CORTEXDB_ENGINE).as_deref(),
        Some("https://cortex.example.test")
    );
    let bound = engine::resolve(&config).engine().expect("rebuilt on");
    assert_eq!(bound.id, CORTEXDB_ENGINE);
    assert_eq!(bound.endpoint, "https://cortex.example.test");

    let clear = EngineSetParams {
        engine: CORTEXDB_ENGINE.into(),
        endpoint: Some(String::new()),
        api_key: Some(String::new()),
    };
    apply_engine_set(&mut config, &clear).unwrap();
    assert!(config.memory.endpoint_for(CORTEXDB_ENGINE).is_none());
    assert!(!engine::is_on(&config), "removing the key turns memory off");
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
    .unwrap();
    let error = fetch(
        &config,
        FetchParams {
            query: "x".into(),
            mode: Some(FetchMode::Keyword),
            filter: None,
            limit: None,
            cursor: None,
        },
    )
    .await
    .unwrap_err();
    assert_eq!(error.code(), UNSUPPORTED);
}
