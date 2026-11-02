use super::super::tier::{escalated_origin_for_nested_harness, escalated_origin_for_prompt};
use super::*;
use crate::config::HttpRequestConfig;
use crate::security::policy::CommandClass;
use crate::security::GateDecision;
use serde_json::json;
use std::sync::Arc;
use tinyagents_harness::tool::response_fields_from_schema;
use tinytools::status::POLICY_BLOCKED_MARKER;

/// The tier gate an `http_request` (Network-class) node calls: BLOCKED under
/// a read-only tier, and passed through (to the ApprovalGate) under
/// supervised/full.
#[tokio::test]
async fn http_request_node_tier_gate_blocks_readonly_allows_higher() {
    use crate::security::AutonomyLevel;

    let err = enforce_node_tier_gate(
        &policy(AutonomyLevel::ReadOnly),
        CommandClass::Network,
        "http_request",
    )
    .await
    .expect_err("read-only must block a Network-class http_request node");
    let EngineError::Capability(msg) = err;
    assert!(
        msg.contains(POLICY_BLOCKED_MARKER),
        "read-only block must carry the policy-blocked marker: {msg}"
    );

    // Supervised/full do not hard-block — they fall through to the
    // ApprovalGate (which performs the Prompt round-trip).
    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::Supervised),
        CommandClass::Network,
        "http_request"
    )
    .await
    .is_ok());
    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::Full),
        CommandClass::Network,
        "http_request"
    )
    .await
    .is_ok());
}

/// The tier gate a `code` (Write-class) node calls: BLOCKED under read-only,
/// allowed under full, prompt-able (not blocked) under supervised.
#[tokio::test]
async fn code_node_tier_gate_blocks_readonly_allows_full() {
    use crate::security::AutonomyLevel;

    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::ReadOnly),
        CommandClass::Write,
        "code"
    )
    .await
    .is_err());
    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::Supervised),
        CommandClass::Write,
        "code"
    )
    .await
    .is_ok());
    assert!(
        enforce_node_tier_gate(&policy(AutonomyLevel::Full), CommandClass::Write, "code")
            .await
            .is_ok()
    );
}

/// A long-lived code capability must observe a tier downgrade at the same
/// config path before it reaches code staging or sandbox execution.
/// End-to-end at the adapter: an `http_request` node under a read-only tier
/// is refused BEFORE any network egress (the tier gate fires ahead of the
/// approval gate, credential resolution, and dispatch).
#[tokio::test]
async fn http_adapter_blocks_under_readonly_tier() {
    use crate::security::AutonomyLevel;

    let (_dir, creds) = http_cred_store();
    let mut config = Config::default();
    config.autonomy.enabled = true;
    config.autonomy.level = AutonomyLevel::ReadOnly;
    let http = OpenHumanHttp {
        ctx: crate::core::runtime::DispatchCtx::unbound(),
        config: Arc::new(config),
        security: Arc::new(policy(AutonomyLevel::ReadOnly)),
        http_config: HttpRequestConfig::default(),
        http_creds: Arc::new(creds),
        approval: Default::default(),
    };

    let request = json!({ "method": "GET", "url": "https://example.com" });
    let err = http
        .request(request, None)
        .await
        .expect_err("read-only http_request node must be blocked");
    let EngineError::Capability(msg) = err;
    assert!(
        msg.contains(POLICY_BLOCKED_MARKER),
        "expected a policy-blocked refusal, got: {msg}"
    );
}

/// End-to-end at the adapter: a Composio `tool_call` node under a
/// read-only tier is refused BEFORE it ever reaches the curation gate or
/// any Composio dispatch — closes the compound bypass where the Composio
/// branch of `OpenHumanTools::invoke` reached `intercept_audited` without
/// ever consulting the autonomy tier, unlike the native `oh:`,
/// `http_request`, and `code` node paths, which all gate on tier first.
#[tokio::test]
async fn composio_tool_call_blocks_under_readonly_tier() {
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
    use crate::security::AutonomyLevel;

    let tools = OpenHumanTools {
        ctx: crate::core::runtime::DispatchCtx::unbound(),
        config: Arc::new(Config::default()),
        security: Arc::new(policy(AutonomyLevel::ReadOnly)),
        approval: Default::default(),
    };

    let err = tools
        .invoke("SLACK_SEND_MESSAGE", json!({}), None)
        .await
        .expect_err("read-only tier must block a Composio tool_call node before dispatch");
    let EngineError::Capability(msg) = err;
    assert!(
        msg.contains(POLICY_BLOCKED_MARKER),
        "expected a policy-blocked refusal, got: {msg}"
    );
}

