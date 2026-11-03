use super::*;

#[test]
fn policy_denial_keeps_its_class_when_error_details_are_withheld() {
    let policy = HostFailurePolicy::new(
        SteeringHandle::allow_all(),
        Arc::new(std::sync::Mutex::new(None)),
    );
    let context = RunContext::new(
        tinyagents_harness::context::RunConfig::new("withheld-policy-error"),
        OpenHumanRunContext::new(),
    );
    let call = TaToolCall::new("denied-call", "delete_artifact", serde_json::json!({}));
    let invocation = ToolInvocationIdentity::new("denied-call", "delete_artifact");
    let scope = policy.scope(&context, &call);
    // An oversized result is deterministically withheld even if another test
    // has initialized a redactor in this process.
    let result = TaToolResult::error(format!(
        "[policy-denied] secret=fixture-private-value{}",
        " ".repeat(65 * 1024)
    ));
    let assessment = policy.assess(&context, &invocation, &call, &scope, &result);
    let tinyagents_harness::middleware::FailureAssessment::ClassifiedFailure {
        class,
        budget,
        error,
        ..
    } = assessment
    else {
        panic!("a policy denial must retain its terminal class when details are withheld");
    };
    assert_eq!(class, "policy");
    assert_eq!(budget, 0);
    assert!(!error.contains("fixture-private-value"));
}

#[tokio::test]
async fn classified_stop_cause_survives_the_latch_cleanup_on_agent_finish() {
    use tinyagents_harness::{context::RunConfig, middleware::AgentRun, steering::SteeringPolicy};

    let slot = std::sync::Arc::new(std::sync::Mutex::new(None));
    let handle = SteeringHandle::new(SteeringPolicy::new());
    let middleware = RepeatedToolFailureMiddleware::new(handle, 3, slot);
    middleware.policy.stop.halt_classified(
        "classified summary".into(),
        "uncertain_side_effect",
        "web_answer_tool",
        false,
        String::new,
    );
    let mut run = AgentRun::default();
    let mut context = RunContext::new(
        RunConfig::new("classified-stop"),
        crate::agent::tinyagents::host::OpenHumanRunContext::new(),
    );

    middleware
        .after_agent(&mut context, &(), &mut run)
        .await
        .expect("middleware cleanup succeeds");

    assert_eq!(
        run.failure_cause,
        Some(tinyagents_harness::no_progress::ClassifiedStopCause {
            class: "uncertain_side_effect".into(),
            operation: "web_answer_tool".into(),
            summary: "classified summary".into(),
        })
    );
}

#[test]
fn identical_recoverable_halt_uses_openhuman_copy() {
    let ladder = RecoverableFailureLadder::default();
    let mut summary = None;
    for _ in 0..8 {
        summary = ladder.record(&HaltCopy, "web_fetch", "fp", "connection reset");
        ladder.reset_streak();
    }
    let summary = summary.expect("eighth identical failure halts");
    assert_eq!(
        summary,
        "Stopping: the `web_fetch` call was retried 8 times with identical arguments and kept failing — repeating it will not help. Last error:\nconnection reset\n\nThis looked recoverable at first, but the same call exhausted the extended transient-failure headroom. Report this back instead of retrying."
    );
}

#[test]
fn varied_recoverable_failures_halt_with_the_no_progress_copy() {
    let ladder = RecoverableFailureLadder::default();
    let mut summary = None;
    for i in 0..20 {
        summary = ladder.record(&HaltCopy, "web_fetch", &format!("fp{i}"), "timed out");
        if summary.is_some() {
            break;
        }
    }
    let summary = summary.expect("a long varied streak halts");
    assert!(summary.starts_with("Stopping: "));
    assert!(summary.contains("in a row with no successful progress"));
}

#[test]
fn first_error_line_is_scrubbed_before_it_reaches_a_summary() {
    let line = first_error_line("boom\nsecond line");
    assert_eq!(line, "boom");
}

#[test]
fn shell_missing_app_nudge_names_the_host_not_the_desktop_target() {
    let tracker = ClassifiedFailureTracker::default();
    let step = classified_step(
        &tracker,
        "missing_app",
        "shell",
        "shell",
        1,
        "jq: not found",
    );
    let ClassifiedStep::Nudge(generic) = step else {
        panic!("missing_app within budget nudges");
    };
    assert!(generic.contains("desktop target"));
    assert!(missing_program_nudge("shell", "jq: not found").contains("not available on this host"));
}

#[test]
fn missing_connection_copy_keeps_its_typed_stop_category_and_operation() {
    let policy = HostFailurePolicy::new(
        SteeringHandle::allow_all(),
        Arc::new(std::sync::Mutex::new(None)),
    );
    let context = tinyagents_harness::context::RunContext::new(
        tinyagents_harness::context::RunConfig::new("typed-stop"),
        OpenHumanRunContext::new(),
    );
    let call = TaToolCall {
        id: "call-1".into(),
        name: "web_fetch".into(),
        arguments: serde_json::json!({}),
        invalid: None,
    };
    let cause = policy
        .halt_cause_for_call(
            &context,
            FailureStopKind::NoProgress,
            Some(tinyagents_harness::no_progress::ClassifiedStopCause {
                class: "repeated_failure".into(),
                operation: "web_fetch".into(),
                summary: "old generic wording".into(),
            }),
            "[composio:error:composio_platform] service is not connected",
            &call,
        )
        .expect("the same actionable blocker that changes the copy changes its category");
    assert_eq!(cause.class, "missing_connection");
    assert_eq!(cause.operation, "web_fetch");
}
