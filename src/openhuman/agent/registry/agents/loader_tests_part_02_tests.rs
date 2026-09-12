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
    // not just the tool implementations. It also has read-only grounding
    // in the user's memory via `memory_recall` (direct lookups) and
    // `memory_hybrid_search` (keyword/lexical lookups — pairs with
    // `memory_recall` the same way the sibling `flow_discovery` agent
    // does) — no `memory_store`, so it can look up context but never
    // write it.
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
                "list_agent_profiles",
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
                "memory_recall",
                "memory_hybrid_search",
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
                // Memory access must stay read-only: no write tool.
                "memory_store",
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
                "memory_recall",
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
                "memory_store",
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
    let scheduler = find("scheduler_agent");
    assert!(matches!(scheduler.model, ModelSpec::Hint(ref h) if h == "burst"));
    match &scheduler.tools {
        ToolScope::Named(names) => {
            for required in ["current_time", "cron_add", "cron_list", "cron_remove"] {
                assert!(
                    names.iter().any(|name| name == required),
                    "scheduler_agent missing `{required}`"
                );
            }
        }
        other => panic!("scheduler_agent must use Named tool scope, got {other:?}"),
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
                assert!(!names.iter().any(|name| name == "call_memory_agent"));
                assert!(names.iter().any(|name| name == "web_search_tool"));
            }
            other => panic!("presentation_agent must use Named tool scope, got {other:?}"),
        }
        // Memory pre-fetch is no longer eager; `omit_memory_context = false`
        // still gives the deck builder the cheap per-turn recall.
        assert_eq!(presentation.trigger_memory_agent, TriggerMemoryAgent::Never);
    }
}

#[test]
fn archivist_runs_in_background() {
    let def = find("archivist");
    assert!(def.background);
    assert_eq!(def.max_iterations, 3);
}

#[test]
fn morning_briefing_is_read_only() {
    let def = find("morning_briefing");
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert!(matches!(def.tools, ToolScope::Wildcard));
    // The brief pulls its own last-24h memory via the `memory_tree`
    // `cover_window` tool, so the stale all-time memory blob is suppressed.
    assert!(def.omit_memory_context);
    assert!(def.omit_identity);
    assert!(def.omit_safety_preamble);
    assert_eq!(def.max_iterations, 8);
}

#[test]
fn help_uses_gitbooks_tools_and_is_read_only() {
    let def = find("help");
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    match &def.tools {
        ToolScope::Named(tools) => {
            assert!(
                tools.iter().any(|t| t == "gitbooks_search"),
                "help needs gitbooks_search"
            );
            assert!(
                tools.iter().any(|t| t == "gitbooks_get_page"),
                "help needs gitbooks_get_page"
            );
            assert!(!tools.iter().any(|t| t == "call_memory_agent"));
            // Help is docs-only — no write/exec tools.
            assert!(!tools.iter().any(|t| t == "shell"));
            assert!(!tools.iter().any(|t| t == "file_write"));
            assert!(!tools.iter().any(|t| t == "curl"));
            assert!(!tools.iter().any(|t| t == "spawn_subagent"));
        }
        ToolScope::Wildcard => panic!("help must have a Named tool scope"),
    }
    assert!(def.omit_identity);
    assert!(def.omit_safety_preamble);
    assert!(!def.omit_memory_context);
    // Help personalises from the cheap per-turn recall (memory_context on),
    // so it no longer pre-fetches the full memory agent before every turn.
    assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
}

