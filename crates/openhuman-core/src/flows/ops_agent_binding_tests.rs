use super::*;

// ── validate_agent_refs (agent-ref resolvability gate, PR #5114) ───────────

#[tokio::test]
async fn agent_ref_plain_node_without_ref_is_accepted() {
    // A plain `agent` node carries NO `agent_ref` — it runs on the default LLM
    // completion and never touches `OpenHumanAgentRunner`'s routing at all, so
    // this gate must never reject it. This is the exact invariant #5114 must
    // preserve: only an UNKNOWN `agent_ref` is rejected, never a plain node.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan", "config": { "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));
    let errors =
        validate_agent_refs(&crate::core::runtime::DispatchCtx::ambient(), &config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn agent_ref_blank_string_is_treated_as_absent() {
    // A whitespace-only `agent_ref` must be treated the same as no ref at all
    // rather than being resolved (and potentially rejected as "unknown").
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan",
              "config": { "agent_ref": "   ", "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));
    let errors =
        validate_agent_refs(&crate::core::runtime::DispatchCtx::ambient(), &config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn agent_ref_resolving_to_a_harness_definition_is_accepted() {
    // "orchestrator" is one of the bundled built-in agent definitions
    // (see `agent_registry::defaults::default_agents_include_core_personas`),
    // so it must resolve via `AgentRoute::Harness` and never touch the
    // custom agent registry at all.
    //
    // This also exercises the CodeRabbit/Codex #5114 review fix: run via the
    // scoped `cargo test --lib flows::ops` filter, no other domain's test gets
    // to call `AgentDefinitionRegistry::init_global_builtins()` first, so this
    // only passes because `validate_agent_refs` now defensively initialises
    // the harness registry itself before resolving a ref.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan",
              "config": { "agent_ref": "orchestrator", "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));
    let errors =
        validate_agent_refs(&crate::core::runtime::DispatchCtx::ambient(), &config, &g).await;
    assert!(
        errors.is_empty(),
        "a real harness agent_ref must never be rejected: {errors:?}"
    );
}

#[tokio::test]
async fn agent_ref_unknown_is_rejected() {
    // The whole point of the gate (and the branch Codex flagged as uncovered on
    // #5114): an `agent` node whose `agent_ref` is NOT a real registered agent —
    // neither a bundled harness definition nor a custom registry entry — must be
    // REJECTED at author time, with the offending id named, rather than silently
    // hitting the `RegistryFallback` persona path at run time. Exercises the
    // error-construction branch of `validate_agent_refs`.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan",
              "config": { "agent_ref": "no_such_agent_xyz", "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));
    let errors =
        validate_agent_refs(&crate::core::runtime::DispatchCtx::ambient(), &config, &g).await;
    assert!(!errors.is_empty(), "an unknown agent_ref must be rejected");
    assert!(
        errors.iter().any(|e| e.contains("no_such_agent_xyz")),
        "the rejection error must name the offending agent_ref: {errors:?}"
    );
}

// ── dynamic agent_ref: refused at authoring, still reachable at run time ──

/// A `=`-expression `agent_ref` is no longer authorable. TinyFlows requires a
/// literal agent-registry reference so run data — which may include model
/// output — cannot choose an agent with different privileges, the same
/// reasoning this host already applies to `tool_call` slugs.
///
/// Pinned here rather than left to the vendor's own suite because the
/// `workflow_builder` agent can propose this shape, and the message a builder
/// sees on rejection is this host's contract with it.
#[test]
fn dynamic_agent_ref_is_rejected_during_structural_validation() {
    let err = validate_and_migrate_graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Dynamic",
              "config": { "agent_ref": "=nodes.t.item.agent_choice", "prompt": "go" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }))
    .expect_err("dynamic agent_ref must fail structural validation");
    assert!(
        err.contains("agent_ref") && err.contains("must be a literal"),
        "the message must say what is wrong, not just that something is: {err}"
    );
}

#[tokio::test]
async fn inference_gate_reports_signed_out_for_dynamic_agent_ref_only_graph() {
    // Finding C, and it survives the rule above: an `agent` node whose
    // `agent_ref` is `=`-derived means "this graph runs inference" whatever
    // its concrete route resolves to, so it must stay in scope for Layer 1
    // (signed-out/session) even though its per-model role cannot be resolved
    // statically. The bug this pins is a graph made up only of such nodes
    // returning `None` — no readiness signal at all — so a signed-out session
    // went completely unreported.
    //
    // This is NOT a dead path just because authoring now refuses the shape.
    // `store::load` runs `tinyflows::migrate::migrate` and deserializes, but
    // never `validate`, and `run_flow_body` hands the loaded `flow.graph`
    // straight to `validate_inference_readiness` — so a flow persisted before
    // the vendor rule still reaches this gate with a dynamic ref, which is
    // also what makes `agent_node_role`'s `=`-filter (and its fallback to the
    // default role) load-bearing rather than vestigial.
    //
    // Built as a struct literal for that reason: `graph()` would reject it,
    // and going through `graph()` would only prove the rule above twice.
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = WorkflowGraph {
        nodes: vec![
            tinyflows::model::Node {
                id: "t".to_string(),
                kind: NodeKind::Trigger,
                type_version: 1,
                name: "Manual".to_string(),
                config: json!({ "trigger_kind": "manual" }),
                ports: Vec::new(),
                position: None,
            },
            tinyflows::model::Node {
                id: "a".to_string(),
                kind: NodeKind::Agent,
                type_version: 1,
                name: "Dynamic".to_string(),
                config: json!({ "agent_ref": "=nodes.t.item.agent_choice", "prompt": "go" }),
                ports: Vec::new(),
                position: None,
            },
        ],
        ..Default::default()
    };

    let errors =
        validate_inference_readiness(&crate::core::runtime::DispatchCtx::ambient(), &config, &g)
            .await;
    assert!(
        !errors.is_empty(),
        "a signed-out session must still be reported even though the only agent node's \
         agent_ref is dynamic: {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.to_ascii_lowercase().contains("signed out")),
        "{errors:?}"
    );
    // `SignedOutTestGuard` restores the prior flag on drop at the end of this
    // scope — no other test observes this override.
}

#[tokio::test]
async fn proposal_includes_inference_status_for_agent_graph() {
    // `build_builder_proposal`'s payload carries the same inference-readiness
    // evaluation, ADVISORY only (B45 design correction), so the UI can render
    // provider-connectivity state alongside the built workflow. This pins the
    // happy-path shape: a `"ready"` graph carries no `inference_message`. A
    // local (`ollama:...`) provider construction is the pass path, matching
    // `inference_gate_passes_when_model_constructs` — no network, no
    // process-global test seam.
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.memory_provider = Some("ollama:llama3".to_string());

    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan", "config": { "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));

    let payload = build_builder_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "propose_workflow",
        "agent-flow",
        &g,
        false,
        false,
        None,
        None,
        None,
    )
    .await
    .expect("proposal must succeed for a well-formed agent graph");

    assert_eq!(payload["inference_status"], json!("ready"));
    assert!(
        payload.get("inference_message").is_none(),
        "a ready status must omit inference_message: {payload:?}"
    );
}

#[tokio::test]
async fn proposal_omits_inference_status_for_tool_call_only_graph() {
    // A graph with no `agent` node has nothing for this check to evaluate —
    // the field must be absent entirely, never a meaningless "ready".
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));

    let payload = build_builder_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        "propose_workflow",
        "tool-flow",
        &g,
        false,
        false,
        None,
        None,
        None,
    )
    .await
    .expect("proposal must succeed for a tool_call-only graph");

    assert!(
        payload.get("inference_status").is_none(),
        "a graph with no agent node must omit inference_status: {payload:?}"
    );
}

