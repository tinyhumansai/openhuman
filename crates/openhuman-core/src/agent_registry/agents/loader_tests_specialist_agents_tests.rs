use super::*;
use tinytools::Tool;

// Both flows agents are `#[cfg(feature = "flows")]` entries in `BUILTINS`
// (#4797), so these tests only apply when the gate is on.
#[cfg(feature = "flows")]
#[test]
fn workflow_builder_is_registered_worker_with_bounded_authoring_scope() {
    // Phase 5a/5b: the workflow-builder must be a Worker-tier leaf whose
    // tool scope is EXACTLY the bounded authoring/read + Composio
    // discovery/connect belt. Creation is limited to `create_workflow`
    // and `duplicate_flow`, which always produce disabled flows; the raw
    // flows_create/update/set_enabled tools remain unavailable, as do
    // shell, file writes, channel sends, and composio_execute. It can list
    // toolkits/connections,
    // raise the inline connect card, `run_flow` a flow the user already
    // SAVED to test it (a real run the prompt gates behind user
    // confirmation), and `save_workflow` a built graph onto a flow the host
    // ALREADY created (the prompt bar's instant-create path) — but it can
    // never enable a flow or perform an arbitrary raw integration action.
    // One narrow, deliberate carve-out (B12): `get_tool_output_sample`
    // DOES make a real Composio call, but only ever a Read-scope one
    // (hard-refused otherwise, regardless of the user's scope preference)
    // against an already-connected toolkit — see `builder_tools.rs`'s
    // module doc. This pins the invariant in the agent definition itself,
    // not just the tool implementations. It reaches the user's memory
    // through the single `memory` tool.
    let def = find("workflow_builder");
    assert_eq!(def.agent_tier, AgentTier::Worker);
    assert!(def.delegate_name.is_none());
    assert_eq!(def.sandbox_mode, SandboxMode::None);
    // Graph authoring is multi-step structured reasoning — reasoning tier.
    assert!(
        matches!(def.model, ModelSpec::Hint(ref h) if h == "reasoning"),
        "workflow_builder should use the reasoning tier"
    );
    // Worker leaf: no onward delegation.
    assert!(
        def.subagents.is_empty(),
        "workflow_builder is a leaf and must not list subagents"
    );
    match &def.tools {
        ToolScope::Named(names) => {
            // Reconciled against `agent.toml`'s current `[tools].named`
            // after the workflow-tools expansion PR widened the belt to
            // agent-native editing/creation/run-control (`edit_workflow`,
            // `validate_workflow`, `create_workflow`, `duplicate_flow`,
            // `list_node_kinds`, `get_node_kind_contract`,
            // `get_flow_history`, `list_flow_runs`, `resume_flow_run`,
            // `cancel_flow_run`, `list_connectable_toolkits`) — these are
            // the agent's own scoped tool surface, not the raw `flows_*`
            // controller RPCs banned below, so the "no flow
            // creation/enable via the raw controller" invariant still
            // holds via the forbidden list.
            let expected = [
                "read_workflow_resource",
                "propose_workflow",
                "revise_workflow",
                "edit_workflow",
                "validate_workflow",
                "save_workflow",
                "list_flows",
                "get_flow",
                "get_flow_history",
                "get_flow_run",
                "list_flow_connections",
                "search_tool_catalog",
                "get_tool_contract",
                "get_tool_output_sample",
                "list_agent_definitions",
                "list_connectable_toolkits",
                "list_node_kinds",
                "get_node_kind_contract",
                "dry_run_workflow",
                "list_flow_runs",
                "resume_flow_run",
                "cancel_flow_run",
                "create_workflow",
                "duplicate_flow",
                "run_flow",
                "composio_list_toolkits",
                "composio_list_connections",
                "composio_connect",
                "memory",
            ];
            for required in expected {
                assert!(
                    names.iter().any(|n| n == required),
                    "workflow_builder tool list missing `{required}`"
                );
            }
            assert_eq!(
                names.len(),
                expected.len(),
                "workflow_builder scope must be EXACTLY the bounded authoring belt (got {names:?})"
            );
            // Hard exclusions: no unrestricted flow mutation, raw
            // integration actions, or host access. Creation is exposed
            // only through the bounded tools above; raw `flows_update`
            // could rename or re-gate arbitrary flows, so it stays out.
            for forbidden in [
                "flows_create",
                "flows_update",
                "flows_set_enabled",
                "shell",
                "file_write",
                "edit",
                "apply_patch",
                "composio_execute",
                "spawn_subagent",
            ] {
                assert!(
                    !names.iter().any(|n| n == forbidden),
                    "workflow_builder must NOT have unrestricted tool `{forbidden}`"
                );
            }
        }
        ToolScope::Wildcard => panic!("workflow_builder must have a Named tool scope"),
    }

    // Reachable by delegation from the orchestrator (Phase 5 routing).
    let orchestrator = find("orchestrator");
    assert!(
        orchestrator
            .subagents
            .iter()
            .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == "workflow_builder")),
        "orchestrator must allow `workflow_builder` for spawn_subagent"
    );
}