// ── Effect-aware Composio tier gating (fixes reads parking as pending
// approvals): the tier gate must classify a Composio action by its
// curated [`ToolScope`], not blanket-treat every action as `Network`.
// Only a curated `Read` entry skips the prompt; curated `Write`/`Admin`,
// an uncurated toolkit, or an unparseable slug all still classify as
// `Network` (fail-safe — same class `http_request` uses).

/// A genuinely curated read (`TWITTER_RECENT_SEARCH`) must resolve to
/// `CommandClass::Read`, which `ReadOnly`'s gate matrix allows — closing
/// the bug where every Composio action (reads included) hard-blocked
/// under a read-only tier.
#[tokio::test]
async fn composio_read_action_allowed_under_readonly_tier() {
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
    use crate::security::AutonomyLevel;

    let class = classify_composio_action_for_tier("TWITTER_RECENT_SEARCH").await;
    assert_eq!(class, CommandClass::Read);
    assert_eq!(
        enforce_node_tier_gate(&policy(AutonomyLevel::ReadOnly), class, "tool_call")
            .await
            .expect("a curated Read action must not be blocked under ReadOnly"),
        GateDecision::Allow
    );

    // End-to-end: the adapter itself must not refuse before dispatch —
    // it may still fail downstream (no Composio session configured in
    // this test), but never with the policy-blocked marker.
    let tools = OpenHumanTools {
        ctx: crate::core::runtime::DispatchCtx::unbound(),
        config: Arc::new(Config::default()),
        security: Arc::new(policy(AutonomyLevel::ReadOnly)),
        approval: Default::default(),
    };
    let err = tools
        .invoke("TWITTER_RECENT_SEARCH", json!({}), None)
        .await
        .expect_err("no live Composio session is configured in this test");
    let EngineError::Capability(msg) = err;
    assert!(
        !msg.contains(POLICY_BLOCKED_MARKER),
        "a curated read must never be refused by the autonomy-tier gate, got: {msg}"
    );
}

/// A curated read under Supervised classifies as `CommandClass::Read`,
/// which the gate matrix always `Allow`s — so it can never trigger the
/// Supervised `Prompt` round-trip (the actual pending-approval bug: a
/// blanket `Network` classification prompted for every Composio call,
/// reads included).
#[tokio::test]
async fn composio_read_action_does_not_prompt_under_supervised_tier() {
    // Reaches the connector module, whose route is process-wide: hold the module
    // lock (before TEST_ENV_LOCK, the established order) for the whole test.
    let _module = crate::integrations::composio::module_client::module_guard().await;
    use crate::security::AutonomyLevel;

    let class = classify_composio_action_for_tier("TWITTER_RECENT_SEARCH").await;
    assert_eq!(class, CommandClass::Read);
    assert_eq!(
        enforce_node_tier_gate(&policy(AutonomyLevel::Supervised), class, "tool_call")
            .await
            .expect("a curated Read action must not be blocked under Supervised"),
        GateDecision::Allow,
        "a curated read must resolve to Allow, never Prompt, under Supervised"
    );

    let tools = OpenHumanTools {
        ctx: crate::core::runtime::DispatchCtx::unbound(),
        config: Arc::new(Config::default()),
        security: Arc::new(policy(AutonomyLevel::Supervised)),
        approval: Default::default(),
    };
    let err = tools
        .invoke("TWITTER_RECENT_SEARCH", json!({}), None)
        .await
        .expect_err("no live Composio session is configured in this test");
    let EngineError::Capability(msg) = err;
    assert!(
        !msg.contains(POLICY_BLOCKED_MARKER),
        "a curated read must pass the tier gate under Supervised, got: {msg}"
    );
}

/// Guard: a curated *write* action must still resolve to a
/// `Network`-class decision that `Prompt`s under Supervised — the
/// effect-aware classification must never widen who skips approval
/// beyond curated reads.
#[tokio::test]
async fn composio_write_action_still_prompts_under_supervised_tier() {
    use crate::security::AutonomyLevel;

    for slug in ["TWITTER_CREATION_OF_A_POST", "GMAIL_SEND_EMAIL"] {
        let class = classify_composio_action_for_tier(slug).await;
        assert_eq!(
            class,
            CommandClass::Network,
            "slug {slug} must classify as Network"
        );
        assert_eq!(
            enforce_node_tier_gate(&policy(AutonomyLevel::Supervised), class, "tool_call")
                .await
                .expect("a Network-class action is not blocked (only prompted) under Supervised"),
            GateDecision::Prompt,
            "slug {slug} must still require a Supervised-tier approval prompt"
        );
    }
}

