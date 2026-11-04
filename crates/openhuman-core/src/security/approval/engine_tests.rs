use super::*;

struct Nobody;

#[async_trait]
impl PromptSurface for Nobody {
    async fn deliver(&self, _delivery: Delivery) -> Result<(), String> {
        Err("no surface".into())
    }
}

fn input() -> CheckInput {
    CheckInput {
        tenant: Tenant::default(),
        variant: Variant::Standard,
        posture: Posture {
            require_approval: true,
            ..Posture::default()
        },
        legacy_tool_grants: Vec::new(),
        call_id: "00000001".into(),
        tool: "gmail_send".into(),
        args: serde_json::json!({}),
        summary: "send".into(),
        thread: None,
        flow: None,
        origin: CallOrigin::Operator,
        ttl: Some(Duration::from_secs(600)),
        prompt: Arc::new(Nobody),
    }
}

#[test]
fn each_posture_variant_is_its_own_subject() {
    let suffixes = [
        Variant::Standard.suffix(),
        Variant::FlowStrict.suffix(),
        Variant::Forced.suffix(),
    ];
    assert_eq!(suffixes[0], "");
    assert_ne!(suffixes[1], suffixes[2]);
    assert!(suffixes[1].starts_with('#') && suffixes[2].starts_with('#'));
}

#[test]
fn the_default_posture_requires_approval_for_nothing_it_does_not_name() {
    let posture = Posture::default();
    assert!(!posture.auto_approve_all);
}

#[tokio::test]
async fn a_build_without_the_module_refuses_every_gated_call() {
    let engine = UnavailableEngine;
    assert!(matches!(
        engine.check(input()).await,
        Err(EngineError::Unavailable(message)) if message.contains("security-module")
    ));
    assert!(engine.reserve("r").await.is_err());
    assert!(engine
        .record_execution("r", super::super::types::ExecutionOutcome::Success)
        .await
        .is_err());
    assert!(engine.expire("r").await.is_err());
    assert!(engine.cancel("r").await.is_err());
    // Nothing is pending, so a decision finds nothing; history is empty, not an error.
    assert_eq!(
        engine.decide("r", Choice::Once, "approve_once").await,
        Err(EngineError::NotPending)
    );
    assert_eq!(engine.is_decided("r").await, Ok(None));
    assert!(engine.history(5).await.unwrap().is_empty());
    engine.release("r");
}

#[tokio::test]
async fn whole_tool_trust_stays_with_the_exact_tenant_and_variant() {
    let engine = super::super::engine_stub::StubEngine::new();
    let trusted = Tenant {
        profile: Some("alice".into()),
        agent: Some("agent-a".into()),
    };
    engine
        .grant_tool_trust(&trusted, Variant::Standard, "gmail_send")
        .await
        .unwrap();

    let exact = CheckInput {
        tenant: trusted.clone(),
        ..input()
    };
    assert!(matches!(
        engine.check(exact).await.unwrap(),
        Checked::Approved { .. }
    ));

    // The grant belongs to one agent's standard approval scope. Another agent,
    // the profile process, or a stricter posture must still prompt.
    for (tenant, variant) in [
        (
            Tenant {
                profile: Some("alice".into()),
                agent: Some("agent-b".into()),
            },
            Variant::Standard,
        ),
        (
            Tenant {
                profile: Some("alice".into()),
                agent: None,
            },
            Variant::Standard,
        ),
        (trusted.clone(), Variant::FlowStrict),
    ] {
        assert!(matches!(
            engine
                .check(CheckInput {
                    tenant,
                    variant,
                    ..input()
                })
                .await,
            Err(EngineError::Unavailable(_))
        ));
    }
}
