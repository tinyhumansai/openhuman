use super::*;

/// The tier gate an `http_request` (Network-class) node calls: BLOCKED under
/// a read-only tier, and passed through (to the ApprovalGate) under
/// supervised/full.
#[test]
fn http_request_node_tier_gate_blocks_readonly_allows_higher() {
    use crate::openhuman::security::AutonomyLevel;

    let err = enforce_node_tier_gate(
        &policy(AutonomyLevel::ReadOnly),
        CommandClass::Network,
        "http_request",
    )
    .expect_err("read-only must block a Network-class http_request node");
    if let EngineError::Capability(msg) = err {
        assert!(
            msg.contains(POLICY_BLOCKED_MARKER),
            "read-only block must carry the policy-blocked marker: {msg}"
        );
    } else {
        panic!("expected EngineError::Capability for a blocked node");
    }

    // Supervised/full do not hard-block — they fall through to the
    // ApprovalGate (which performs the Prompt round-trip).
    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::Supervised),
        CommandClass::Network,
        "http_request"
    )
    .is_ok());
    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::Full),
        CommandClass::Network,
        "http_request"
    )
    .is_ok());
}

/// The tier gate a `code` (Write-class) node calls: BLOCKED under read-only,
/// allowed under full, prompt-able (not blocked) under supervised.
#[test]
fn code_node_tier_gate_blocks_readonly_allows_full() {
    use crate::openhuman::security::AutonomyLevel;

    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::ReadOnly),
        CommandClass::Write,
        "code"
    )
    .is_err());
    assert!(enforce_node_tier_gate(
        &policy(AutonomyLevel::Supervised),
        CommandClass::Write,
        "code"
    )
    .is_ok());
    assert!(
        enforce_node_tier_gate(&policy(AutonomyLevel::Full), CommandClass::Write, "code").is_ok()
    );
}

/// End-to-end at the adapter: an `http_request` node under a read-only tier
/// is refused BEFORE any network egress (the tier gate fires ahead of the
/// approval gate, credential resolution, and dispatch).
#[tokio::test]
async fn http_adapter_blocks_under_readonly_tier() {
    use crate::openhuman::security::AutonomyLevel;

    let (_dir, creds) = http_cred_store();
    let http = OpenHumanHttp {
        security: Arc::new(policy(AutonomyLevel::ReadOnly)),
        http_config: HttpRequestConfig::default(),
        http_creds: Arc::new(creds),
    };

    let request = json!({ "method": "GET", "url": "https://example.com" });
    let err = http
        .request(request, None)
        .await
        .expect_err("read-only http_request node must be blocked");
    if let EngineError::Capability(msg) = err {
        assert!(
            msg.contains(POLICY_BLOCKED_MARKER),
            "expected a policy-blocked refusal, got: {msg}"
        );
    } else {
        panic!("expected EngineError::Capability");
    }
}

/// End-to-end at the adapter: a Composio `tool_call` node under a
/// read-only tier is refused BEFORE it ever reaches the curation gate or
/// any Composio dispatch — closes the compound bypass where the Composio
/// branch of `OpenHumanTools::invoke` reached `intercept_audited` without
/// ever consulting the autonomy tier, unlike the native `oh:`,
/// `http_request`, and `code` node paths, which all gate on tier first.
#[tokio::test]
async fn composio_tool_call_blocks_under_readonly_tier() {
    use crate::openhuman::security::AutonomyLevel;

    let tools = OpenHumanTools {
        config: Arc::new(Config::default()),
        security: Arc::new(policy(AutonomyLevel::ReadOnly)),
    };

    let err = tools
        .invoke("SLACK_SEND_MESSAGE", json!({}), None)
        .await
        .expect_err("read-only tier must block a Composio tool_call node before dispatch");
    if let EngineError::Capability(msg) = err {
        assert!(
            msg.contains(POLICY_BLOCKED_MARKER),
            "expected a policy-blocked refusal, got: {msg}"
        );
    } else {
        panic!("expected EngineError::Capability");
    }
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
    use crate::openhuman::security::AutonomyLevel;

    let class = classify_composio_action_for_tier("TWITTER_RECENT_SEARCH").await;
    assert_eq!(class, CommandClass::Read);
    assert_eq!(
        enforce_node_tier_gate(&policy(AutonomyLevel::ReadOnly), class, "tool_call")
            .expect("a curated Read action must not be blocked under ReadOnly"),
        GateDecision::Allow
    );

    // End-to-end: the adapter itself must not refuse before dispatch —
    // it may still fail downstream (no Composio session configured in
    // this test), but never with the policy-blocked marker.
    let tools = OpenHumanTools {
        config: Arc::new(Config::default()),
        security: Arc::new(policy(AutonomyLevel::ReadOnly)),
    };
    let err = tools
        .invoke("TWITTER_RECENT_SEARCH", json!({}), None)
        .await
        .expect_err("no live Composio session is configured in this test");
    if let EngineError::Capability(msg) = err {
        assert!(
            !msg.contains(POLICY_BLOCKED_MARKER),
            "a curated read must never be refused by the autonomy-tier gate, got: {msg}"
        );
    } else {
        panic!("expected EngineError::Capability");
    }
}