/// Guard: an uncurated / unrecognized slug must fail safe to
/// `Network` (never `Read`) so it still prompts under Supervised and
/// blocks under ReadOnly — an agent can't dodge approval just by
/// calling a toolkit action OpenHuman hasn't curated yet.
#[tokio::test]
async fn composio_unknown_slug_prompts_under_supervised_tier() {
    use crate::security::AutonomyLevel;

    let class = classify_composio_action_for_tier("UNKNOWN_SERVICE_DO_THING").await;
    assert_eq!(class, CommandClass::Network);
    assert_eq!(
        enforce_node_tier_gate(&policy(AutonomyLevel::Supervised), class, "tool_call")
            .await
            .expect("Network-class is prompted, not blocked, under Supervised"),
        GateDecision::Prompt
    );
    assert!(
        enforce_node_tier_gate(&policy(AutonomyLevel::ReadOnly), class, "tool_call")
            .await
            .is_err()
    );
}

/// A `Prompt` tier decision on a default (`require_approval: false`)
/// workflow trust root escalates to `require_approval: true` — the forced
/// human-in-the-loop round trip that closes the Codex P1 finding.
#[test]
fn prompt_decision_escalates_default_workflow_origin() {
    let escalated =
        escalated_origin_for_prompt(GateDecision::Prompt, Some(workflow_origin("flow-1", false)))
            .expect("a Prompt decision on require_approval=false must escalate");
    assert!(matches!(
        escalated,
        AgentTurnOrigin::TrustedAutomation {
            source: TrustedAutomationSource::Workflow {
                require_approval: true
            },
            ..
        }
    ));
}

/// A flow that already opted into `require_approval: true` needs no
/// escalation — it's already forced through the parking flow.
#[test]
fn prompt_decision_does_not_re_escalate_already_gated_workflow() {
    assert!(escalated_origin_for_prompt(
        GateDecision::Prompt,
        Some(workflow_origin("flow-1", true))
    )
    .is_none());
}

/// An `Allow` tier decision never escalates, regardless of the workflow's
/// `require_approval` toggle — Full-tier runs keep running unattended.
#[test]
fn allow_decision_never_escalates() {
    assert!(escalated_origin_for_prompt(
        GateDecision::Allow,
        Some(workflow_origin("flow-1", false))
    )
    .is_none());
}

/// No scoped origin (or a non-Workflow origin) never escalates — there is
/// nothing to force through the workflow-specific parking flow.
#[test]
fn prompt_decision_does_not_escalate_without_a_workflow_origin() {
    assert!(escalated_origin_for_prompt(GateDecision::Prompt, None).is_none());
}

// ── Nested agent-node harness escalation (issue #4595) ─────────────────
//
// The `agent` node's harness turn runs the full agent tool loop, and the
// flow author never pre-declared the tool selection (only the `agent_ref`).
// So `escalated_origin_for_nested_harness` must escalate a default
// `Workflow { require_approval: false }` origin so
// `ApprovalGate::intercept_audited` can't apply its
// pre-declared-action `Allow` shortcut to tools the nested LLM picks at
// runtime.

/// A default `require_approval: false` workflow origin unconditionally
/// escalates: the nested harness's tool selection was not pre-declared, so
/// the trust-root shortcut in `ApprovalGate` must not apply. `job_id` is
/// preserved so the parked approval is still attributable to the flow run.
#[test]
fn nested_harness_escalates_default_workflow_origin_and_preserves_job_id() {
    let escalated = escalated_origin_for_nested_harness(Some(workflow_origin("flow-42", false)))
        .expect("a default require_approval=false workflow must escalate");
    match escalated {
        AgentTurnOrigin::TrustedAutomation {
            job_id,
            source:
                TrustedAutomationSource::Workflow {
                    require_approval: true,
                },
        } => assert_eq!(job_id, "flow-42"),
        other => panic!("expected escalated Workflow origin, got {other:?}"),
    }
}

/// A flow that already opted into `require_approval: true` needs no
/// escalation — the parking branch already applies.
#[test]
fn nested_harness_does_not_re_escalate_already_gated_workflow() {
    assert!(escalated_origin_for_nested_harness(Some(workflow_origin("flow-42", true,))).is_none());
}

/// A non-Workflow origin (Cron, Cli, WebChat, Unknown, …) passes through
/// unchanged: their own gate branches already make the right decision.
#[test]
fn nested_harness_does_not_escalate_non_workflow_origin() {
    assert!(
        escalated_origin_for_nested_harness(Some(AgentTurnOrigin::TrustedAutomation {
            job_id: "cron-1".into(),
            source: TrustedAutomationSource::Cron,
        }))
        .is_none()
    );
    assert!(escalated_origin_for_nested_harness(Some(AgentTurnOrigin::Cli)).is_none());
}

