use super::*;
use std::error::Error;

#[test]
fn display_formatting() {
    let err = AgentError::MaxIterationsExceeded { max: 10 };
    assert_eq!(
        err.to_string(),
        "Agent exceeded maximum tool iterations (10)"
    );

    let err = AgentError::CostBudgetExceeded {
        spent_microdollars: 5_500_000,
        budget_microdollars: 5_000_000,
    };
    assert!(err.to_string().contains("5.5000"));
}

#[test]
fn context_limit_detection() {
    assert!(is_context_limit_error("prompt is too long for model"));
    assert!(is_context_limit_error("context_length_exceeded"));
    assert!(!is_context_limit_error("rate limit exceeded"));
}

#[test]
fn max_iterations_detection_matches_display() {
    // The substring helper must match the variant's own Display output —
    // the channels dispatch / web_channel sites flatten the typed error
    // through a `String` boundary, so any drift between the constant
    // and `Display` silently re-enables Sentry emission for the cap
    // (OPENHUMAN-TAURI-99 / -98).
    let rendered = AgentError::MaxIterationsExceeded { max: 8 }.to_string();
    assert!(is_max_iterations_error(&rendered));
    assert!(is_max_iterations_error(
        "run_chat_task failed client_id=abc thread_id=t1 \
         error=Agent exceeded maximum tool iterations (10)"
    ));
    assert!(!is_max_iterations_error("provider returned 503"));
    assert!(!is_max_iterations_error(
        "Tool execution error [shell]: denied"
    ));
}

#[test]
fn permission_denied_display() {
    let err = AgentError::PermissionDenied {
        tool_name: "shell".into(),
        required_level: "Execute".into(),
        channel_max_level: "ReadOnly".into(),
    };
    assert!(err.to_string().contains("shell"));
    assert!(err.to_string().contains("Execute"));
}

#[test]
fn display_formats_other_variants() {
    assert!(AgentError::ProviderError {
        message: "boom".into(),
        retryable: true,
    }
    .to_string()
    .contains("retryable=true"));
    assert!(AgentError::ContextLimitExceeded {
        utilization_pct: 98
    }
    .to_string()
    .contains("98% utilized"));
    assert!(AgentError::ToolExecutionError {
        tool_name: "shell".into(),
        message: "denied".into(),
    }
    .to_string()
    .contains("Tool execution error [shell]"));
    assert!(AgentError::CompactionFailed {
        message: "summary failed".into(),
        consecutive_failures: 3,
    }
    .to_string()
    .contains("3 consecutive"));
}

#[test]
fn from_anyhow_recovers_typed_agent_error_and_other_source() {
    let typed = anyhow::anyhow!(AgentError::MaxIterationsExceeded { max: 4 });
    match AgentError::from(typed) {
        AgentError::MaxIterationsExceeded { max } => assert_eq!(max, 4),
        other => panic!("unexpected variant: {other}"),
    }

    let other = AgentError::from(anyhow::anyhow!("plain failure"));
    assert!(matches!(other, AgentError::Other(_)));
    assert!(other.source().is_some());
}

// ── AgentError::EmptyProviderResponse (TAURI-RUST-4JX) ──────────────────
//
// `agent::harness::session::turn` returns this variant when the provider's
// chat completion contains no text, no thinking, and no tool calls (a
// degenerate/poisoned response — typically a flaky local model). The
// variant was added so `run_single` can route it through `skips_sentry()`
// and demote like `MaxIterationsExceeded`, keeping TAURI-RUST-4JX off
// Sentry while preserving the user-visible error and the `Err` propagation
// contract.

#[test]
fn empty_provider_response_display_matches_user_facing_string() {
    // The exact wire string is anchored: the UI surfaces it verbatim to
    // the user, and the emit-site comment at
    // `agent/harness/session/turn.rs:801` (the warn breadcrumb) explicitly
    // calls out the "surfacing as error instead of a silent blank reply"
    // contract. Any change to this byte string is a user-visible message
    // change and a Sentry-fingerprint change.
    let err = AgentError::EmptyProviderResponse { iteration: 1 };
    assert_eq!(
        err.to_string(),
        "The model returned an empty response. Please try again."
    );
}

#[test]
fn skips_sentry_returns_true_for_known_user_state_variants() {
    // The two variants that represent user/provider state rather than a
    // code bug — `run_single` suppresses both from Sentry while still
    // returning `Err` so the user sees the failure.
    assert!(AgentError::MaxIterationsExceeded { max: 10 }.skips_sentry());
    assert!(AgentError::EmptyProviderResponse { iteration: 1 }.skips_sentry());
}

#[test]
fn skips_sentry_returns_false_for_real_failures() {
    // Every other variant represents either an actionable bug, an
    // upstream provider/network failure that triage cares about, or a
    // CompactionFailed that already has its own follow-up logic — none
    // of them should silently disappear from Sentry.
    let real_failures = [
        AgentError::ProviderError {
            message: "boom".into(),
            retryable: true,
        },
        AgentError::ContextLimitExceeded {
            utilization_pct: 98,
        },
        AgentError::ToolExecutionError {
            tool_name: "shell".into(),
            message: "denied".into(),
        },
        AgentError::CostBudgetExceeded {
            spent_microdollars: 1_000,
            budget_microdollars: 500,
        },
        AgentError::CompactionFailed {
            message: "summary failed".into(),
            consecutive_failures: 2,
        },
        AgentError::PermissionDenied {
            tool_name: "shell".into(),
            required_level: "Execute".into(),
            channel_max_level: "ReadOnly".into(),
        },
        AgentError::Other(anyhow::anyhow!("plain failure")),
    ];
    for err in real_failures {
        assert!(!err.skips_sentry(), "must NOT skip Sentry for: {err}");
    }
}