/// B45 run-time preflight (design correction, judge finding on live run
/// 104aab90): since authoring no longer hard-blocks on inference readiness, a
/// flow whose `agent` node cannot currently reach a working LLM provider can
/// be created and then RUN. `run_flow_body` must catch that BEFORE invoking
/// the tinyflows engine, finalizing the run row as `failed` with a clear,
/// actionable message rather than letting the engine attempt (and fail) real
/// work, or surface the opaque several-layers-deep "capability error: graph
/// error: capability error: model error: ... API key not configured for
/// provider" a mid-run failure produces.
///
/// Uses the signed-out seam (`SignedOutTestGuard`) rather than a mock
/// provider-not-configured backend response: both are classified `Err` by
/// `evaluate_inference_readiness` and reach the same preflight code path in
/// `run_flow_body`, and signed-out needs no network/mock server at all
/// (matching the existing gate tests' no-network convention). The
/// provider_not_configured class is covered end-to-end by
/// `probe_readiness_surfaces_api_key_not_configured` (construction) and the
/// negative-cache test below (through `cached_probe_inference_readiness`).
#[tokio::test]
async fn flows_run_fails_cleanly_without_invoking_engine_when_inference_not_ready() {
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let g = json!({
        "name": "needs-a-provider",
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan", "config": { "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    });
    let created = flows_create(&config, "needs-a-provider".to_string(), g, false)
        .await
        .expect("creating (authoring) an agent-node flow must succeed even when signed out");

    let err = flows_run(
        &crate::core::runtime::DispatchCtx::ambient(),
        &config,
        &created.value.id,
        json!({}),
        serde_json::Map::new(),
        FlowRunTrigger::Rpc,
    )
    .await
    .expect_err("a run whose agent node cannot reach a provider must fail cleanly");
    assert!(
        err.to_ascii_lowercase().contains("ai provider"),
        "error must explain the AI-provider problem: {err}"
    );
    assert!(
        err.to_ascii_lowercase().contains("signed out"),
        "error must surface the specific reason (signed out): {err}"
    );

    // The run row settled `failed` with that same message, and the engine
    // never ran (no persisted steps) — this is the "no pointless work" half
    // of the contract, not just "the RPC call returned an error".
    let runs = flows_list_runs(&config, &created.value.id, 1)
        .await
        .unwrap()
        .value;
    let run = runs.first().expect("a run row must exist");
    assert_eq!(run.status, "failed");
    assert!(
        run.steps.is_empty(),
        "the engine must never have executed a step: {:?}",
        run.steps
    );
    let run_error = run
        .error
        .as_deref()
        .expect("a failed run must carry an error message");
    assert!(
        run_error.to_ascii_lowercase().contains("ai provider"),
        "the persisted run error must explain the AI-provider problem: {run_error}"
    );

    // `SignedOutTestGuard` restores the prior flag on drop at the end of this
    // scope — no other test observes this override.
}

