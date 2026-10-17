use super::*;

#[tokio::test]
async fn a_timeout_is_not_reported_as_a_provider_error_left_in_the_slot() {
    // #6724: a provider error from an earlier attempt must not be re-surfaced
    // as the cause of a run that timed out or was cancelled.
    for error in [
        tinyagents_harness::TinyAgentsError::CallTimeout("model call".to_string()),
        tinyagents_harness::TinyAgentsError::Timeout("run".to_string()),
        tinyagents_harness::TinyAgentsError::Cancelled,
    ] {
        let slot: ModelErrorSlot = std::sync::Arc::new(std::sync::Mutex::new(Some(
            tinyinference_llm::Error::Model(
                "OpenHuman returned HTTP 503: upstream unavailable".to_string(),
            ),
        )));
        let mapped = map_turn_run_error(error, "m", 10, &slot, None).await;
        assert!(
            !mapped.to_string().contains("503"),
            "stale provider error re-surfaced: {mapped}"
        );
    }
}

#[tokio::test]
async fn the_model_call_cap_and_depth_limit_win_over_a_stale_provider_error() {
    let stale = || -> ModelErrorSlot {
        std::sync::Arc::new(std::sync::Mutex::new(Some(
            tinyinference_llm::Error::Model(
                "OpenHuman returned HTTP 503: upstream unavailable".to_string(),
            ),
        )))
    };
    let cap = tinyagents_harness::TinyAgentsError::LimitExceeded("model call cap".to_string());
    let mapped = map_turn_run_error(cap, "m", 7, &stale(), None).await;
    assert!(matches!(
        mapped.downcast_ref::<tinyagents_harness::agent_error::AgentError>(),
        Some(tinyagents_harness::agent_error::AgentError::MaxIterationsExceeded { max: 7 })
    ));

    let depth = tinyagents_harness::TinyAgentsError::SubAgentDepth(3);
    let mapped = map_turn_run_error(depth, "m", 7, &stale(), None).await;
    assert!(matches!(
        mapped.downcast_ref::<crate::agent::subagent_host::SubagentRunError>(),
        Some(
            crate::agent::subagent_host::SubagentRunError::SpawnDepthExceeded {
                attempted_depth: 4,
                max_depth: 3
            }
        )
    ));
}
