use super::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
    PromptInjectionVerdict, MAX_PROMPT_BYTES,
};

#[tokio::test]
async fn holds_prompts_over_the_security_module_limit() {
    let input = "ordinary text ".repeat(MAX_PROMPT_BYTES / 12 + 1);
    assert!(input.len() > MAX_PROMPT_BYTES);

    let decision = enforce_prompt_input(
        &input,
        PromptEnforcementContext {
            source: "unit-test",
            request_id: None,
            user_id: None,
            session_id: None,
        },
    )
    .await;

    assert_eq!(decision.verdict, PromptInjectionVerdict::Review);
    assert_eq!(decision.action, PromptEnforcementAction::ReviewBlocked);
    assert!(decision
        .reasons
        .iter()
        .any(|reason| reason.code == "input.oversized"));
}
