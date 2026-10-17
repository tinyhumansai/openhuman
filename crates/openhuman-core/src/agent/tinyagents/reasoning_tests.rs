use super::*;

#[test]
fn parses_wire_tokens_and_aliases() {
    assert_eq!(parse_reasoning_effort("high"), Some(ReasoningEffort::High));
    assert_eq!(parse_reasoning_effort(" Low "), Some(ReasoningEffort::Low));
    assert_eq!(parse_reasoning_effort("off"), Some(ReasoningEffort::None));
    assert_eq!(parse_reasoning_effort("max"), Some(ReasoningEffort::XHigh));
    assert_eq!(
        parse_reasoning_effort("xhigh"),
        Some(ReasoningEffort::XHigh)
    );
    assert_eq!(parse_reasoning_effort("turbo"), None);
}

#[test]
fn unset_config_leaves_the_provider_default() {
    assert_eq!(reasoning_for_config(&Config::default()), None);
}

#[test]
fn configured_effort_becomes_the_turn_reasoning() {
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("high".into());
    assert_eq!(
        reasoning_for_config(&config),
        Some(ReasoningConfig::effort(ReasoningEffort::High))
    );
}

#[test]
fn the_turn_models_own_level_wins_over_the_global_one() {
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("low".into());
    config
        .runtime
        .reasoning_effort_by_model
        .insert("deep-model".into(), "high".into());
    config.default_model = Some("deep-model".into());
    assert_eq!(
        reasoning_for_config(&config),
        Some(ReasoningConfig::effort(ReasoningEffort::High))
    );
    // Another model has no entry and falls back to the global level.
    config.default_model = Some("quick-model".into());
    assert_eq!(
        reasoning_for_config(&config),
        Some(ReasoningConfig::effort(ReasoningEffort::Low))
    );
}

#[test]
fn reasoning_disabled_without_an_effort_asks_for_none() {
    let mut config = Config::default();
    config.runtime.reasoning_enabled = Some(false);
    assert_eq!(
        reasoning_for_config(&config),
        Some(ReasoningConfig::effort(ReasoningEffort::None))
    );
}

#[test]
fn explicit_effort_wins_over_reasoning_disabled() {
    let mut config = Config::default();
    config.runtime.reasoning_enabled = Some(false);
    config.runtime.reasoning_effort = Some("low".into());
    assert_eq!(
        reasoning_for_config(&config),
        Some(ReasoningConfig::effort(ReasoningEffort::Low))
    );
}

#[test]
fn unknown_effort_falls_back_to_the_provider_default() {
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("turbo".into());
    assert_eq!(reasoning_for_config(&config), None);
}

#[test]
fn thread_choice_wins_over_config_and_clears_back_to_it() {
    let thread = "reasoning-test-thread-precedence";
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("low".into());

    apply_requested_effort(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread,
        Some("high"),
    )
    .expect("valid effort");
    assert_eq!(
        turn_reasoning(
            &crate::core::runtime::DispatchCtx::ambient(),
            Some(thread),
            Some(&config)
        ),
        Some(ReasoningConfig::effort(ReasoningEffort::High))
    );

    // Absent leaves the thread's choice untouched.
    apply_requested_effort(&crate::core::runtime::DispatchCtx::ambient(), thread, None)
        .expect("absent is fine");
    assert_eq!(
        thread_effort(&crate::core::runtime::DispatchCtx::ambient(), thread),
        Some(ReasoningEffort::High)
    );

    apply_requested_effort(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread,
        Some("default"),
    )
    .expect("clear");
    assert_eq!(
        thread_effort(&crate::core::runtime::DispatchCtx::ambient(), thread),
        None
    );
    assert_eq!(
        turn_reasoning(
            &crate::core::runtime::DispatchCtx::ambient(),
            Some(thread),
            Some(&config)
        ),
        Some(ReasoningConfig::effort(ReasoningEffort::Low))
    );
}

#[test]
fn unknown_requested_effort_is_rejected_and_keeps_the_prior_choice() {
    let thread = "reasoning-test-thread-reject";
    apply_requested_effort(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread,
        Some("off"),
    )
    .expect("valid effort");
    assert!(apply_requested_effort(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread,
        Some("turbo")
    )
    .is_err());
    assert_eq!(
        thread_effort(&crate::core::runtime::DispatchCtx::ambient(), thread),
        Some(ReasoningEffort::None)
    );
    set_thread_effort(&crate::core::runtime::DispatchCtx::ambient(), thread, None);
}

#[test]
fn turn_without_thread_or_config_has_no_reasoning() {
    assert_eq!(
        turn_reasoning(&crate::core::runtime::DispatchCtx::ambient(), None, None),
        None
    );
}

#[test]
fn root_turn_follows_the_thread_choice() {
    let thread = "reasoning-test-thread-root";
    set_thread_effort(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread,
        Some(ReasoningEffort::High),
    );
    let mut ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    ctx.thread_id = Some(thread.to_string());
    assert_eq!(
        turn_reasoning_for(&ctx, None, None),
        Some(ReasoningConfig::effort(ReasoningEffort::High))
    );
    set_thread_effort(&crate::core::runtime::DispatchCtx::ambient(), thread, None);
}

