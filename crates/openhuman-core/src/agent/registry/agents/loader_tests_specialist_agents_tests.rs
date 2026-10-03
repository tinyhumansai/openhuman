use super::*;

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
    assert_eq!(def.delegate_name.as_deref(), Some("build_workflow"));
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
        "orchestrator must allow `workflow_builder` so build_workflow can spawn it"
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
    assert_eq!(def.delegate_name.as_deref(), Some("discover_workflows"));
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

    // Reachable by delegation from the orchestrator so `discover_workflows`
    // can spawn it.
    let orchestrator = find("orchestrator");
    assert!(
        orchestrator
            .subagents
            .iter()
            .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == "flow_discovery")),
        "orchestrator must allow `flow_discovery` so discover_workflows can spawn it"
    );
}

#[test]
fn specialist_agents_are_registered_with_narrow_tools() {
    // Scheduling is the `scheduling` skill over the collapsed `cron` tool,
    // with the time tools the orchestrator holds directly.
    let scheduling = crate::tools::toolpacks::pack("scheduling").expect("scheduling skill");
    assert_eq!(
        scheduling.tools,
        &["cron"],
        "the skill uses collapsed `cron`"
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
        assert!(!presentation.omit_memory_context);
    }
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
    assert!(!def.omit_memory_context);
    assert!(def.omit_identity);
    assert!(def.omit_safety_preamble);
    assert_eq!(def.max_iterations, 8);
}

#[test]
fn chatty_sub_agents_have_bounded_output() {
    // critic results flow up verbatim. Without a cap the output is
    // unbounded and bloats the caller's context (#4099), so it must carry
    // the normal sub-agent cap so a long diff review can't leak unbounded
    // text.
    assert_eq!(
        find("critic").max_result_chars,
        Some(8000),
        "critic output must be bounded so reviews don't leak unbounded text up"
    );
}

/// R4 regression: `hint:vision` is deprecated (`vision-v1` silently falls
/// back to the chat default on managed routes, with no error), so no
/// built-in agent may still declare `ModelSpec::Hint("vision")`.
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
    use crate::config::{
        MODEL_IMAGE_GENERATION_AGENT, MODEL_MEDIA_UNDERSTANDING, MODEL_VIDEO_GENERATION_AGENT,
    };

    for (agent_id, expected_model) in [
        ("vision_agent", MODEL_MEDIA_UNDERSTANDING),
        ("image_agent", MODEL_IMAGE_GENERATION_AGENT),
        ("video_agent", MODEL_VIDEO_GENERATION_AGENT),
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
