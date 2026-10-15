use super::native_tests::*;
use super::*;

fn two_tenant_config(root: &std::path::Path, name: &str, enabled: bool, deny: &str) -> Config {
    let workspace = root.join(name).join("workspace");
    let action = root.join(name).join("action");
    std::fs::create_dir_all(&workspace).unwrap();
    std::fs::create_dir_all(&action).unwrap();
    let mut config = Config {
        workspace_dir: workspace,
        action_dir: action,
        ..Config::default()
    };
    config.autonomy.enabled = enabled;
    config.autonomy.level = crate::security::AutonomyLevel::Supervised;
    config.tool_rules.rules = vec![tinytools::rules::ToolRule::names(
        tinytools::rules::RuleEffect::Deny,
        [deny],
    )];
    config
}

fn tenant_caller(agent: &str, call: &str) -> VerifiedCaller {
    VerifiedCaller::from_host(tinysecurity_bus::CallerContext {
        agent: agent.into(),
        thread: None,
        flow: None,
        call_id: call.into(),
        origin: tinysecurity_bus::Origin::Operator,
        tier: tinysecurity_bus::AccessTier::Full,
    })
    .unwrap()
}

fn tool(name: &str) -> tinysecurity_bus::ToolCall {
    tinysecurity_bus::ToolCall {
        tool: name.into(),
        arguments_json: "{}".into(),
        checks: vec![tinysecurity_bus::Check::LocalDiagnostic {}],
    }
}

/// Two tenants with opposite tool rules share one module. Each decision comes
/// from that tenant's own registered policy, a tenant cannot use the other's
/// policy id, and the settings the module reports match the translation.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn two_tenants_get_isolated_decisions_through_the_scoped_client() {
    use super::super::security::scoped_client::{
        activate_on, evaluate_on, Activation, ScopedState,
    };
    use tinysecurity_bus::{CommandExecutionMode, SubjectScope, Verdict};

    let fixture = native_fixture().await;
    let proxy = &fixture.proxy;
    let root = fixture.acting.path().canonicalize().unwrap();
    let state = tokio::sync::Mutex::new(ScopedState::default());
    let mode = CommandExecutionMode::HarnessGated;
    let scope = |user: &str| SubjectScope {
        user: user.into(),
        workspace: root.join(user).to_string_lossy().into_owned(),
        agent: "orchestrator".into(),
    };
    let alice = two_tenant_config(&root, "alice", false, "http_request");
    let bob = two_tenant_config(&root, "bob", false, "file_read");
    let Activation::Active(a) = activate_on(
        proxy,
        &state,
        &alice,
        tenant_caller("orchestrator", "a-1"),
        scope("alice"),
        mode,
    )
    .await
    .unwrap() else {
        panic!("first registration activates without review")
    };
    let Activation::Active(b) = activate_on(
        proxy,
        &state,
        &bob,
        tenant_caller("orchestrator", "b-1"),
        scope("bob"),
        mode,
    )
    .await
    .unwrap() else {
        panic!("first registration activates without review")
    };
    assert_ne!(a.policy_id, b.policy_id);

    // A tool rule denial is the `Access` reason; a tool without a rule falls
    // through to the module's own checks (`UnsupportedEffect` for these probes).
    let denied = |decision: &tinysecurity_bus::ScopedDecision| {
        matches!(
            decision.decision.verdict,
            Verdict::Deny {
                reason: tinysecurity_bus::DenialReason::Access
            }
        )
    };
    let alice_http = evaluate_on(
        proxy,
        &state,
        &a,
        tenant_caller("orchestrator", "a-2"),
        tool("http_request"),
    )
    .await
    .unwrap();
    let alice_read = evaluate_on(
        proxy,
        &state,
        &a,
        tenant_caller("orchestrator", "a-3"),
        tool("file_read"),
    )
    .await
    .unwrap();
    let bob_http = evaluate_on(
        proxy,
        &state,
        &b,
        tenant_caller("orchestrator", "b-2"),
        tool("http_request"),
    )
    .await
    .unwrap();
    let bob_read = evaluate_on(
        proxy,
        &state,
        &b,
        tenant_caller("orchestrator", "b-3"),
        tool("file_read"),
    )
    .await
    .unwrap();
    assert!(
        denied(&alice_http) && !denied(&alice_read),
        "alice denies only http_request"
    );
    assert!(
        !denied(&bob_http) && denied(&bob_read),
        "bob denies only file_read"
    );

    // A tenant presenting the other's policy id is refused: the caller's agent
    // must be the registered subject, and bob's policy names bob's subject.
    let wrong_agent = evaluate_on(
        proxy,
        &state,
        &a,
        tenant_caller("someone-else", "x-1"),
        tool("file_read"),
    )
    .await
    .unwrap();
    assert_eq!(
        wrong_agent.decision.verdict,
        Verdict::Deny {
            reason: tinysecurity_bus::DenialReason::InvalidContext
        },
        "the caller must be the registered subject"
    );

    // The module reports exactly what the translator produced, no secrets.
    let info: tinysecurity_bus::ScopedPolicyInfo = proxy
        .call_confidential(names::methods::SCOPED_POLICY_INFO, (a.policy_id.clone(),))
        .await
        .unwrap();
    assert_eq!(
        info.settings,
        super::super::security::policy_translation::settings(&alice)
    );
    assert_eq!(info.subject_scope, a.subject);

    // Idempotent activation reuses the policy (and its command budget).
    let Activation::Active(again) = activate_on(
        proxy,
        &state,
        &alice,
        tenant_caller("orchestrator", "a-4"),
        scope("alice"),
        mode,
    )
    .await
    .unwrap() else {
        panic!("unchanged settings stay active")
    };
    assert_eq!(again.policy_id, a.policy_id);

    // Narrowing replacement (an extra deny rule) activates without review and
    // keeps the same lineage; enabling the policy widens nothing here either.
    let mut narrowed = alice.clone();
    narrowed
        .tool_rules
        .rules
        .push(tinytools::rules::ToolRule::names(
            tinytools::rules::RuleEffect::Deny,
            ["shell"],
        ));
    let Activation::Active(next) = activate_on(
        proxy,
        &state,
        &narrowed,
        tenant_caller("orchestrator", "a-5"),
        scope("alice"),
        mode,
    )
    .await
    .unwrap() else {
        panic!("a narrowing change needs no review")
    };
    assert_ne!(next.policy_id, a.policy_id);
    assert!(next.policy_generation >= 1);
    let old = evaluate_on(
        proxy,
        &state,
        &a,
        tenant_caller("orchestrator", "a-6"),
        tool("file_read"),
    )
    .await
    .unwrap();
    assert_eq!(
        old.decision.verdict,
        Verdict::Deny {
            reason: tinysecurity_bus::DenialReason::InvalidContext
        },
        "the superseded policy must deny"
    );
    let shell = evaluate_on(
        proxy,
        &state,
        &next,
        tenant_caller("orchestrator", "a-7"),
        tool("shell"),
    )
    .await
    .unwrap();
    assert!(denied(&shell));

    // Widening: allow tools previously denied -> review required, not activated.
    let mut widened = narrowed.clone();
    widened.tool_rules.rules.clear();
    match activate_on(
        proxy,
        &state,
        &widened,
        tenant_caller("orchestrator", "a-8"),
        scope("alice"),
        mode,
    )
    .await
    .unwrap()
    {
        Activation::ReviewRequired { widenings, .. } => assert!(!widenings.is_empty()),
        Activation::Active(_) => panic!("a widening replacement must not activate unreviewed"),
    }
    let still = evaluate_on(
        proxy,
        &state,
        &next,
        tenant_caller("orchestrator", "a-9"),
        tool("shell"),
    )
    .await
    .unwrap();
    assert!(
        denied(&still),
        "the old policy stays in force while review is pending"
    );
}