#[test]
fn subagent_turn_keeps_the_provider_default() {
    let thread = "reasoning-test-thread-subagent";
    set_thread_effort(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread,
        Some(ReasoningEffort::High),
    );
    let mut ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    ctx.thread_id = Some(thread.to_string());
    ctx.spawn_depth = 1;
    assert_eq!(turn_reasoning_for(&ctx, None, None), None);
    set_thread_effort(&crate::core::runtime::DispatchCtx::ambient(), thread, None);
}

#[test]
fn root_turn_without_a_thread_choice_uses_the_session_config() {
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("medium".into());
    let ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    assert_eq!(
        turn_reasoning_for(&ctx, Some(&config), None),
        Some(ReasoningConfig::effort(ReasoningEffort::Medium))
    );
}

#[test]
fn root_turn_reasoning_reserves_output_room_with_a_budget() {
    // #6951: with only an effort level, a reasoning model could think through
    // the whole output cap and return `length` with no tool call. A capped
    // turn now also carries a thinking budget of about 55% of that cap.
    let thread = "reasoning-test-thread-budget";
    set_thread_effort(
        &crate::core::runtime::DispatchCtx::ambient(),
        thread,
        Some(ReasoningEffort::High),
    );
    let mut ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    ctx.thread_id = Some(thread.to_string());
    let reasoning = turn_reasoning_for(
        &ctx,
        None,
        Some(crate::inference::provider::AGENT_TURN_MAX_OUTPUT_TOKENS),
    );
    set_thread_effort(&crate::core::runtime::DispatchCtx::ambient(), thread, None);
    assert_eq!(
        reasoning,
        Some(ReasoningConfig {
            effort: Some(ReasoningEffort::High),
            budget_tokens: Some(9011),
            summary: None,
        })
    );
}

#[test]
fn configured_effort_gets_a_budget_under_a_turn_cap() {
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("medium".into());
    let ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    let reasoning = turn_reasoning_for(&ctx, Some(&config), Some(10_000)).expect("reasoning");
    assert_eq!(reasoning.effort, Some(ReasoningEffort::Medium));
    assert_eq!(reasoning.budget_tokens, Some(5_500));
}

#[test]
fn disabled_reasoning_gets_no_budget() {
    let mut config = Config::default();
    config.runtime.reasoning_enabled = Some(false);
    let ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    assert_eq!(
        turn_reasoning_for(&ctx, Some(&config), Some(16_384)),
        Some(ReasoningConfig::effort(ReasoningEffort::None))
    );
}

#[test]
fn provider_default_reasoning_is_not_turned_on_by_a_budget() {
    // A bare budget would switch reasoning on for models that default to off
    // (OpenRouter treats `reasoning.max_tokens` as "enable"), so a turn with
    // no reasoning choice stays at the provider default.
    let ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    assert_eq!(turn_reasoning_for(&ctx, None, Some(16_384)), None);
}

#[test]
fn uncapped_turn_keeps_the_effort_only() {
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("high".into());
    let ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    assert_eq!(
        turn_reasoning_for(&ctx, Some(&config), None),
        Some(ReasoningConfig::effort(ReasoningEffort::High))
    );
}

#[test]
fn a_cap_too_small_for_a_provider_minimum_budget_keeps_the_effort_only() {
    // Anthropic models behind OpenRouter / the managed backend reject a
    // thinking budget under 1024 tokens; below that the effort alone is sent.
    let mut config = Config::default();
    config.runtime.reasoning_effort = Some("low".into());
    let ctx = crate::agent::tinyagents::host::OpenHumanRunContext::new();
    assert_eq!(
        turn_reasoning_for(&ctx, Some(&config), Some(1_500)),
        Some(ReasoningConfig::effort(ReasoningEffort::Low))
    );
}

#[tokio::test]
async fn reasoning_effort_stays_with_the_agent_that_chose_it() {
    use crate::core::runtime::agent_scope::test_agent_context;
    use crate::core::runtime::{context::CoreContext, DomainSet};
    let root = CoreContext::for_test(DomainSet::full(), None);
    let alpha = test_agent_context(&root, "alpha");
    let beta = test_agent_context(&root, "beta");
    let thread = "effort-shared-thread";

    {
        let scope_ctx = crate::core::runtime::DispatchCtx::new(std::sync::Arc::clone(&alpha));

        set_thread_effort(&scope_ctx, thread, Some(ReasoningEffort::High));
    };

    assert_eq!(
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(alpha);

            thread_effort(&scope_ctx, thread)
        },
        Some(ReasoningEffort::High)
    );
    assert_eq!(
        {
            let scope_ctx = crate::core::runtime::DispatchCtx::new(beta);

            thread_effort(&scope_ctx, thread)
        },
        None
    );
}
