use super::*;

fn hosted(kind: HostedErrorKind, bound: Option<TimeoutBound>) -> HostedError {
    HostedError {
        kind,
        message: "hosted agent invocation timed out".to_string(),
        timeout_bound: bound,
        dispatch_refusal: None,
        run: None,
    }
}

#[test]
fn a_per_call_timeout_names_the_per_model_call_bound() {
    let err = run_error_from_hosted(hosted(
        HostedErrorKind::Timeout,
        Some(TimeoutBound::PerModelCall),
    ));
    assert!(matches!(err, TinyAgentsError::Timeout(_)));
    let text = err.to_string();
    assert_eq!(
        TurnTimeoutBound::from_message(&text),
        Some(TurnTimeoutBound::PerModelCall)
    );
}

#[test]
fn a_run_budget_timeout_names_the_run_bound() {
    let text = run_error_from_hosted(hosted(HostedErrorKind::Timeout, Some(TimeoutBound::Run)))
        .to_string();
    assert_eq!(
        TurnTimeoutBound::from_message(&text),
        Some(TurnTimeoutBound::RunRemaining)
    );
}

#[test]
fn an_unbounded_timeout_keeps_the_owner_conversion() {
    let owner = TinyAgentsError::from(hosted(HostedErrorKind::Timeout, None));
    let projected = run_error_from_hosted(hosted(HostedErrorKind::Timeout, None));

    assert_eq!(projected.to_string(), owner.to_string());
    assert_eq!(
        TurnTimeoutBound::from_message(&projected.to_string()),
        None,
        "the host must not invent a run-remaining bound when the owner supplied none"
    );
}

#[test]
fn a_non_timeout_failure_is_converted_as_before() {
    let err = run_error_from_hosted(hosted(HostedErrorKind::Provider, None));
    assert!(matches!(err, TinyAgentsError::Model(_)));
    assert_eq!(TurnTimeoutBound::from_message(&err.to_string()), None);
}

#[test]
fn a_hosted_physical_dispatch_refusal_keeps_its_typed_reason() {
    let refusal = crate::agent::tinyagents::budget::DispatchRefusal::new(
        crate::agent::tinyagents::budget::DispatchRefusalReason::CallLimit,
    );
    let error = HostedError {
        kind: HostedErrorKind::LimitExceeded,
        message: "hosted agent invocation exceeded a configured limit".to_string(),
        timeout_bound: None,
        dispatch_refusal: Some(refusal),
        run: None,
    };

    assert!(matches!(
        run_error_from_hosted(error),
        TinyAgentsError::PhysicalDispatchAdmission(actual)
            if actual.reason() == refusal.reason()
    ));
}
