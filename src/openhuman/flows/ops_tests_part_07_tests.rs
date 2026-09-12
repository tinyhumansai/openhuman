use super::*;

#[test]
fn binding_to_agent_with_matching_schema_is_accepted() {
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "summarize", "kind": "agent", "name": "Summarize",
              "config": { "prompt": "summarize",
                "output_parser": { "schema": { "type": "object",
                    "required": ["channel"],
                    "properties": { "channel": { "type": "string" } } } } } },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE",
                "args": { "channel": "=nodes.summarize.item.json.channel" } } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "summarize" },
            { "from_node": "summarize", "to_node": "post" }
        ]
    }));
    assert!(
        validate_binding_resolvability(&g).is_empty(),
        "{:?}",
        validate_binding_resolvability(&g)
    );
}

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
    let errors = validate_agent_refs(&config, &g).await;
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
    let errors = validate_agent_refs(&config, &g).await;
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
    let errors = validate_agent_refs(&config, &g).await;
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
    let errors = validate_agent_refs(&config, &g).await;
    assert!(!errors.is_empty(), "an unknown agent_ref must be rejected");
    assert!(
        errors.iter().any(|e| e.contains("no_such_agent_xyz")),
        "the rejection error must name the offending agent_ref: {errors:?}"
    );
}

#[tokio::test]
async fn inference_gate_skips_when_no_agent_nodes() {
    // A tool_call-only graph never has an inference dependency to check — the
    // gate must short-circuit to empty without touching sign-in state or the
    // network at all.
    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "post", "kind": "tool_call", "name": "Post",
              "config": { "slug": "SLACK_SEND_MESSAGE", "args": { "channel": "#general" } } }
        ],
        "edges": [ { "from_node": "t", "to_node": "post" } ]
    }));
    let errors = validate_inference_readiness(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

// B45 design correction (judge finding on live run 104aab90): the gate used
// to hard-reject `run_builder_gates` when signed out, which blocked
// `propose_workflow`/`edit_workflow` from ever showing the user the graph at
// all. Authoring must now succeed unconditionally; readiness only ever
// surfaces as an advisory `inference_status` on the proposal. These two tests
// replace the old `inference_gate_rejects_when_signed_out`, which asserted
// the opposite (a hard reject) of the now-correct contract.

#[tokio::test]
async fn run_builder_gates_does_not_reject_when_signed_out() {
    // Authoring is never blocked by inference readiness (design correction,
    // B45): a signed-out session must NOT appear among `run_builder_gates`'
    // errors for an otherwise-valid agent-node graph.
    let _signed_out = crate::openhuman::cron::scheduler_gate::SignedOutTestGuard::set(true);

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan", "config": { "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));
    let errors = run_builder_gates(&config, &g).await;
    assert!(
        errors.is_empty(),
        "authoring must not be blocked by a signed-out session: {errors:?}"
    );
    // `SignedOutTestGuard` restores the prior flag on drop at the end of this
    // scope — no other test observes this override.
}

#[tokio::test]
async fn proposal_surfaces_signed_out_inference_status() {
    // The proposal still WARNS about the signed-out state (advisory, never a
    // rejection) so the UI can render a "sign in" nudge alongside the built
    // workflow.
    let _signed_out = crate::openhuman::cron::scheduler_gate::SignedOutTestGuard::set(true);

    let tmp = TempDir::new().unwrap();
    let config = test_config(&tmp);
    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan", "config": { "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));

    let payload = build_builder_proposal(
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
    .expect("a signed-out session must NOT block proposing the graph");

    assert_eq!(payload["inference_status"], json!("signed_out"));
    let message = payload["inference_message"]
        .as_str()
        .expect("a non-ready status must carry inference_message");
    assert!(
        message.to_ascii_lowercase().contains("signed out"),
        "message must tell the user they are signed out: {message}"
    );
    // `SignedOutTestGuard` restores the prior flag on drop at the end of this
    // scope — no other test observes this override.
}

#[tokio::test]
async fn inference_gate_passes_when_model_constructs() {
    // Layer 2 (async probe), happy path: the resolved role ("summarization" —
    // the default for a plain agent node) points at a local runtime
    // (`ollama:...`), which `probe_inference_readiness` never probes over the
    // network at all — `resolves_to_managed_backend` is false for a local
    // provider, so construction succeeding is the whole check (no HTTP, no
    // process-global test seam, so this can never race another test that
    // installs `test_provider_override`).
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
    let errors = validate_inference_readiness(&config, &g).await;
    assert!(errors.is_empty(), "{errors:?}");
}

