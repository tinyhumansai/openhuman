//! Inference-readiness gate: `validate_inference_readiness`, the builder-gate
//! advisory on signed-out sessions, and per-role probing of agent nodes.

use super::*;

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
    let errors =
        validate_inference_readiness(&crate::core::runtime::DispatchCtx::ambient(), &config, &g)
            .await;
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
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);

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
        run_builder_gates(&crate::core::runtime::DispatchCtx::ambient(), &config, &g).await;
    assert!(
        errors.is_empty(),
        "authoring must not be blocked by a signed-out session: {errors:?}"
    );
    // Under cfg(test), `SignedOutTestGuard` stores the override by Tokio
    // runtime ID and restores it on drop, so parallel async tests cannot
    // observe this runtime's signed-out state.
}

#[tokio::test]
async fn proposal_surfaces_signed_out_inference_status() {
    // The proposal still WARNS about the signed-out state (advisory, never a
    // rejection) so the UI can render a "sign in" nudge alongside the built
    // workflow.
    let _signed_out = crate::platform::llm_gate::SignedOutTestGuard::set(true);

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
    .expect("a signed-out session must NOT block proposing the graph");

    assert_eq!(payload["inference_status"], json!("signed_out"));
    let message = payload["inference_message"]
        .as_str()
        .expect("a non-ready status must carry inference_message");
    assert!(
        message.to_ascii_lowercase().contains("signed out"),
        "message must tell the user they are signed out: {message}"
    );
    // Under cfg(test), `SignedOutTestGuard` stores the override by Tokio
    // runtime ID and restores it on drop, so parallel async tests cannot
    // observe this runtime's signed-out state.
}

#[tokio::test]
async fn inference_gate_passes_when_model_constructs() {
    // Layer 2 (async probe), happy path: the resolved role ("summarization" —
    // the default for a plain agent node) points at a local runtime
    // (`ollama:...`), which `probe_inference_readiness` never probes over the
    // network at all — `resolves_to_managed_backend` is false for a local
    // provider, so construction succeeding is the whole check (no HTTP, no
    // network at all. Construction still observes the process-global test
    // provider seam, so serialize this probe with tests that install an
    // override.
    let _inference = crate::inference::inference_test_guard_async().await;
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
    let errors =
        validate_inference_readiness(&crate::core::runtime::DispatchCtx::ambient(), &config, &g)
            .await;
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
    // Construction must FAIL here, so no test may have the process-global
    // `test_provider_override` installed meanwhile; its installers hold this.
    let _inference = crate::inference::inference_test_guard_async().await;
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
    let errors =
        validate_inference_readiness(&crate::core::runtime::DispatchCtx::ambient(), &config, &g)
            .await;
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
    use crate::agent::registry::types::{AgentRegistryEntry, AgentRegistrySource};

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
        tool_rules: None,
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
    // Construction must FAIL here, so no test may have the process-global
    // `test_provider_override` installed meanwhile; its installers hold this.
    let _inference = crate::inference::inference_test_guard_async().await;
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

    let errors =
        validate_inference_readiness(&crate::core::runtime::DispatchCtx::ambient(), &config, &g)
            .await;
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