#[cfg(feature = "flows")]
#[test]
fn flow_discovery_is_registered_readonly_reasoning_scout() {
    // The Flow Scout must be a read-only reasoning leaf: it reads the
    // user's data and ends by emitting `suggest_workflows`. It must NOT
    // carry any tool that persists/enables/runs a flow, sends a message,
    // writes memory, or mutates the workspace — it can run on
    // prompt-injectable content, so a write tool would be an injection
    // foothold.
    let def = find("flow_discovery");
    assert_eq!(def.agent_tier, AgentTier::Reasoning);
    assert!(def.delegate_name.is_none());
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert!(
        def.subagents.is_empty(),
        "flow_discovery is a leaf and must not list subagents"
    );
    match &def.tools {
        ToolScope::Named(names) => {
            // The one write it is allowed: its terminal emit sink.
            assert!(
                names.iter().any(|n| n == "suggest_workflows"),
                "flow_discovery must have its `suggest_workflows` emit sink"
            );
            // A representative slice of the read-only gathering surface.
            for required in [
                "list_flows",
                "list_flow_connections",
                "search_tool_catalog",
                "web_search_tool",
            ] {
                assert!(
                    names.iter().any(|n| n == required),
                    "flow_discovery tool list missing read tool `{required}`"
                );
            }
            // Hard exclusions: nothing that persists, executes, sends, or
            // writes user data.
            for forbidden in [
                "flows_create",
                "flows_update",
                "flows_set_enabled",
                "flows_run",
                "propose_workflow",
                "shell",
                "file_write",
                "edit",
                "memory",
                "thread_message_append",
                "spawn_subagent",
            ] {
                assert!(
                    !names.iter().any(|n| n == forbidden),
                    "flow_discovery must NOT have `{forbidden}` — read + suggest only"
                );
            }
        }
        ToolScope::Wildcard => panic!("flow_discovery must have a Named tool scope"),
    }

    // Reachable from the orchestrator through the shared spawn tool.
    let orchestrator = find("orchestrator");
    assert!(
        orchestrator
            .subagents
            .iter()
            .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == "flow_discovery")),
        "orchestrator must allow `flow_discovery` for spawn_subagent"
    );
}

#[test]
fn specialist_agents_are_registered_with_narrow_tools() {
    // Scheduling is the `scheduling` skill over the per-operation cron
    // tools, with the time tools the orchestrator holds directly.
    let scheduling = crate::tools::toolpacks::pack("scheduling").expect("scheduling skill");
    assert_eq!(
        scheduling.tools,
        crate::cron::tools::CRON_OPERATION_TOOLS,
        "the skill lists the per-operation cron tools"
    );
    match &find("orchestrator").tools {
        ToolScope::Named(names) => {
            for required in ["current_time", "resolve_time"] {
                assert!(
                    names.iter().any(|name| name == required),
                    "orchestrator must hold `{required}` to ground a schedule"
                );
            }
        }
        other => panic!("orchestrator must use Named tool scope, got {other:?}"),
    }

    // `presentation_agent` is only registered under the `documents` feature
    // (its deck tool `generate_presentation` is gated there and the agent is
    // filtered from the registry in lockstep — see `builtin_enabled`), so
    // skip its assertions in slim builds where it is intentionally absent.
    #[cfg(feature = "documents")]
    {
        let presentation = find("presentation_agent");
        match &presentation.tools {
            ToolScope::Named(names) => {
                assert!(names.iter().any(|name| name == "generate_presentation"));
                assert!(names.iter().any(|name| name == "web_search_tool"));
            }
            other => panic!("presentation_agent must use Named tool scope, got {other:?}"),
        }
        // `omit_memory_context = false` opens the deck builder's session
        // with the compiled memory context.
        assert!(!presentation.extensions.omit_memory_context);
    }
}