/// No scoped origin (unlabelled caller) passes through: the gate maps it
/// to `Unknown` and fails closed on external_effect tools already, so we
/// don't invent an escalation.
#[test]
fn nested_harness_does_not_escalate_without_an_origin() {
    assert!(escalated_origin_for_nested_harness(None).is_none());
}

// ── Issue #4868 — agent-node iteration cap + timeout scaling ───────────

/// Regression for issue #4868: the agent-node runtime path
/// (`OpenHumanAgentRunner::run_via_harness`) must build an `Agent` that
/// carries `agent_ref`'s definition's effective cap (50 for an
/// extended-policy agent), not the global `config.agent.max_tool_iterations`
/// default (10). This mirrors the exact build step `run_via_harness` takes
/// before dispatching the turn (so it doesn't require a live model
/// provider to exercise).
#[tokio::test]
async fn agent_node_runtime_resolves_to_the_definitions_effective_iteration_cap() {
    crate::agent::session_host::test_support::install_model_fixture();
    let tmp = tempfile::TempDir::new().unwrap();
    let config = agent_test_config(&tmp);
    assert_eq!(config.agent.max_tool_iterations, 10);

    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    let definitions = std::sync::Arc::new(
        crate::agent_registry::host_definition::AgentDefinitionRegistry::load(
            &config.workspace_dir,
        )
        .expect("agent registry init"),
    );
    let core = CoreContext::for_test(DomainSet::full(), Some(config.workspace_dir.clone()));
    let context = core.derive_with(
        ContextOverlay::new(
            config.clone(),
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .definitions(std::sync::Arc::clone(&definitions)),
    );

    CoreContext::scope(context, async {
        let agent_ref = "task_manager_agent";
        let def = definitions
            .get(agent_ref)
            .expect("task manager definition registered")
            .clone();
        let expected = def.effective_max_iterations();
        assert_eq!(expected, 50);

        let agent = crate::agent::OpenHumanSessionHost::from_config_for_agent(
            &crate::core::runtime::DispatchCtx::unbound(),
            &config,
            agent_ref,
        )
        .await
        .expect("build planner agent");
        assert_eq!(agent.agent_config().max_tool_iterations, expected);

        let runner = OpenHumanAgentRunner {
            config: Arc::new(config),
            ctx: crate::core::runtime::DispatchCtx::unbound(),
            approval: Default::default(),
            progress: None,
        };
        let prepared = runner
            .prepare_agent_run(json!({"prompt":"work"}), None, None, 50)
            .await
            .expect("module prepares agent timeout");
        assert_eq!(prepared.base_timeout_secs, 240);
        assert_eq!(prepared.timeout_secs, 600);
    })
    .await;
}

// ── response_fields_from_schema ─────────────────────────────────────────
// Direct unit tests for the pure schema-extraction step inside
// `composio_response_fields`'s live-fetch loop — cheaper and more
// targeted than exercising the whole `composio_list_tools` round trip,
// and covers the schema shapes that loop actually has to handle.

#[test]
fn response_fields_from_schema_reads_standard_properties_object() {
    let schema = json!({
        "type": "object",
        "properties": { "id": {"type": "string"}, "threadId": {"type": "string"} }
    });
    assert_eq!(
        response_fields_from_schema(Some(&schema)),
        vec!["id".to_string(), "threadId".to_string()]
    );
}

#[test]
fn response_fields_from_schema_reads_nested_data_error_wrapper_as_top_level_keys() {
    // A `{data, error}` envelope has no special unwrapping — the function
    // documents (and this test locks in) that it reports the schema's own
    // top-level property names, not the fields nested inside `data`.
    let schema = json!({
        "type": "object",
        "properties": {
            "data": {"type": "object", "properties": {"id": {"type": "string"}}},
            "error": {"type": "string"}
        }
    });
    assert_eq!(
        response_fields_from_schema(Some(&schema)),
        vec!["data".to_string(), "error".to_string()]
    );
}

#[test]
fn response_fields_from_schema_falls_back_to_top_level_keys_minus_schema_keywords() {
    // Legacy/loose shape with no `properties` wrapper: falls back to the
    // schema object's own keys, filtering out JSON-Schema keywords.
    let schema = json!({
        "type": "object",
        "description": "legacy shape",
        "id": {"type": "string"},
        "threadId": {"type": "string"}
    });
    assert_eq!(
        response_fields_from_schema(Some(&schema)),
        vec!["id".to_string(), "threadId".to_string()]
    );
}

#[test]
fn response_fields_from_schema_empty_for_none_or_non_object() {
    assert!(response_fields_from_schema(None).is_empty());
    assert!(response_fields_from_schema(Some(&json!("not an object"))).is_empty());
    assert!(response_fields_from_schema(Some(&json!({}))).is_empty());
}

#[path = "ops_tier_live_config_tests.rs"]
mod live_config_tests;
