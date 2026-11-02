use super::*;
use tinyagents_session::turn_state::types::{TurnLifecycle, TurnState};

#[tokio::test]
async fn streamed_flow_finalizer_settles_the_bridge_workspace_turn_state() {
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let config_a = test_config(&tmp_a);
    let config_b = test_config(&tmp_b);
    let ctx = crate::core::runtime::DispatchCtx::new(
        crate::core::runtime::CoreContext::for_test_with_config(
            crate::core::runtime::DomainSet::full(),
            config_a.clone(),
        ),
    );

    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global(
        &config_b.workspace_dir,
    )
    .expect("agent registry init");
    let mut agent = match crate::agent::OpenHumanSessionHost::from_config_for_agent(
        &ctx,
        &config_b,
        "workflow_builder",
    )
    .await
    {
        Ok(agent) => agent,
        Err(error) => {
            eprintln!("build workflow_builder agent: {error:#}");
            panic!("build workflow_builder agent failed");
        }
    };
    let target = FlowStreamTarget {
        thread_id: "flow-stream-workspace-binding".to_string(),
        request_id: "flow-stream-request-binding".to_string(),
    };
    for config in [&config_a, &config_b] {
        crate::storage::sessions::turn_states_for(&config.workspace_dir)
            .put(&TurnState::started(
                target.thread_id.clone(),
                target.request_id.clone(),
                25,
                "2026-10-16T00:00:00Z",
            ))
            .expect("seed turn state");
    }

    attach_flow_progress_bridge(&ctx, &mut agent, &target, "flows_build", &config_b);
    finalize_flow_stream(
        &ctx,
        &target,
        &Err("fixture agent failure".to_string()),
        "fixture prompt",
        &config_b,
    )
    .await;

    let state_a = crate::storage::sessions::turn_states_for(&config_a.workspace_dir)
        .get_turn(&target.thread_id, &target.request_id)
        .unwrap()
        .unwrap();
    let state_b = crate::storage::sessions::turn_states_for(&config_b.workspace_dir)
        .get_turn(&target.thread_id, &target.request_id)
        .unwrap()
        .unwrap();
    assert_eq!(state_a.lifecycle, TurnLifecycle::Started);
    assert_eq!(
        state_b.lifecycle,
        TurnLifecycle::Interrupted,
        "the finalizer must settle the exact store installed by the progress bridge"
    );
}