#[test]
fn orchestrator_and_nested_agents_do_not_expose_agent_prepare_context() {
    // First-turn context preparation is owned by the harness. Keeping the
    // direct tool out of the orchestrator scope prevents a duplicate scout
    // pass after the harness has already prepared context.
    let orch = find("orchestrator");
    if let ToolScope::Named(tools) = &orch.tools {
        assert!(
            !tools.iter().any(|t| t == "agent_prepare_context"),
            "orchestrator must NOT allowlist `agent_prepare_context`"
        );
    }
    // The planner must NOT: when invoked via delegate_plan it runs under
    // the orchestrator's PARENT_CONTEXT, so a nested scout would render the
    // wrong (orchestrator) visible catalog/session.
    let planner = find("planner");
    if let ToolScope::Named(tools) = &planner.tools {
        assert!(
            !tools.iter().any(|t| t == "agent_prepare_context"),
            "planner must NOT allowlist `agent_prepare_context` (nested-context mismatch)"
        );
    }
    // The scout itself must NOT see the tool (would be circular).
    let scout = find("context_scout");
    if let ToolScope::Named(tools) = &scout.tools {
        assert!(!tools.iter().any(|t| t == "agent_prepare_context"));
    }
}

#[test]
fn context_scout_is_read_only_worker_with_bounded_output() {
    let def = find("context_scout");
    assert_eq!(def.agent_tier, AgentTier::Worker);
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    // The context scout rides the cheap, high-throughput `burst` tier
    // (resolves to `burst-v1` on the managed backend), not the pricier
    // agentic/reasoning tiers.
    assert!(
        matches!(&def.model, ModelSpec::Hint(h) if h == "burst"),
        "context_scout must spawn on the burst tier, got {:?}",
        def.model
    );
    // Bundle cap — load-bearing for the parent's context budget. Leaves
    // room for the `recommended_skills` block alongside summary + plan.
    assert_eq!(def.max_result_chars, Some(5000));
    // Keeps goals/profile + long-term memory so it can ground the
    // orchestrator in who the user is and what they want.
    assert!(!def.omit_profile, "context_scout needs PROFILE.md (goals)");
    assert!(!def.omit_memory_md, "context_scout needs MEMORY.md");
    // Strictly read-only gathering surface — no writes / shell / delegation.
    match &def.tools {
        ToolScope::Named(tools) => {
            for required in [
                "memory_recall",
                // Transcripts + thread metadata + message reader (read-only).
                // Skill discovery (read-only).
                "list_workflows",
                "skill_registry_browse",
                "skill_registry_search",
                // Web.
                "web_search_tool",
                "web_fetch",
            ] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "context_scout needs read-only gathering tool `{required}`"
                );
            }
            for forbidden in [
                "shell",
                "file_write",
                "spawn_subagent",
                "spawn_async_subagent",
                "agent_prepare_context",
                // memory_tree bundles a write mode (ingest_document) under a
                // ReadOnly wrapper — must not be reachable by the auto-run scout.
                "memory_tree",
                // Write-capable thread + skill tools must stay out of the
                // auto-run, prompt-injectable scout.
                "thread_create",
                "thread_delete",
                "skill_registry_install",
                "skill_registry_uninstall",
            ] {
                assert!(
                    !tools.iter().any(|t| t == forbidden),
                    "context_scout must NOT have `{forbidden}` — it only gathers context"
                );
            }
        }
        ToolScope::Wildcard => panic!("context_scout must have a Named tool scope"),
    }
    // Worker leaf: no onward delegation.
    assert!(
        def.subagents.is_empty(),
        "context_scout is a leaf and must not list subagents"
    );
}

