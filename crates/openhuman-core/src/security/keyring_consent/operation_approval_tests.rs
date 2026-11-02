use super::*;

#[test]
fn approval_event_contains_only_operation_metadata() {
    let event = crate::core::events::DomainEvent::KeyringAccessApprovalRequested(
        crate::core::events::KeyringAccessApprovalRequest {
            request_id: "request-1".into(),
            caller_agent: "openhuman.credentials".into(),
            origin: "LocalOperator".into(),
            purpose: "OpenHuman auth profile credential access".into(),
            access: "read".into(),
            name: "user-1:auth:openai:default".into(),
            expires_at_ms: 1_800_000_000_000,
        },
    );
    assert_eq!(event.variant_name(), "KeyringAccessApprovalRequested");
    assert_eq!(crate::core::events::DomainEvent::domain(&event), "keyring");
    let value = serde_json::to_value(&event).unwrap();
    let payload = &value["KeyringAccessApprovalRequested"];
    assert_eq!(payload["name"], "user-1:auth:openai:default");
    assert!(payload.get("value").is_none());
}

#[tokio::test]
async fn approval_response_resolves_only_its_matching_request() {
    let mut registry = PendingConsentRegistry::default();
    let timeout = std::time::Duration::from_secs(30);
    let (first, first_waiter) = registry
        .register("read", "auth:first", "call-first", timeout)
        .unwrap();
    let (second, second_waiter) = registry
        .register("write", "auth:second", "call-second", timeout)
        .unwrap();

    assert_ne!(first.request_id, second.request_id);
    assert!(registry.resolve(&second.request_id, false));
    assert!(!registry.resolve(&second.request_id, true));

    drop(registry);
    assert!(first_waiter.await.is_err());
    assert!(!second_waiter.await.unwrap());
}

#[test]
fn pending_approval_capacity_is_bounded() {
    let mut registry = PendingConsentRegistry::default();
    for index in 0..MAX_PENDING_APPROVALS {
        registry
            .register(
                "read",
                &format!("auth:{index}"),
                &format!("call:{index}"),
                std::time::Duration::from_secs(120),
            )
            .unwrap();
    }
    assert!(registry
        .register(
            "read",
            "auth:overflow",
            "call:overflow",
            std::time::Duration::from_secs(120)
        )
        .is_err());
}

#[tokio::test]
async fn dropping_waiter_discards_later_resolution() {
    let mut registry = PendingConsentRegistry::default();
    let (pending, waiter) = registry
        .register(
            "delete",
            "auth:profile",
            "call:profile",
            std::time::Duration::from_secs(120),
        )
        .unwrap();
    drop(waiter);

    assert!(!registry.resolve(&pending.request_id, true));
}

#[tokio::test]
async fn timed_out_approval_is_removed_from_capacity() {
    let registry = Mutex::new(PendingConsentRegistry::default());
    let (pending, waiter) = registry
        .lock()
        .unwrap()
        .register(
            "read",
            "auth:profile",
            "call:profile",
            std::time::Duration::from_millis(1),
        )
        .unwrap();

    assert_eq!(
        await_decision(
            &registry,
            &pending.request_id,
            waiter,
            std::time::Duration::from_millis(1),
        )
        .await,
        None
    );
    assert!(registry.lock().unwrap().pending.is_empty());
    registry.lock().unwrap().finish(
        &pending.request_id,
        "call:profile",
        Some(OperationOutcome::Pending),
    );
    assert_eq!(
        registry
            .lock()
            .unwrap()
            .take_operation_outcome("call:profile"),
        Some(OperationOutcome::Pending)
    );
}

#[test]
fn operation_outcome_is_correlated_and_consumed_once() {
    let call_id = uuid::Uuid::new_v4().to_string();
    let mut registry = PendingConsentRegistry::default();
    let (prompt, _waiter) = registry
        .register(
            "write",
            "auth:profile",
            &call_id,
            std::time::Duration::from_secs(120),
        )
        .unwrap();
    registry.finish(&prompt.request_id, &call_id, Some(OperationOutcome::Denied));

    assert_eq!(
        registry.take_operation_outcome(&call_id),
        Some(OperationOutcome::Denied)
    );
    assert_eq!(registry.take_operation_outcome(&call_id), None);
}

#[test]
fn unconsumed_denials_reserve_capacity_before_new_requests_register() {
    let mut registry = PendingConsentRegistry::default();
    for index in 0..MAX_PENDING_APPROVALS {
        let call_id = format!("denied-{index}");
        let (prompt, _waiter) = registry
            .register(
                "write",
                "auth:profile",
                &call_id,
                std::time::Duration::from_secs(120),
            )
            .unwrap();
        registry.finish(&prompt.request_id, &call_id, Some(OperationOutcome::Denied));
    }

    assert!(
        registry
            .register(
                "write",
                "auth:overflow",
                "call:overflow",
                std::time::Duration::from_secs(120)
            )
            .is_err(),
        "do not admit another keyring operation while denial outcomes await consumption"
    );
    assert_eq!(
        registry.take_operation_outcome("denied-0"),
        Some(OperationOutcome::Denied)
    );
    assert!(registry
        .register(
            "write",
            "auth:next",
            "call:next",
            std::time::Duration::from_secs(120)
        )
        .is_ok());
}
