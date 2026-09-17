use super::*;

#[test]
fn explicit_override_wins_every_other_signal() {
    for mode in [
        PrimaryTurnMode::Chat,
        PrimaryTurnMode::Assist,
        PrimaryTurnMode::Agent,
    ] {
        assert_eq!(
            resolve_primary_turn_mode(ModeResolutionInput {
                user_message: "autonomously edit the repository and run tests",
                explicit_override: Some(mode),
                has_live_agent_checkpoint: true,
            }),
            mode
        );
    }
}

#[test]
fn live_checkpoint_stays_agent_until_explicitly_cancelled() {
    assert_eq!(
        resolve_primary_turn_mode(ModeResolutionInput {
            user_message: "continue",
            explicit_override: None,
            has_live_agent_checkpoint: true,
        }),
        PrimaryTurnMode::Agent
    );
    assert_eq!(
        resolve_primary_turn_mode(ModeResolutionInput {
            user_message: "cancel that task",
            explicit_override: None,
            has_live_agent_checkpoint: true,
        }),
        PrimaryTurnMode::Chat
    );
}

#[test]
fn durable_work_is_agent_and_bounded_actions_are_assist() {
    for prompt in [
        "implement the parser and run tests",
        "schedule a daily report",
        "monitor the release until it completes",
        "delegate this research task",
    ] {
        assert_eq!(mode(prompt), PrimaryTurnMode::Agent, "{prompt}");
    }
    for prompt in [
        "show me today's news headlines",
        "fetch https://example.com",
        "remember that my dog is called Ada",
        "generate a portrait",
    ] {
        assert_eq!(mode(prompt), PrimaryTurnMode::Assist, "{prompt}");
    }
}

#[test]
fn conversation_and_ambiguity_default_downward() {
    for prompt in [
        "hey",
        "explain why the sky is blue",
        "rewrite this paragraph more clearly",
        "could you help me with something?",
        "write a poem about rain",
    ] {
        assert_eq!(mode(prompt), PrimaryTurnMode::Chat, "{prompt}");
    }
}

#[test]
fn engine_rollout_is_exact_and_has_persisted_rollback() {
    assert_eq!(
        resolve_orchestration_engine(
            OrchestrationEngine::Goose,
            LOCAL_QWEN_PROVIDER_BINDING,
            true,
        ),
        OrchestrationEngine::Goose
    );
    assert_eq!(
        resolve_orchestration_engine(
            OrchestrationEngine::Tinyagents,
            LOCAL_QWEN_PROVIDER_BINDING,
            true,
        ),
        OrchestrationEngine::Tinyagents
    );
    assert_eq!(
        resolve_orchestration_engine(OrchestrationEngine::Goose, "openai:gpt", true),
        OrchestrationEngine::Tinyagents
    );
    assert_eq!(
        resolve_orchestration_engine(
            OrchestrationEngine::Goose,
            LOCAL_QWEN_PROVIDER_BINDING,
            false,
        ),
        OrchestrationEngine::Tinyagents
    );
}

fn mode(prompt: &str) -> PrimaryTurnMode {
    resolve_primary_turn_mode(ModeResolutionInput {
        user_message: prompt,
        explicit_override: None,
        has_live_agent_checkpoint: false,
    })
}
