use super::*;

use chrono::Utc;
use tinymemory_api::{LearningKind, MemoryEngine, MemoryMeta, StoreItem};

use crate::memory::lifecycle::hooks::{self, PreTurnInput};
use crate::memory::scope::MemoryIdentity;
use crate::memory::test_fixtures::{bind_reference, config_in};

#[test]
fn the_policy_round_trips_and_refuses_out_of_range_values() {
    let mut config = Config::default();
    let view = policy_view(&config);
    assert!(view.log_conversations);
    assert_eq!(view.agent_id, scope::DEFAULT_AGENT_ID);
    assert_eq!(view.root, "root");
    assert!(!view.host_bound);

    apply_policy_set(
        &mut config,
        &PolicySetParams {
            log_conversations: Some(false),
            recall_enabled: Some(false),
            budget_tokens: Some(800),
            team_limit: Some(0),
            build_beliefs_every: Some(0),
            ..PolicySetParams::default()
        },
    )
    .unwrap();
    let view = policy_view(&config);
    assert!(!view.log_conversations);
    assert!(!view.recall.enabled);
    assert_eq!(view.recall.budget_tokens, 800);
    assert_eq!(view.recall.team_limit, 0);
    assert_eq!(view.recall.build_beliefs_every, 0);

    for bad in [
        PolicySetParams {
            budget_tokens: Some(10),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            brain_limit: Some(51),
            ..PolicySetParams::default()
        },
        PolicySetParams {
            pre_turn_timeout_ms: Some(0),
            ..PolicySetParams::default()
        },
    ] {
        assert!(matches!(
            apply_policy_set(&mut config, &bad),
            Err(MemoryError::InvalidRequest(_))
        ));
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
    let writer = scope::MemoryIdentity::agent("writer").resolve(&config);
    hooks::pre_turn(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &writer,
        PreTurnInput {
            thread_id: "t".into(),
            turn_index: 0,
            user_text: "draft the invoice email".into(),
            in_prompt_from: 0,
            at: Utc::now(),
            resumed_after_compaction: false,
            observed_actor: None,
        },
    )
    .await;

    let turn = pack_preview(
        &config,
        &MemoryIdentity::root(),
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
        &MemoryIdentity::root(),
        PackPreviewParams::default(),
    )
    .await
    .unwrap();
    assert_eq!(session.mode, "session");

    let agents = agents_list(&config, &MemoryIdentity::root()).await.unwrap();
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
