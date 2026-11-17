use super::*;

use super::super::binding::AgentRunRegistry;

#[test]
fn generated_caps_host_interface_members_match_the_v2_wire_contract() {
    let temp = tempfile::TempDir::new().unwrap();
    let binding =
        super::super::binding_tests::binding(crate::core::runtime::DispatchCtx::unbound(), &temp);
    let service = CapsHostService::new(Arc::new(binding));
    let actual: std::collections::HashSet<_> = tinybus::service::Interface::members(&service)
        .into_iter()
        .map(|member| member.to_string())
        .collect();
    let expected: std::collections::HashSet<_> = runs::CAPS_METHODS
        .iter()
        .map(|member| (*member).to_owned())
        .collect();
    assert_eq!(actual, expected);
}

fn state() -> Arc<tokio::sync::Mutex<super::super::agent::AgentRunState>> {
    Arc::new(tokio::sync::Mutex::new(
        super::super::agent::AgentRunState::default(),
    ))
}

#[tokio::test]
async fn cancel_before_run_admission_tombstones_id_and_prevents_task_start() {
    let mut registry = AgentRunRegistry::default();
    registry.cancel("request-1".into());
    let cancellation = tokio_util::sync::CancellationToken::new();

    let admission = registry
        .admit("request-1".into(), state(), cancellation.clone())
        .unwrap();

    assert!(!admission.start_task);
    assert!(admission.cancelled_before_admission);
    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn cancel_after_run_admission_cancels_the_existing_invocation() {
    let mut registry = AgentRunRegistry::default();
    let cancellation = tokio_util::sync::CancellationToken::new();
    let admission = registry
        .admit("request-1".into(), state(), cancellation.clone())
        .unwrap();
    assert!(admission.start_task);

    registry.cancel("request-1".into());

    assert!(cancellation.is_cancelled());
}

#[tokio::test]
async fn duplicate_agent_run_request_returns_the_original_handle_without_restarting() {
    let mut registry = AgentRunRegistry::default();
    let first_state = state();
    let first = registry
        .admit(
            "request-1".into(),
            Arc::clone(&first_state),
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();

    let duplicate = registry
        .admit(
            "request-1".into(),
            state(),
            tokio_util::sync::CancellationToken::new(),
        )
        .unwrap();

    assert!(first.start_task);
    assert!(!duplicate.start_task);
    assert_eq!(duplicate.entry.handle, first.entry.handle);
    assert!(Arc::ptr_eq(&duplicate.entry.state, &first_state));
}

#[test]
fn malformed_agent_request_ids_are_rejected() {
    assert!(!valid_agent_request_id(""));
    assert!(!valid_agent_request_id(&"x".repeat(129)));
    assert!(valid_agent_request_id("client-request-1"));
}

#[test]
fn tombstone_capacity_overflow_fails_closed_instead_of_evicting_cancelled_ids() {
    let mut registry = AgentRunRegistry::default();
    for index in 0..4_097 {
        registry.cancel(format!("cancelled-{index}"));
    }

    let result = registry.admit(
        "cancelled-4096".into(),
        state(),
        tokio_util::sync::CancellationToken::new(),
    );

    assert!(result.is_err());
}

#[tokio::test]
async fn agent_run_rejects_an_identity_from_another_bound_run() {
    let temp = tempfile::TempDir::new().unwrap();
    let binding =
        super::super::binding_tests::binding(crate::core::runtime::DispatchCtx::unbound(), &temp);
    let service = CapsHostService::new(Arc::new(binding));
    let member = tinybus::MemberName::new(runs::caps::AGENT_RUN).unwrap();
    let error = tinybus::service::Interface::call(
        &service,
        &member,
        serde_json::json!([{
            "client_request_id":"request-1",
            "agent":{"id":"agent"},
            "identity":{"run_id":"different-run","node_id":"agent-node"},
            "config":{}
        }]),
    )
    .await
    .unwrap_err();

    assert!(
        error.to_string().contains("different workflow run"),
        "unexpected AgentRun validation error: {error}"
    );
}

#[tokio::test]
async fn late_agent_run_after_cancel_ack_returns_cancelled_handle_without_starting_work() {
    let temp = tempfile::TempDir::new().unwrap();
    let binding =
        super::super::binding_tests::binding(crate::core::runtime::DispatchCtx::unbound(), &temp);
    let service = CapsHostService::new(Arc::new(binding));
    let cancel_member = tinybus::MemberName::new(runs::caps::AGENT_CANCEL).unwrap();
    let run_member = tinybus::MemberName::new(runs::caps::AGENT_RUN).unwrap();
    let events_member = tinybus::MemberName::new(runs::caps::AGENT_EVENTS).unwrap();

    tinybus::service::Interface::call(
        &service,
        &cancel_member,
        serde_json::json!([{"client_request_id":"request-1"}]),
    )
    .await
    .unwrap();
    let handle = tinybus::service::Interface::call(
        &service,
        &run_member,
        serde_json::json!([{
            "client_request_id":"request-1",
            "agent":{"id":"agent"},
            "config":{}
        }]),
    )
    .await
    .unwrap()["handle"]
        .as_str()
        .unwrap()
        .to_owned();
    let error = tinybus::service::Interface::call(
        &service,
        &events_member,
        serde_json::json!([{"handle":handle,"cursor":0}]),
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("cancelled before admission"));
}
