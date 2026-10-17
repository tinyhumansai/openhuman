use super::*;

#[test]
fn identical_halt_summary_text_is_pinned() {
    assert_eq!(
        recoverable_identical_halt_summary("web_fetch", 5, "503 upstream"),
        "Stopping: the `web_fetch` call was retried 5 times with identical arguments and kept \
         failing — repeating it will not help. Last error:\n503 upstream\n\nThis looked \
         recoverable at first, but the same call exhausted the extended transient-failure \
         headroom. Report this back instead of retrying."
    );
}

#[test]
fn no_progress_halt_summary_text_is_pinned() {
    assert_eq!(
        recoverable_no_progress_halt_summary(4, "shell", "boom"),
        "Stopping: 4 recoverable-looking tool failures happened in a row with no successful \
         progress. Last error (from `shell`):\nboom\n\nThe turn is still bounded by the \
         iteration/cost limits, but this many consecutive transient failures means the goal is \
         not currently reachable. Report this back instead of retrying."
    );
}

#[test]
fn terminal_budget_summary_text_is_pinned() {
    assert_eq!(
        terminal_inference_halt_summary(TerminalInferenceFailure::BudgetExhausted, "spawn", "x"),
        "Stopping: the `spawn` step failed because the account is out of inference \
         budget/credits — every retry hits the same wall. Add credits to your account (or, \
         when using a custom/BYO provider, top up that provider's own account) and try again. \
         Details:\nx"
    );
}

#[test]
fn terminal_provider_config_summary_text_is_pinned() {
    assert_eq!(
        terminal_inference_halt_summary(TerminalInferenceFailure::ProviderConfig, "spawn", "x"),
        "Stopping: the `spawn` step failed because the configured model/provider rejected the \
         request (e.g. an unknown model, a non-chat/embedding model, a missing credential, or a \
         region block) — retrying will not help. Fix the model or API key in Connections → API \
         keys → LLM. Details:\nx"
    );
}

#[test]
fn halt_text_clamps_the_embedded_error_at_600_chars() {
    let long = "e".repeat(700);
    let out = truncate_for_halt(&long);
    assert_eq!(out, format!("{}\n… [truncated]", "e".repeat(600)));
    assert_eq!(truncate_for_halt(&"e".repeat(600)), "e".repeat(600));
}

#[test]
fn user_actionable_escalation_text_is_pinned() {
    assert_eq!(
        user_actionable_escalation("gmail_send", "Gmail is not connected").as_deref(),
        Some(
            "I can't continue without your input: the `gmail_send` action needs a service that \
             isn't connected. Gmail is not connected\n\nConnect it (Connections), then tell me \
             to retry — or tell me how you'd like to proceed instead."
        )
    );
    assert_eq!(
        user_actionable_escalation("gmail_send", "insufficient scope; not connected"),
        None
    );
    assert_eq!(user_actionable_escalation("shell", "exit 1"), None);
}