#[tokio::test]
async fn inference_gate_surfaces_construction_error() {
    // Layer 2 (async probe), construction-failure path: the resolved role
    // ("summarization" — the default for a plain agent node with no pinned
    // `config.model`) points at a cloud slug that isn't in `cloud_providers`
    // at all, so `create_chat_model_with_model_id_inner` fails on a pure
    // config lookup — no test override installed, no network involved — and
    // the gate must surface that failure, naming the offending node.
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    seed_app_session_for_gate_test(&tmp);
    config.memory_provider = Some("no_such_slug:some-model".to_string());

    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Plan", "config": { "prompt": "outline it" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));
    let errors = validate_inference_readiness(&config, &g).await;
    assert!(!errors.is_empty(), "a construction failure must reject");
    assert!(
        errors.iter().any(|e| e.contains("Node 'a'")),
        "error must name the offending node 'a': {errors:?}"
    );
    assert!(
        errors
            .iter()
            .any(|e| e.contains("no_such_slug") || e.contains("no cloud provider configured")),
        "error must surface the construction failure detail: {errors:?}"
    );
}

// ── multi-role agent-node graphs (findings A+B, P1) ─────────────────────────
//
// Previously `evaluate_inference_readiness` collected every applicable
// `agent` node but derived the Layer-2 probe role from ONLY the graph's
// first node — a second (or later) node pinned to a different `config.model`
// (and therefore routed to a different, possibly broken, provider) was never
// probed at all. These tests wire each role to its own pure-config-lookup
// failure (no network, no test-provider-override seam) so a bug that skips a
// role would show up as a falsely-empty `errors` list.

#[test]
fn agent_node_role_prefers_custom_registry_entry_model_pin_over_default() {
    // Finding A/B: a node with no per-node `config.model` but a STATIC
    // (non-`=`) `agent_ref` naming a custom registry entry that itself pins a
    // model (e.g. `hint:reasoning`) must resolve to THAT role — the same
    // precedence `OpenHumanAgentRunner::run_via_harness` applies via
    // `resolve_node_model(&request, entry_model)`, reusing the same sync,
    // config-only accessor (`find_custom_in_config`) it calls.
    use crate::openhuman::agent::registry::types::{AgentRegistryEntry, AgentRegistrySource};

    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    config.agent_registry.entries.push(AgentRegistryEntry {
        id: "researcher_custom".to_string(),
        name: "Researcher".to_string(),
        description: "does research".to_string(),
        source: AgentRegistrySource::Custom,
        enabled: true,
        model: Some("hint:reasoning".to_string()),
        system_prompt: None,
        tool_allowlist: Vec::new(),
        tool_denylist: Vec::new(),
        subagents: Default::default(),
        tags: Vec::new(),
        metadata: Value::Null,
    });

    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Research",
              "config": { "agent_ref": "researcher_custom", "prompt": "go" } }
        ],
        "edges": [ { "from_node": "t", "to_node": "a" } ]
    }));
    let node = g.nodes.iter().find(|n| n.id == "a").expect("node 'a'");
    assert_eq!(
        agent_node_role(&config, node),
        "reasoning",
        "the custom registry entry's `hint:reasoning` pin must win over the default role"
    );
}

#[tokio::test]
async fn inference_gate_probes_every_distinct_agent_node_role() {
    // A graph with TWO `agent` nodes, each pinned (via `config.model`) to a
    // DIFFERENT role — `chat` and `reasoning` — each wired to its own broken
    // provider slug for that specific role's config knob
    // (`chat_provider`/`reasoning_provider`). If the gate only probed the
    // first node's role (the pre-fix bug), the second node's broken
    // `reasoning` provider would never be checked and this graph would
    // incorrectly pass. Both failures must be named.
    let tmp = TempDir::new().unwrap();
    let mut config = test_config(&tmp);
    seed_app_session_for_gate_test(&tmp);
    config.chat_provider = Some("no_such_chat_slug:some-model".to_string());
    config.reasoning_provider = Some("no_such_reasoning_slug:some-model".to_string());

    let g = graph(json!({
        "nodes": [
            { "id": "t", "kind": "trigger", "name": "Manual" },
            { "id": "a", "kind": "agent", "name": "Chat step",
              "config": { "prompt": "chat", "model": "chat-v1" } },
            { "id": "b", "kind": "agent", "name": "Reasoning step",
              "config": { "prompt": "reason", "model": "reasoning-v1" } }
        ],
        "edges": [
            { "from_node": "t", "to_node": "a" },
            { "from_node": "a", "to_node": "b" }
        ]
    }));

    let errors = validate_inference_readiness(&config, &g).await;
    assert!(
        !errors.is_empty(),
        "both roles are broken, the gate must reject"
    );
    let combined = errors.join("\n");
    assert!(
        combined.contains("'a'") && combined.contains("no_such_chat_slug"),
        "the `chat` role's failure (node 'a') must be named: {combined}"
    );
    assert!(
        combined.contains("'b'") && combined.contains("no_such_reasoning_slug"),
        "the `reasoning` role's failure (node 'b') must be named — this is the exact \
         regression the pre-fix \"probe only the first node's role\" bug would have hidden: \
         {combined}"
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
    let _signed_out = crate::openhuman::cron::scheduler_gate::SignedOutTestGuard::set(true);

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

    let errors = validate_inference_readiness(&config, &g).await;
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
    let _signed_out = crate::openhuman::cron::scheduler_gate::SignedOutTestGuard::set(true);

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