#[tokio::test]
async fn local_agent_flow_is_ready_without_openhuman_session() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.chat_provider = Some("omlx:test-model".into());
    config.memory_provider = Some("omlx:test-model".into());
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Local", "config": { "prompt": "test" } }
        ],
        "edges": [{ "from_node": "t", "to_node": "a" }]
    }));
    let errors =
        validate_inference_readiness(&crate::core::runtime::DispatchCtx::ambient(), &config, &g)
            .await;
    assert!(
        errors.is_empty(),
        "local flow must not need a session: {errors:?}"
    );
}

#[tokio::test]
async fn claude_agent_flow_readiness_does_not_require_an_openhuman_session() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);
    // Isolate the caller's session classification from CLI installation/auth
    // and provider availability. Layer 1 must still reject managed inference
    // even while the Layer 2 probe has an injected model.
    let _model =
        crate::inference::provider::factory::test_provider_override::install_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(vec!["OK"]),
        ));
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Claude", "config": { "prompt": "test" } }
        ],
        "edges": [{ "from_node": "t", "to_node": "a" }]
    }));
    for provider in [
        "claude-code:test-model",
        "claude_agent_sdk:test-model",
        "openhuman",
    ] {
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.memory_provider = Some(provider.into());
        let errors = validate_inference_readiness(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            &g,
        )
        .await;
        if provider == "openhuman" {
            assert!(
                errors.iter().any(|e| e.contains("signed out")),
                "{errors:?}"
            );
        } else {
            assert!(
                errors.is_empty(),
                "{provider} must not require a session: {errors:?}"
            );
        }
    }
}

