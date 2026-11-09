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

#[tokio::test]
async fn physical_dispatch_refusal_keeps_its_typed_reason_ahead_of_stale_provider_error() {
    let refusal = crate::agent::tinyagents::budget::DispatchRefusal::new(
        crate::agent::tinyagents::budget::DispatchRefusalReason::CallLimit,
    );
    let slot: ModelErrorSlot = std::sync::Arc::new(std::sync::Mutex::new(Some(
        tinyinference_llm::Error::Model("stale provider error".to_string()),
    )));
    let mapped = map_turn_run_error(
        tinyagents_harness::TinyAgentsError::PhysicalDispatchAdmission(refusal),
        "m",
        10,
        &slot,
        None,
    )
    .await;

    assert_eq!(
        mapped.downcast_ref::<crate::agent::tinyagents::budget::DispatchRefusal>(),
        Some(&refusal)
    );
    assert!(slot.lock().unwrap().is_none());
}

#[tokio::test]
async fn any_other_run_failure_re_surfaces_the_typed_provider_error_from_the_slot() {
    // A run that failed for any other reason re-surfaces the provider error
    // the model adapter recorded, typed, so callers keep their downcasts and
    // the user sees the real cause; the slot is consumed.
    let slot: ModelErrorSlot = std::sync::Arc::new(std::sync::Mutex::new(Some(
        tinyinference_llm::Error::Model("OpenHuman returned HTTP 402: insufficient credits".into()),
    )));
    let other = || tinyagents_harness::TinyAgentsError::MissingNode("agent".into());

    let mapped = map_turn_run_error(other(), "m", 10, &slot, None).await;
    assert!(
        matches!(
            mapped.downcast_ref::<tinyinference_llm::Error>(),
            Some(tinyinference_llm::Error::Model(message)) if message.contains("402")
        ),
        "expected the typed provider error, got: {mapped}"
    );
    assert!(slot.lock().unwrap().is_none(), "the slot must be consumed");

    // With nothing recorded, the harness's own error is reported.
    let mapped = map_turn_run_error(other(), "m", 10, &slot, None).await;
    assert!(mapped.downcast_ref::<tinyinference_llm::Error>().is_none());
    assert!(
        mapped
            .to_string()
            .starts_with("tinyagents harness run failed:"),
        "{mapped}"
    );
}