/// A curated read under Supervised classifies as `CommandClass::Read`,
/// which the gate matrix always `Allow`s — so it can never trigger the
/// Supervised `Prompt` round-trip (the actual pending-approval bug: a
/// blanket `Network` classification prompted for every Composio call,
/// reads included).
#[tokio::test]
async fn composio_read_action_does_not_prompt_under_supervised_tier() {
    use crate::openhuman::security::AutonomyLevel;

    let class = classify_composio_action_for_tier("TWITTER_RECENT_SEARCH").await;
    assert_eq!(class, CommandClass::Read);
    assert_eq!(
        enforce_node_tier_gate(&policy(AutonomyLevel::Supervised), class, "tool_call")
            .expect("a curated Read action must not be blocked under Supervised"),
        GateDecision::Allow,
        "a curated read must resolve to Allow, never Prompt, under Supervised"
    );

    let tools = OpenHumanTools {
        config: Arc::new(Config::default()),
        security: Arc::new(policy(AutonomyLevel::Supervised)),
    };
    let err = tools
        .invoke("TWITTER_RECENT_SEARCH", json!({}), None)
        .await
        .expect_err("no live Composio session is configured in this test");
    if let EngineError::Capability(msg) = err {
        assert!(
            !msg.contains(POLICY_BLOCKED_MARKER),
            "a curated read must pass the tier gate under Supervised, got: {msg}"
        );
    } else {
        panic!("expected EngineError::Capability");
    }
}

