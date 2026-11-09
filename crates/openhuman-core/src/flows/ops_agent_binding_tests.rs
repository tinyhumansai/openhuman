use super::*;
#[tokio::test]
async fn dynamic_agent_ref_is_rejected_during_structural_validation() {
    let err = validate_and_migrate_graph(
        &crate::core::runtime::DispatchCtx::ambient(),
        &Config::default(),
        json!({
            "nodes": [
                { "id": "t", "kind": "trigger", "name": "Manual" },
                { "id": "a", "kind": "agent", "name": "Dynamic",
                  "config": { "agent_ref": "=nodes.t.item.agent_choice", "prompt": "go" } }
            ],
            "edges": [ { "from_node": "t", "to_node": "a" } ]
        }),
    )
    .await
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
    let g = json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual", "config": { "trigger_kind": "manual" } },
            { "id": "a", "kind": "agent", "name": "Dynamic", "config": { "agent_ref": "=nodes.t.item.agent_choice", "prompt": "go" } }
        ],
        "edges": []
    });

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

#[tokio::test]
async fn proposal_graph_carries_the_proposal_name() {
    // The builder resolves the name from its `name` argument; a graph authored
    // without one used to reach the card (and a save) with an empty name.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);

    let g = graph(json!({
        "name": "",
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
        "edit_workflow",
        "Daily Tech Digest",
        &g,
        false,
        true,
        None,
        None,
        None,
    )
    .await
    .expect("proposal must succeed for a tool_call-only graph");

    assert_eq!(payload["name"], json!("Daily Tech Digest"));
    assert_eq!(payload["graph"]["name"], json!("Daily Tech Digest"));
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

    // Provider readiness is a host preflight before the module's StartRun
    // admission boundary, so rejection must not create a module-owned run row
    // or stamp the saved workflow as executed.
    let runs = flows_list_runs(&config, &created.value.id, 1)
        .await
        .unwrap()
        .value;
    assert!(runs.is_empty(), "preflight rejection must not admit a run");
    let reloaded = flows_get(&config, &created.value.id).await.unwrap();
    assert!(reloaded.value.last_run_at.is_none());
    assert!(reloaded.value.last_status.is_none());

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

#[tokio::test]
async fn harness_readiness_uses_runtime_role_and_node_override() {
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
    let node = g["nodes"]
        .as_array_mut()
        .unwrap()
        .iter_mut()
        .find(|node| node["id"] == "a")
        .unwrap();
    assert_eq!(agent_node_role(&config, &node["config"]).await, "coding");
    config.default_model = Some("hint:reasoning".into());
    assert_eq!(agent_node_role(&config, &node["config"]).await, "reasoning");
    node["config"]["model"] = json!("chat-v1");
    assert_eq!(agent_node_role(&config, &node["config"]).await, "chat");
    node["config"]["model"] = json!("hint:summarization");
    assert_eq!(
        agent_node_role(&config, &node["config"]).await,
        "summarization"
    );
    node["config"]["model"] = json!("custom-model-id");
    assert_eq!(agent_node_role(&config, &node["config"]).await, "chat");
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
    use crate::agent_registry::types::{AgentRegistryEntry, AgentRegistrySource};
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
    let graph_json = serde_json::to_value(&g).unwrap();

    // Both calls run scoped to tenant B, so an ambient read would answer B's
    // registry for A's call.
    let (for_a, for_b) = CoreContext::scope(core_b, async {
        (
            crate::flows::host_gates::validate_agent_refs(&ctx_a, &config_a, &graph_json).await,
            crate::flows::host_gates::validate_agent_refs(&ctx_b, &config_b, &graph_json).await,
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

fn tool_call_graph() -> Value {
    graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "oh:noop" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }))
}

async fn revision_payload(config: &Config, flow_id: &str, draft_id: Option<String>) -> Value {
    build_builder_proposal(
        &crate::core::runtime::DispatchCtx::ambient(),
        config,
        "edit_workflow",
        "Base",
        &tool_call_graph(),
        false,
        true,
        None,
        draft_id,
        Some(flow_id.to_string()),
    )
    .await
    .expect("proposal must succeed for a tool_call-only graph")
}

#[tokio::test]
async fn revision_proposal_carries_the_version_its_draft_was_seeded_from() {
    // The draft was seeded from the flow as it is now, so the flow's current
    // version is the one the revision was derived from.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flows_create(&config, "Base".to_string(), tool_call_graph(), false)
        .await
        .unwrap()
        .value;
    let draft = crate::flows::draft_store::create_draft(
        &config,
        Some(flow.id.clone()),
        "Base".to_string(),
        tool_call_graph(),
        tinyflows_bus::catalog::DraftOrigin::Chat,
    )
    .unwrap();

    let payload = revision_payload(&config, &flow.id, Some(draft.id)).await;

    assert_eq!(payload["flow_id"], json!(flow.id));
    assert_eq!(payload["flow_version"], json!(flow.updated_at));
}

#[tokio::test]
async fn revision_proposal_omits_the_version_after_the_flow_changed_under_its_draft() {
    // An edit after the draft was seeded makes the flow's current version a
    // later one than the revision was derived from; stamping it would let the
    // stale graph pass `expected_version`.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let flow = flows_create(&config, "Base".to_string(), tool_call_graph(), false)
        .await
        .unwrap()
        .value;
    let draft = crate::flows::draft_store::create_draft(
        &config,
        Some(flow.id.clone()),
        "Base".to_string(),
        tool_call_graph(),
        tinyflows_bus::catalog::DraftOrigin::Chat,
    )
    .unwrap();
    flows_update(
        &config,
        &flow.id,
        Some("Edited".to_string()),
        None,
        None,
        None,
    )
    .await
    .unwrap();

    let payload = revision_payload(&config, &flow.id, Some(draft.id)).await;

    assert_eq!(payload["flow_id"], json!(flow.id));
    assert!(payload.get("flow_version").is_none(), "{payload:?}");

    // Without a draft to date the revision, no version is claimed either.
    let undated = revision_payload(&config, &flow.id, None).await;
    assert!(undated.get("flow_version").is_none(), "{undated:?}");
}