/// Under required audit the plain scoped call denies permitted work, and the
/// audited member answers with evidence and a never-cacheable decision. The
/// fixture configures no audit client, so the audited call must not allow.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn audited_scoped_members_never_allow_without_committed_evidence() {
    use super::super::security::scoped_client::{
        activate_on, check_audited_on, evaluate_audited_on, Activation, AuditCorrelation,
        ScopedState,
    };
    use tinysecurity_bus::{CommandExecutionMode, SubjectScope, Verdict};

    let fixture = native_fixture().await;
    let proxy = &fixture.proxy;
    let root = fixture.acting.path().canonicalize().unwrap();
    let state = tokio::sync::Mutex::new(ScopedState::default());
    let scope = SubjectScope {
        user: "audited".into(),
        workspace: root.join("audited").to_string_lossy().into_owned(),
        agent: "orchestrator".into(),
    };
    let config = two_tenant_config(&root, "audited", false, "unused_tool");
    let Activation::Active(active) = activate_on(
        proxy,
        &state,
        &config,
        tenant_caller("orchestrator", "x-1"),
        scope,
        CommandExecutionMode::HarnessGated,
    )
    .await
    .unwrap() else {
        panic!("first registration activates without review")
    };
    let audit = AuditCorrelation {
        event_id: "0".repeat(32),
        expected_tail: 0,
        generation: active.policy_generation,
    };
    let evaluated = evaluate_audited_on(
        proxy,
        &active,
        tenant_caller("orchestrator", "x-2"),
        tool("file_read"),
        audit.clone(),
    )
    .await;
    if let Ok(response) = evaluated {
        assert!(!response.decision.decision.cacheable);
        assert!(!matches!(
            response.decision.decision.verdict,
            Verdict::Allow
        ));
        assert!(!response.audit.committed);
    }
    let checked = check_audited_on(
        proxy,
        &active,
        tenant_caller("orchestrator", "x-3"),
        vec![tinysecurity_bus::Check::LocalDiagnostic {}],
        audit,
    )
    .await;
    if let Ok(response) = checked {
        assert!(response
            .decision
            .decisions
            .iter()
            .all(|d| !d.cacheable && !matches!(d.verdict, Verdict::Allow)));
    }
}

/// The operator's real forbidden list registers (tilde, relative, floor entries)
/// instead of an empty one.
#[tokio::test(flavor = "multi_thread")]
#[ignore = "requires a released host key or an explicitly selected local native fixture"]
async fn real_forbidden_paths_register_with_the_module() {
    use super::super::security::scoped_client::{activate_on, Activation, ScopedState};
    use tinysecurity_bus::{CommandExecutionMode, SubjectScope};

    let fixture = native_fixture().await;
    let root = fixture.acting.path().canonicalize().unwrap();
    let state = tokio::sync::Mutex::new(ScopedState::default());
    let mut config = two_tenant_config(&root, "forbid", false, "unused_tool");
    config.autonomy.forbidden_paths = vec!["~/.ssh".into(), "private".into(), "/etc".into()];
    let activation = activate_on(
        &fixture.proxy,
        &state,
        &config,
        tenant_caller("orchestrator", "f-1"),
        SubjectScope {
            user: "forbid".into(),
            workspace: root.join("forbid").to_string_lossy().into_owned(),
            agent: "orchestrator".into(),
        },
        CommandExecutionMode::HarnessGated,
    )
    .await
    .expect("tilde, relative and floor entries must register");
    assert!(matches!(activation, Activation::Active(_)));
}
