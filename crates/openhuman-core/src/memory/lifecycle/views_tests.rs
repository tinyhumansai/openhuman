use super::*;

use chrono::Utc;
use tinymemory_api::{Error, LearningKind, MemoryEngine, MemoryMeta, StoreItem};

use crate::memory::lifecycle::hooks::pre_turn_work;
use crate::memory::test_fixtures::{bind_reference, config_in};

#[test]
fn policy_set_accepts_fields_reported_by_policy_get() {
    let params: PolicySetParams = serde_json::from_value(serde_json::json!({
        "log_conversations": false,
        "recall_enabled": false,
        "budget_tokens": 800,
        "learnings_limit": 9,
        "brain_limit": 5,
        "history_limit": 4,
        "team_limit": 3,
        "build_beliefs_every": 7,
        "pre_turn_timeout_ms": 5000,
        "date_hint": true,
        "compaction_timeout_ms": 12_000,
        "build_delay_secs": 900,
    }))
    .unwrap();
    assert_eq!(params.log_conversations, Some(false));
    assert_eq!(params.recall_enabled, Some(false));
    assert_eq!(params.budget_tokens, Some(800));
    assert_eq!(params.learnings_limit, Some(9));
    assert_eq!(params.brain_limit, Some(5));
    assert_eq!(params.history_limit, Some(4));
    assert_eq!(params.team_limit, Some(3));
    assert_eq!(params.build_beliefs_every, Some(7));
    assert_eq!(params.pre_turn_timeout_ms, Some(5000));
    assert_eq!(params.date_hint, Some(true));
    assert_eq!(params.compaction_timeout_ms, Some(12_000));
    assert_eq!(params.build_delay_secs, Some(900));
}

#[test]
fn the_policy_round_trips_and_refuses_out_of_range_values() {
    let mut config = Config::default();
    let view = policy_view(&config);
    assert!(view.log_conversations);
    assert_eq!(
        view.agent_id,
        tinymemory_tools::layout::identity::DEFAULT_AGENT_ID
    );
    assert_eq!(view.root, "root");
    assert!(!view.host_bound);

    apply_policy_set(
        &mut config,
        &PolicySetParams {
            log_conversations: Some(false),
            recall_enabled: Some(false),
            budget_tokens: Some(800),
            learnings_limit: Some(9),
            brain_limit: Some(5),
            history_limit: Some(4),
            team_limit: Some(3),
            build_beliefs_every: Some(7),
            pre_turn_timeout_ms: Some(5000),
            ..PolicySetParams::default()
        },
    )
    .unwrap();
    let view = policy_view(&config);
    assert!(!view.log_conversations);
    assert!(!view.recall.enabled);
    assert_eq!(view.recall.budget_tokens, 800);
    assert_eq!(view.recall.learnings_limit, 9);
    assert_eq!(view.recall.brain_limit, 5);
    assert_eq!(view.recall.history_limit, 4);
    assert_eq!(view.recall.team_limit, 3);
    assert_eq!(view.recall.build_beliefs_every, 7);
    assert_eq!(view.recall.pre_turn_timeout_ms, 5000);

    apply_policy_set(
        &mut config,
        &PolicySetParams {
            team_limit: Some(0),
            build_beliefs_every: Some(0),
            ..PolicySetParams::default()
        },
    )
    .unwrap();
    let zero_limits = policy_view(&config);
    assert_eq!(zero_limits.recall.team_limit, 0);
    assert_eq!(zero_limits.recall.build_beliefs_every, 0);

    apply_policy_set(
        &mut config,
        &PolicySetParams {
            date_hint: Some(true),
            compaction_timeout_ms: Some(12_000),
            build_delay_secs: Some(900),
            ..PolicySetParams::default()
        },
    )
    .unwrap();
    let view = policy_view(&config);
    assert!(view.recall.date_hint);
    assert_eq!(view.recall.compaction_timeout_ms, 12_000);
    assert_eq!(view.recall.build_delay_secs, 900);

    for bad in [
        PolicySetParams {
            budget_tokens: Some(10),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            budget_tokens: Some(16_001),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            learnings_limit: Some(51),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            brain_limit: Some(51),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            history_limit: Some(51),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            team_limit: Some(51),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            build_beliefs_every: Some(1001),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            pre_turn_timeout_ms: Some(0),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            pre_turn_timeout_ms: Some(30_001),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            build_delay_secs: Some(u64::MAX),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            build_delay_secs: Some(i64::MAX as u64),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            build_delay_secs: Some(10_000_000_000_000),
            ..PolicySetParams::default()
        },
    ] {
        let before = serde_json::to_value(&config).unwrap();
        let log_conversations = !config.memory.conversations.enabled;
        let recall_enabled = !config.memory.recall.enabled;
        let result = apply_policy_set(
            &mut config,
            &PolicySetParams {
                log_conversations: Some(log_conversations),
                recall_enabled: Some(recall_enabled),
                ..bad
            },
        );
        assert!(matches!(result, Err(Error::InvalidRequest(_))));
        assert!(
            serde_json::to_value(&config).unwrap() == before,
            "invalid policy update must be atomic"
        );
    }
    config.memory.agent_id = Some("employee-7".into());
    assert!(policy_view(&config).host_bound);
}

#[tokio::test]
async fn the_preview_reads_without_logging_and_agents_are_listed() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    let engine = bind_reference(&config);
    engine
        .store(StoreItem::learning(
            "Invoices go out on the 1st",
            LearningKind::Fact,
            0.9,
            MemoryMeta::default(),
        ))
        .await
        .unwrap();
    let writer = tinymemory_tools::layout::identity::Identity::agent("writer")
        .resolve(&crate::memory::scope::identity_pins(&config));
    pre_turn_work(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &writer,
        "t".into(),
        0,
        "draft the invoice email".into(),
        0,
        Utc::now(),
        false,
        None,
        None,
    )
    .await;

    let turn = pack_preview(
        &config,
        &tinymemory_tools::layout::identity::Identity::root(),
        PackPreviewParams {
            query: Some("when do invoices go out?".into()),
            agent_id: Some("writer".into()),
            ..PackPreviewParams::default()
        },
    )
    .await
    .unwrap();
    assert_eq!(turn.mode, "turn");
    assert_eq!(turn.agent_id, "writer");
    assert!(turn.pack.markdown.contains("1st"));

    let session = pack_preview(
        &config,
        &tinymemory_tools::layout::identity::Identity::root(),
        PackPreviewParams::default(),
    )
    .await
    .unwrap();
    assert_eq!(session.mode, "session");

    let agents = agents_list(
        &config,
        &tinymemory_tools::layout::identity::Identity::root(),
    )
    .await
    .unwrap();
    assert_eq!(
        agents.agents,
        [AgentCount {
            agent_id: "writer".into(),
            turns: 1
        }]
    );
}

#[tokio::test]
async fn jobs_run_needs_an_engine_and_names_an_unknown_job() {
    let tmp = tempfile::tempdir().unwrap();
    let config = config_in(&tmp);
    assert!(jobs_run(&config, JobsRunParams::default()).await.is_err());
    bind_reference(&config);
    assert!(jobs_run(&config, JobsRunParams::default())
        .await
        .unwrap()
        .runs
        .is_empty());
    assert!(jobs_run(
        &config,
        JobsRunParams {
            id: Some("x".into())
        }
    )
    .await
    .is_err());
    assert!(jobs_list(&config).await.pending.is_empty());
}