#[cfg(feature = "flows")]
#[test]
fn flow_memory_agent_is_read_only_worker_with_bounded_memory_belt() {
    let def = find("flow_memory_agent");
    assert_eq!(def.agent_tier, AgentTier::Worker);
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert!(
        matches!(&def.model, ModelSpec::Hint(h) if h == "burst"),
        "flow_memory_agent must spawn on the burst tier, got {:?}",
        def.model
    );
    // Bundle cap — load-bearing for the flow's context budget.
    assert_eq!(def.max_result_chars, Some(4000));
    // Keeps goals/profile + long-term memory so it can ground retrieval
    // in who the user is and what they want.
    assert!(
        !def.omit_profile,
        "flow_memory_agent needs PROFILE.md (goals)"
    );
    assert!(!def.omit_memory_md, "flow_memory_agent needs MEMORY.md");
    // Strictly bounded read-only memory/context belt — exactly 8 tools,
    // no more, no less.
    match &def.tools {
        ToolScope::Named(tools) => {
            let expected = ["memory_recall", "memory_hybrid_search", "memory_flavour"];
            for required in expected {
                assert!(
                    tools.iter().any(|t| t == required),
                    "flow_memory_agent needs read-only belt tool `{required}`"
                );
            }
            assert_eq!(
                tools.len(),
                expected.len(),
                "flow_memory_agent scope must be EXACTLY the bounded read-only \
                 memory belt (got {tools:?})"
            );
            for forbidden in [
                // `memory_tree` bundles a write mode (`ingest_document`)
                // under a ReadOnly-declared wrapper — must never be
                // reachable by this auto-run, prompt-injectable agent.
                "memory_tree",
                "memory_store",
                "update_memory_md",
                "shell",
                "file_write",
                "spawn_subagent",
                "web_search_tool",
                "web_fetch",
            ] {
                assert!(
                    !tools.iter().any(|t| t == forbidden),
                    "flow_memory_agent must NOT have `{forbidden}` — it only \
                     retrieves memory/context"
                );
            }
        }
        ToolScope::Wildcard => panic!("flow_memory_agent must have a Named tool scope"),
    }
    // Worker leaf: no onward delegation.
    assert!(
        def.subagents.is_empty(),
        "flow_memory_agent is a leaf and must not list subagents"
    );
}

#[test]
fn chatty_sub_agents_have_bounded_output() {
    // critic + archivist results flow up to the orchestrator verbatim
    // (delegate_critic / delegate_archivist). Without a cap their output
    // is unbounded and bloats the orchestrator's context (#4099). Both
    // must carry the normal sub-agent cap so a long diff review or a
    // verbose memory-write confirmation can't leak unbounded text.
    assert_eq!(
        find("critic").max_result_chars,
        Some(8000),
        "critic output must be bounded so reviews don't leak unbounded text up"
    );
    assert_eq!(
        find("archivist").max_result_chars,
        Some(8000),
        "archivist output must be bounded so memory summaries stay concise"
    );
}

#[test]
fn researcher_is_bounded_to_search_and_fetch() {
    let def = find("researcher");
    assert_eq!(
        def.max_iterations, 10,
        "researcher keeps enough turns to recover from bad search results without broadening its tool surface"
    );
    assert_eq!(
        def.max_turn_output_tokens,
        Some(4096),
        "researcher must cap each model turn so verbose research loops cannot flood context"
    );
    assert!(
        def.extra_tools.is_empty(),
        "researcher must not widen its tool surface via extra_tools"
    );
    match &def.tools {
        ToolScope::Named(tools) => {
            assert_eq!(
                tools,
                &vec!["web_search_tool".to_string(), "web_fetch".to_string()],
                "researcher must stay limited to search+fetch so simple lookups do not fan out into deep research loops"
            );
        }
        ToolScope::Wildcard => panic!("researcher must have Named tool scope"),
    }
}

#[test]
fn code_executor_has_curl_for_artifact_downloads() {
    let def = find("code_executor");
    match &def.tools {
        ToolScope::Named(tools) => {
            assert!(
                tools.iter().any(|t| t == "curl"),
                "code_executor needs curl for artifact/dataset fetches"
            );
        }
        ToolScope::Wildcard => panic!("code_executor must have Named tool scope"),
    }
}

#[test]
fn orchestrator_does_not_get_curl() {
    // Per design: curl is a `Write` permission tool that writes
    // to the workspace. The orchestrator delegates rather than
    // executing — code_executor / tools_agent own actual downloads.
    let def = find("orchestrator");
    if let ToolScope::Named(tools) = &def.tools {
        assert!(
            !tools.iter().any(|t| t == "curl"),
            "orchestrator must not have curl — it should delegate"
        );
    }
}