/// Guard: a curated *write* action must still resolve to a
/// `Network`-class decision that `Prompt`s under Supervised — the
/// effect-aware classification must never widen who skips approval
/// beyond curated reads.
#[tokio::test]
async fn composio_write_action_still_prompts_under_supervised_tier() {
    use crate::openhuman::security::AutonomyLevel;

    for slug in ["TWITTER_CREATION_OF_A_POST", "GMAIL_SEND_EMAIL"] {
        let class = classify_composio_action_for_tier(slug).await;
        assert_eq!(
            class,
            CommandClass::Network,
            "slug {slug} must classify as Network"
        );
        assert_eq!(
            enforce_node_tier_gate(&policy(AutonomyLevel::Supervised), class, "tool_call")
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
    use crate::openhuman::security::AutonomyLevel;

    let class = classify_composio_action_for_tier("UNKNOWN_SERVICE_DO_THING").await;
    assert_eq!(class, CommandClass::Network);
    assert_eq!(
        enforce_node_tier_gate(&policy(AutonomyLevel::Supervised), class, "tool_call")
            .expect("Network-class is prompted, not blocked, under Supervised"),
        GateDecision::Prompt
    );
    assert!(enforce_node_tier_gate(&policy(AutonomyLevel::ReadOnly), class, "tool_call").is_err());
}

/// Unit coverage of the classifier itself, independent of the gate: a
/// curated Read entry classifies as `Read`; curated Write/Admin entries,
/// an uncurated toolkit, and an unparseable/empty slug all classify as
/// `Network` (fail-safe default — never silently widen to Read).
#[tokio::test]
async fn classify_composio_action_for_tier_matches_curated_scope_fail_safe() {
    assert_eq!(
        classify_composio_action_for_tier("TWITTER_RECENT_SEARCH").await,
        CommandClass::Read
    );
    assert_eq!(
        classify_composio_action_for_tier("TWITTER_CREATION_OF_A_POST").await,
        CommandClass::Network
    );
    assert_eq!(
        classify_composio_action_for_tier("TWITTER_POST_DELETE_BY_POST_ID").await,
        CommandClass::Network
    );
    // Uncurated toolkit (no catalog at all for "unknown").
    assert_eq!(
        classify_composio_action_for_tier("UNKNOWN_SERVICE_DO_THING").await,
        CommandClass::Network
    );
    // Unparseable / empty slug.
    assert_eq!(
        classify_composio_action_for_tier("").await,
        CommandClass::Network
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

#[test]
fn scale_timeout_for_iteration_cap_leaves_default_cap_unscaled() {
    // An agent whose effective cap is at or below the old global default
    // (10) doesn't need extra wall-clock time.
    assert_eq!(scale_timeout_for_iteration_cap(240, 10), 240);
    assert_eq!(scale_timeout_for_iteration_cap(240, 3), 240);
}

#[test]
fn scale_timeout_for_iteration_cap_scales_extended_agents_up() {
    // 50 iterations * 12s/iter = 600s, exactly the existing ceiling.
    assert_eq!(scale_timeout_for_iteration_cap(240, 50), 600);
}

#[test]
fn scale_timeout_for_iteration_cap_never_lowers_an_explicit_request() {
    // A caller-requested timeout higher than the scaled floor must win.
    assert_eq!(scale_timeout_for_iteration_cap(600, 50), 600);
}

#[test]
fn scale_timeout_for_iteration_cap_caps_at_600_even_for_very_high_iteration_counts() {
    assert_eq!(scale_timeout_for_iteration_cap(240, 200), 600);
}

/// Post-merge Codex P2 finding on issue #4868: an explicit `timeout_secs`
/// the node config supplied (a caller-chosen fast-fail/SLA bound) must be
/// honored as-is — never scaled up just because the agent's iteration cap
/// is high — while the absence of one still gets the iteration-cap
/// scaling so a 50-iteration agent isn't killed by the 240s default.
#[test]
fn resolve_run_timeout_secs_preserves_an_explicit_request_even_for_a_high_cap_agent() {
    assert_eq!(resolve_run_timeout_secs(Some(120), 50), 120);
}

#[test]
fn resolve_run_timeout_secs_scales_the_default_up_for_a_high_cap_agent() {
    // No explicit timeout_secs (None) -> default 240s, scaled by the
    // 50-iteration cap to min(50*12, 600) = 600.
    assert_eq!(resolve_run_timeout_secs(None, 50), 600);
}

#[test]
fn resolve_run_timeout_secs_leaves_low_cap_agents_unscaled_either_way() {
    assert_eq!(resolve_run_timeout_secs(None, 10), 240);
    assert_eq!(resolve_run_timeout_secs(Some(120), 10), 120);
}

/// Regression for issue #4868: the agent-node runtime path
/// (`OpenHumanAgentRunner::run_via_harness`) must build an `Agent` that
/// carries `agent_ref`'s definition's effective cap (50 for an
/// extended-policy agent), not the global `config.agent.max_tool_iterations`
/// default (10). This mirrors the exact build step `run_via_harness` takes
/// before dispatching the turn (so it doesn't require a live model
/// provider to exercise).
#[test]
fn agent_node_runtime_resolves_to_the_definitions_effective_iteration_cap() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = resolver_test_config(&tmp);
    assert_eq!(config.agent.max_tool_iterations, 10);

    crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global(
        &config.workspace_dir,
    )
    .expect("agent registry init");
    let def = crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
        .expect("registry initialised")
        .get("code_executor")
        .expect("code_executor definition registered")
        .clone();
    let expected = def.effective_max_iterations();
    assert_eq!(expected, 50);

    let agent = crate::openhuman::agent::Agent::from_config_for_agent(&config, "code_executor")
        .expect("build code_executor agent");
    assert_eq!(agent.agent_config().max_tool_iterations, expected);

    // And the timeout scaling this cap feeds into actually widens the
    // default 240s bound for this node.
    let base_timeout = clamp_run_timeout_secs(None);
    assert_eq!(base_timeout, 240);
    let scaled =
        scale_timeout_for_iteration_cap(base_timeout, agent.agent_config().max_tool_iterations);
    assert_eq!(scaled, 600);
}

/// The resolver loads a saved flow's graph by its id — the by-`workflow_id`
/// sub_workflow path resolves against the real flows store.
#[tokio::test]
async fn resolver_loads_saved_flow_graph_by_id() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Arc::new(resolver_test_config(&tmp));

    let graph_json = serde_json::to_value(trigger_only_graph()).unwrap();
    let flow = flows::ops::flows_create(&config, "child".to_string(), graph_json, false)
        .await
        .expect("create flow");
    let flow_id = flow.value.id.clone();

    let resolver = OpenHumanWorkflowResolver {
        config: config.clone(),
    };
    let graph = resolver
        .resolve(&flow_id)
        .await
        .expect("resolver should load the saved flow graph");
    assert_eq!(graph.nodes.len(), 1);
    assert_eq!(graph.nodes[0].id, "t");
}

/// An unknown workflow_id surfaces a capability error naming the id, rather
/// than silently resolving to nothing.
#[tokio::test]
async fn resolver_unknown_id_is_a_capability_error() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Arc::new(resolver_test_config(&tmp));
    let resolver = OpenHumanWorkflowResolver { config };

    let err = resolver
        .resolve("does-not-exist")
        .await
        .expect_err("unknown workflow_id must error");
    match err {
        EngineError::Capability(msg) => assert!(
            msg.contains("does-not-exist"),
            "error should name the missing id: {msg}"
        ),
        other => panic!("expected a capability error, got: {other:?}"),
    }
}

#[tokio::test]
async fn resolver_rejects_an_engine_incompatible_saved_graph() {
    let tmp = tempfile::TempDir::new().unwrap();
    let config = Arc::new(resolver_test_config(&tmp));
    let flow = flows::ops::flows_create(
        &config,
        "legacy child".to_string(),
        serde_json::to_value(trigger_only_graph()).unwrap(),
        false,
    )
    .await
    .unwrap()
    .value;
    let unsafe_graph = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Trigger" },
            { "id": "outer", "kind": "condition", "name": "Outer", "config": { "field": "outer" } },
            { "id": "inner", "kind": "condition", "name": "Inner", "config": { "field": "inner" } },
            { "id": "outer_else", "kind": "output_parser", "name": "Outer else" },
            { "id": "inner_else", "kind": "output_parser", "name": "Inner else" },
            { "id": "a", "kind": "output_parser", "name": "A" },
            { "id": "c", "kind": "output_parser", "name": "C" },
            { "id": "m", "kind": "merge", "name": "Merge" }
        ],
        "edges": [
            { "from_node": "t", "from_port": "main", "to_node": "outer" },
            { "from_node": "t", "from_port": "main", "to_node": "c" },
            { "from_node": "outer", "from_port": "true", "to_node": "inner" },
            { "from_node": "outer", "from_port": "false", "to_node": "outer_else" },
            { "from_node": "inner", "from_port": "true", "to_node": "a" },
            { "from_node": "inner", "from_port": "false", "to_node": "inner_else" },
            { "from_node": "a", "from_port": "main", "to_node": "m" },
            { "from_node": "c", "from_port": "main", "to_node": "m" }
        ]
    });
    let db = config.workspace_dir.join("flows").join("flows.db");
    rusqlite::Connection::open(db)
        .unwrap()
        .execute(
            "UPDATE flow_definitions SET graph_json = ?1 WHERE id = ?2",
            rusqlite::params![unsafe_graph.to_string(), flow.id],
        )
        .unwrap();

    let error = OpenHumanWorkflowResolver { config }
        .resolve(&flow.id)
        .await
        .expect_err("resolver must reject an incompatible legacy child");
    match error {
        EngineError::Capability(message) => assert!(
            message.contains("unsupported_nested_conditional_fan_in"),
            "{message}"
        ),
        other => panic!("expected a capability error, got: {other:?}"),
    }
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

// ── unsupported_arg_names (B13) ──────────────────────────────────────────
// Direct unit tests for the pure name-validity check — see
// `openhuman::flows::ops_tests` for the end-to-end
// `validate_tool_contracts` coverage of the same behavior.

#[test]
fn unsupported_arg_names_flags_a_name_not_in_properties() {
    let schema = json!({
        "type": "object",
        "properties": { "channel": {"type": "string"}, "markdown_text": {"type": "string"} }
    });
    let args = json!({ "channel": "#general", "text": "hi" });
    assert_eq!(
        unsupported_arg_names(Some(&schema), &args),
        Some(vec!["text".to_string()])
    );
}