#[test]
fn harness_readiness_uses_runtime_role_and_node_override() {
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.default_model = None;
    let mut g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Agent",
              "config": { "agent_ref": "orchestrator", "prompt": "go" } }
        ],
        "edges": [{"from_node": "t", "to_node": "a"}]
    }));
    let node = g.nodes.iter_mut().find(|n| n.id == "a").unwrap();
    assert_eq!(agent_node_role(&config, node), "coding");
    config.default_model = Some("hint:reasoning".into());
    assert_eq!(agent_node_role(&config, node), "reasoning");
    node.config["model"] = json!("chat-v1");
    assert_eq!(agent_node_role(&config, node), "chat");
    node.config["model"] = json!("hint:summarization");
    assert_eq!(agent_node_role(&config, node), "summarization");
    node.config["model"] = json!("custom-model-id");
    assert_eq!(agent_node_role(&config, node), "chat");
}

#[tokio::test]
async fn harness_readiness_checks_session_for_execution_provider() {
    let _inference = crate::inference::inference_test_guard_async().await;
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);
    let _model =
        crate::inference::provider::factory::test_provider_override::install_model(Arc::new(
            tinyagents_harness::testkit::ScriptedModel::replies(vec!["OK"]),
        ));
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Agent",
              "config": { "agent_ref": "orchestrator", "prompt": "go" } }
        ],
        "edges": [{"from_node": "t", "to_node": "a"}]
    }));
    for (runtime, summary, should_reject) in [
        ("ollama:test", "openhuman", false),
        ("openhuman", "ollama:test", true),
    ] {
        let tmp = TempDir::new().unwrap();
        let mut config = test_config(&tmp);
        config.default_model = Some("hint:reasoning".into());
        config.reasoning_provider = Some(runtime.into());
        config.memory_provider = Some(summary.into());
        let errors = validate_inference_readiness(
            &crate::core::runtime::DispatchCtx::ambient(),
            &config,
            &g,
        )
        .await;
        assert_eq!(
            errors.iter().any(|e| e.contains("signed out")),
            should_reject,
            "{errors:?}"
        );
        if !should_reject {
            assert!(errors.is_empty(), "{errors:?}");
        }
    }
}

/// The agent-ref gate resolves a custom agent through the context it is handed,
/// not the one the calling task happens to run under: two tenants' registries
/// never cross.
#[tokio::test]
async fn agent_ref_gate_reads_the_registry_of_the_explicit_context() {
    use crate::agent::registry::types::{AgentRegistryEntry, AgentRegistrySource};
    use crate::core::runtime::{CoreContext, DispatchCtx, DomainSet};

    let entry = AgentRegistryEntry {
        id: "tenant_a_only_agent".to_string(),
        name: "A".to_string(),
        description: "registered for tenant A".to_string(),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: None,
        system_prompt: None,
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
        tool_rules: None,
        subagents: Default::default(),
        tags: Vec::new(),
        metadata: Value::Null,
    };
    let tmp_a = TempDir::new().unwrap();
    let tmp_b = TempDir::new().unwrap();
    let mut config_a = test_config(&tmp_a);
    config_a.agent_registry.entries.push(entry);
    let config_b = test_config(&tmp_b);
    let core_a = CoreContext::for_test_with_config(DomainSet::full(), config_a.clone());
    let core_b = CoreContext::for_test_with_config(DomainSet::full(), config_b.clone());
    let ctx_a = DispatchCtx::new(core_a);
    let ctx_b = DispatchCtx::new(std::sync::Arc::clone(&core_b));

    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan",
              "config": { "agent_ref": "tenant_a_only_agent", "prompt": "go" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));

    // Both calls run scoped to tenant B, so an ambient read would answer B's
    // registry for A's call.
    let (for_a, for_b) = CoreContext::scope(core_b, async {
        (
            validate_agent_refs(&ctx_a, &config_a, &g).await,
            validate_agent_refs(&ctx_b, &config_b, &g).await,
        )
    })
    .await;
    assert!(
        for_a.is_empty(),
        "tenant A's own agent must resolve: {for_a:?}"
    );
    assert!(
        for_b.iter().any(|e| e.contains("tenant_a_only_agent")),
        "tenant B must not see tenant A's agent: {for_b:?}"
    );
}
