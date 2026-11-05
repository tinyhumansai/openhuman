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

fn shown(prompt: &ApprovalPrompt) -> crate::core::events::KeyringAccessApprovalRequest {
    crate::core::events::KeyringAccessApprovalRequest {
        request_id: prompt.request_id.clone(),
        caller_agent: "openhuman.credentials".into(),
        origin: "LocalOperator".into(),
        purpose: "OpenHuman auth profile credential access".into(),
        access: prompt.access.clone(),
        name: prompt.name.clone(),
        expires_at_ms: prompt.expires_at_ms,
    }
}

#[test]
fn a_pending_prompt_is_listed_until_it_is_decided_or_expires() {
    let mut registry = PendingConsentRegistry::default();
    let timeout = std::time::Duration::from_secs(120);
    let (decided, _decided_waiter) = registry
        .register("write", "app-session:default", "call-decided", timeout)
        .unwrap();
    let (open, _open_waiter) = registry
        .register("read", "auth:openai:default", "call-open", timeout)
        .unwrap();
    for prompt in [&decided, &open] {
        registry
            .prompts
            .insert(prompt.request_id.clone(), shown(prompt));
    }

    let listed = registry.pending_prompts(now_ms());
    assert_eq!(listed.len(), 2);
    assert!(listed
        .iter()
        .all(|prompt| prompt.caller_agent == "openhuman.credentials"));

    assert!(registry.resolve(&decided.request_id, true));
    let listed = registry.pending_prompts(now_ms());
    assert_eq!(
        listed
            .iter()
            .map(|prompt| prompt.request_id.as_str())
            .collect::<Vec<_>>(),
        [open.request_id.as_str()]
    );

    assert!(registry.pending_prompts(open.expires_at_ms).is_empty());
    registry.cancel(&open.request_id);
    assert!(registry.pending_prompts(now_ms()).is_empty());
}

fn consent_request(call_id: &str, timeout_ms: u32) -> tinysecurity_bus::KeyringConsentRequest2 {
    tinysecurity_bus::KeyringConsentRequest2 {
        caller: tinysecurity_bus::CallerContext {
            // Not the credential store, which test builds answer.
            agent: "registry-test".into(),
            call_id: call_id.into(),
            ..Default::default()
        },
        purpose: "OpenHuman auth profile credential access".into(),
        access: tinysecurity_bus::KeyringAccess2::Write,
        name: "user-1:auth:app-session:default".into(),
        timeout_ms,
    }
}

#[tokio::test]
async fn unanswered_prompts_in_a_row_never_exhaust_the_capacity() {
    // The keychain takes each outcome once the module answers; more
    // unanswered prompts than the capacity must all still register.
    for index in 0..=MAX_PENDING_APPROVALS {
        let call_id = format!("unanswered-in-a-row-{index}");
        assert_eq!(request_approval(consent_request(&call_id, 1)).await, None);
        assert_eq!(
            take_operation_outcome(&call_id),
            Some(OperationOutcome::Pending),
            "prompt {index} registered and timed out"
        );
    }
}

#[test]
fn an_outcome_no_caller_took_frees_its_slot_once_stale() {
    let mut registry = PendingConsentRegistry::default();
    for index in 0..MAX_PENDING_APPROVALS {
        let call_id = format!("abandoned-{index}");
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
    for (_, decided_at) in registry.outcomes.values_mut() {
        *decided_at = decided_at.saturating_sub(OUTCOME_TTL_MS);
    }

    assert!(registry
        .register(
            "write",
            "auth:next",
            "call:next",
            std::time::Duration::from_secs(120)
        )
        .is_ok());
    assert!(registry.outcomes.is_empty());
}

#[tokio::test]
async fn a_caller_that_goes_away_releases_its_reservation() {
    let call_id = "caller-goes-away";
    let waiting = tokio::spawn(request_approval(consent_request(call_id, 60_000)));
    let reserved = || {
        pending()
            .lock()
            .unwrap()
            .reserved_call_ids
            .contains(call_id)
    };
    for _ in 0..200 {
        if reserved() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(5)).await;
    }
    assert!(reserved(), "the prompt registered");

    waiting.abort();
    assert!(waiting.await.unwrap_err().is_cancelled());
    assert!(!reserved());
    assert_eq!(take_operation_outcome(call_id), None);
}

#[tokio::test]
async fn a_test_build_answers_the_credential_stores_prompts() {
    let mut request = consent_request("credential-store-prompt", 5_000);
    request.caller.agent = "openhuman.credentials".into();
    // Unit tests approve unless the lane says otherwise.
    let expected = match std::env::var("OPENHUMAN_TEST_KEYRING_CONSENT").as_deref() {
        Ok("deny") => Some(false),
        Ok("approve") | Err(_) => Some(true),
        Ok(_) => None,
    };
    if expected.is_some() {
        assert_eq!(request_approval(request).await, expected);
    }
}