#[test]
fn orchestrator_routes_every_registered_specialist_through_spawn_subagent() {
    use crate::core::runtime::{ContextOverlay, CoreContext, DomainSet};
    use std::sync::Arc;

    let registry =
        Arc::new(crate::agent_registry::host_definition::AgentDefinitionRegistry::builtins_only());
    let orchestrator = registry.get("orchestrator").expect("orchestrator exists");
    let config = crate::config::Config::default();
    let core = CoreContext::for_test_with_config(DomainSet::full(), config.clone()).derive_with(
        ContextOverlay::new(
            config,
            DomainSet::full(),
            crate::tools::toolpacks::ToolGroups::none(),
        )
        .definitions(Arc::clone(&registry)),
    );
    CoreContext::sync_scope(core, || {
        let spawn = crate::agent::orchestration::tools::SpawnSubagentTool::new();
        let schema = spawn.parameters_schema();
        let allowed = schema
            .pointer("/properties/agent_id/enum")
            .and_then(serde_json::Value::as_array)
            .expect("spawn_subagent has a finite agent_id enum");
        for entry in &orchestrator.subagents {
            let SubagentEntry::AgentId(agent_id) = entry else {
                continue;
            };
            if agent_id == "presentation_agent" && !cfg!(feature = "documents") {
                assert!(
                    registry.get(agent_id).is_none(),
                    "presentation_agent is intentionally absent without documents"
                );
                assert!(
                    !allowed
                        .iter()
                        .any(|candidate| candidate.as_str() == Some(agent_id)),
                    "spawn_subagent must not advertise presentation_agent without documents"
                );
                continue;
            }
            if agent_id == "skill_setup" && !cfg!(feature = "skills") {
                assert!(
                    registry.get(agent_id).is_none(),
                    "skill_setup is intentionally absent without skills"
                );
                assert!(
                    !allowed
                        .iter()
                        .any(|candidate| candidate.as_str() == Some(agent_id)),
                    "spawn_subagent must not advertise skill_setup without skills"
                );
                continue;
            }
            if matches!(agent_id.as_str(), "workflow_builder" | "flow_discovery")
                && !cfg!(feature = "flows")
            {
                assert!(
                    registry.get(agent_id).is_none(),
                    "workflow specialists are intentionally absent without flows"
                );
                assert!(
                    !allowed
                        .iter()
                        .any(|candidate| candidate.as_str() == Some(agent_id)),
                    "spawn_subagent must not advertise workflow specialists without flows"
                );
                continue;
            }
            assert!(
                registry.get(agent_id).is_some(),
                "orchestrator specialist `{agent_id}` must be registered"
            );
            assert!(
                allowed
                    .iter()
                    .any(|candidate| candidate.as_str() == Some(agent_id)),
                "orchestrator allows `{agent_id}` but spawn_subagent does not"
            );
        }
    });
}

#[test]
fn morning_briefing_is_read_only() {
    let def = find("morning_briefing");
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    // A named belt, not a wildcard: a cron-driven read-only job has no use
    // for every registered tool.
    match &def.tools {
        ToolScope::Named(tools) => {
            for required in ["composio_execute", "tool_search", "current_time"] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "morning_briefing needs `{required}`"
                );
            }
            assert!(!tools.iter().any(|t| t == "shell" || t == "file_write"));
            // `memory` reports Write permission without arguments, which the
            // read-only sandbox cannot admit.
            assert!(!tools.iter().any(|t| t == "memory"));
        }
        ToolScope::Wildcard => panic!("morning_briefing must have a named belt"),
    }
    // The brief grounds itself in the compiled memory context.
    assert!(!def.extensions.omit_memory_context);
    assert!(def.extensions.omit_identity);
    assert!(def.extensions.omit_safety_preamble);
    assert_eq!(def.max_iterations, 8);
}

#[test]
fn no_builtin_agent_declares_the_deprecated_vision_hint() {
    for def in load_builtins().expect("built-ins load") {
        assert!(
            !matches!(&def.model, ModelSpec::Hint(h) if h == "vision"),
            "`{}` still declares the deprecated `hint:vision` — pin an exact model instead",
            def.id
        );
    }
}

/// The three media agents are pinned to their dedicated OpenRouter
/// passthrough models (regression R4), not left on `Inherit` or a `Hint`.
#[test]
fn media_agents_are_pinned_to_their_exact_models() {
    for (agent_id, expected_model) in [
        ("vision_agent", "openrouter/qwen/qwen3.5-flash-02-23"),
        ("image_agent", "openrouter/qwen/qwen3.5-flash-02-23"),
        ("video_agent", "openrouter/qwen/qwen3.5-flash-02-23"),
    ] {
        let def = find(agent_id);
        match &def.model {
            ModelSpec::Exact(model) => assert_eq!(
                model, expected_model,
                "{agent_id} must be pinned to `{expected_model}`, got `{model}`"
            ),
            other => panic!("{agent_id} must use ModelSpec::Exact, got {other:?}"),
        }
    }
}

#[test]
fn orchestrator_does_not_get_curl() {
    // Per design: curl is a `Write` permission tool that writes to the
    // workspace. It stays off the orchestrator's belt; the orchestrator
    // reaches it as a `Deferred` tool (skill `coding` / `tool_search`).
    let def = find("orchestrator");
    if let ToolScope::Named(tools) = &def.tools {
        assert!(
            !tools.iter().any(|t| t == "curl"),
            "orchestrator must not carry curl on its belt — it is deferred"
        );
    }
}
