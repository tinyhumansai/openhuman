use super::*;
use crate::config::Config;
use serde_json::{json, Value};
use tempfile::TempDir;

fn test_config(tmp: &TempDir) -> Config {
    let config = Config {
        workspace_dir: tmp.path().join("workspace"),
        action_dir: tmp.path().join("workspace"),
        config_path: tmp.path().join("config.toml"),
        ..Config::default()
    };
    std::fs::create_dir_all(&config.workspace_dir).unwrap();
    config
}

fn graph(value: Value) -> Value {
    let graph = tinyflows::migrate::deserialize_graph(value).expect("test graph deserializes");
    tinyflows::validate::validate(&graph).expect("test graph is structurally valid");
    tinyflows::compat::ensure_compatible(&graph).expect("test graph is compatible");
    serde_json::to_value(graph).expect("validated graph serializes")
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
///
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
