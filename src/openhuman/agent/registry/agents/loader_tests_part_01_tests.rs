use super::*;

#[test]
fn all_builtins_parse() {
    let defs = load_builtins().expect("built-in TOML must parse");
    // `load_builtins` filters feature-gated built-ins (e.g. `presentation_agent`
    // when `documents` is off), so compare against the same filtered count
    // rather than the raw `BUILTINS` length.
    let expected = BUILTINS.iter().filter(|b| builtin_enabled(b)).count();
    assert_eq!(defs.len(), expected);
}

/// Pins the `presentation_agent` compile-time gate, both directions: it is
/// registered under the `documents` feature (its `generate_presentation`
/// deck tool lives there) and filtered out of the registry without it, so
/// slim builds never advertise `make_presentation` with no tool to fulfil it.
#[cfg(feature = "documents")]
#[test]
fn presentation_agent_registered_when_documents_on() {
    let defs = load_builtins().expect("built-in TOML must parse");
    assert!(
        defs.iter().any(|d| d.id == "presentation_agent"),
        "presentation_agent must register when the `documents` feature is on"
    );
}

#[cfg(not(feature = "documents"))]
#[test]
fn presentation_agent_absent_when_documents_off() {
    let defs = load_builtins().expect("built-in TOML must parse");
    assert!(
        !defs.iter().any(|d| d.id == "presentation_agent"),
        "presentation_agent must be filtered from the registry when `documents` is off"
    );
}

#[test]
fn automatic_memory_agents_do_not_expose_call_memory_agent() {
    for def in load_builtins().expect("built-in TOML must parse") {
        if def.trigger_memory_agent != TriggerMemoryAgent::Always {
            continue;
        }

        let exposes_call_memory_agent = match &def.tools {
            ToolScope::Named(tools) => tools.iter().any(|tool| tool == "call_memory_agent"),
            ToolScope::Wildcard => false,
        };

        assert!(
            !exposes_call_memory_agent,
            "{} uses trigger_memory_agent but still exposes call_memory_agent",
            def.id
        );
        assert!(
            !def.subagents
                .iter()
                .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == "agent_memory")),
            "{} uses trigger_memory_agent but still lists agent_memory in subagents",
            def.id
        );
    }
}

#[test]
fn trigger_reactor_has_agentic_hint_and_narrow_tools() {
    let def = find("trigger_reactor");
    assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "agentic"));
    match &def.tools {
        ToolScope::Named(tools) => {
            assert!(!tools.iter().any(|t| t == "call_memory_agent"));
            assert!(
                tools.iter().any(|t| t == "memory_store"),
                "trigger_reactor needs memory_store"
            );
            assert!(
                tools.iter().any(|t| t == "spawn_subagent"),
                "trigger_reactor needs spawn_subagent for escalation"
            );
            // No shell / file_write — reactor does not execute code.
            assert!(!tools.iter().any(|t| t == "shell"));
            assert!(!tools.iter().any(|t| t == "file_write"));
        }
        ToolScope::Wildcard => panic!("trigger_reactor must have a Named tool scope"),
    }
    assert_eq!(def.sandbox_mode, SandboxMode::None);
    assert_eq!(def.max_iterations, 6);
    assert!(
        !def.omit_memory_context,
        "trigger_reactor needs global memory/context"
    );
}

#[test]
fn orchestrator_can_resume_paused_subagents_via_continue_subagent() {
    // #4291: when a delegated sub-agent (e.g. mcp_setup) pauses on
    // ask_user_clarification, the orchestrator gets a
    // [SUBAGENT_AWAITING_USER] envelope and must resume that exact
    // checkpoint with `continue_subagent`. Without the tool in scope the
    // only continuation is to re-delegate a fresh, stateless sub-agent
    // that asks again — the infinite re-spawn loop. Lock the tool in.
    let def = find("orchestrator");
    match &def.tools {
        ToolScope::Named(tools) => assert!(
            tools.iter().any(|t| t == "continue_subagent"),
            "orchestrator must expose continue_subagent to resume paused \
             sub-agents instead of re-spawning them (#4291)"
        ),
        ToolScope::Wildcard => {
            panic!("orchestrator must have a Named tool scope")
        }
    }
}

#[test]
fn trigger_triage_has_no_tools_and_pulls_memory_context() {
    let def = find("trigger_triage");
    match &def.tools {
        ToolScope::Named(tools) => assert!(
            tools.is_empty(),
            "trigger_triage must have zero tools (got {tools:?})"
        ),
        ToolScope::Wildcard => panic!("trigger_triage must have a Named empty tool scope"),
    }
    assert!(
        !def.omit_memory_context,
        "trigger_triage needs global memory/context to reason about triggers"
    );
    assert!(def.omit_identity);
    assert!(def.omit_safety_preamble);
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert_eq!(def.max_iterations, 2);
}

#[test]
fn folder_ids_match_toml_ids() {
    for b in BUILTINS {
        let def = parse_builtin(b).expect("parse");
        assert_eq!(def.id, b.id, "folder `{}` id mismatch", b.id);
    }
}

/// Regression guard for #3236.
///
/// PR #3074 introduced the `Config.action_dir` / `Config.workspace_dir`
/// split: acting tools resolve to `action_dir` (default
/// `~/OpenHuman/projects`), and `workspace_dir` is reserved for
/// internal product state (memory / sessions / vault / etc.) that is
/// denied to agent tools. The coding-agent prompts must reflect that
/// split — saying "in a sandboxed environment" or "the workspace has
/// code …" without anchoring contradicts the new model and steers
/// the model toward paths that hit the internal-state denylist.
///
/// If a future edit reintroduces stale phrasing, this assertion fires
/// at `cargo test` time before the bad prompt ships.
#[test]
fn coding_agent_prompts_reference_action_sandbox_not_stale_workspace() {
    let code_executor = include_str!("code_executor/prompt.md");
    assert!(
        !code_executor.contains("sandboxed environment"),
        "code_executor/prompt.md still says 'sandboxed environment' \
         generically — anchor in the action sandbox path (see #3236)"
    );
    assert!(
        code_executor.contains("action sandbox") || code_executor.contains("action_dir"),
        "code_executor/prompt.md must reference the action sandbox or action_dir (see #3236)"
    );

    let planner = include_str!("planner/prompt.md");
    assert!(
        !planner.contains("the workspace has code"),
        "planner/prompt.md still says 'the workspace has code …' — \
         use 'the project tree' or similar to avoid colliding with \
         `Config.workspace_dir` (internal product state). See #3236."
    );
}

#[test]
fn every_builtin_has_a_prompt_body() {
    use crate::openhuman::agent::context::prompt::{
        ConnectedIntegration, LearnedContextData, PromptContext, PromptTool, ToolCallFormat,
    };
    let empty_tools: Vec<PromptTool<'_>> = Vec::new();
    let empty_integrations: Vec<ConnectedIntegration> = Vec::new();
    let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
    for def in load_builtins().unwrap() {
        match &def.system_prompt {
            PromptSource::Dynamic(build) => {
                let ctx = PromptContext {
                    workspace_dir: std::path::Path::new("."),
                    model_name: "test",
                    agent_id: &def.id,
                    tools: &empty_tools,
                    workflows: &[],
                    dispatcher_instructions: "",
                    learned: LearnedContextData::default(),
                    visible_tool_names: &empty_visible,
                    tool_call_format: ToolCallFormat::PFormat,
                    connected_integrations: &empty_integrations,
                    connected_identities_md: String::new(),
                    include_profile: false,
                    include_memory_md: false,
                    curated_snapshot: None,
                    user_identity: None,
                    personality_soul_md: None,
                    personality_memory_md: None,
                    personality_roster: vec![],
                    agents_md_global: None,
                    agents_md_local: None,
                };
                let body =
                    build(&ctx).unwrap_or_else(|e| panic!("{} prompt build failed: {e}", def.id));
                assert!(!body.is_empty(), "{} has empty prompt", def.id);
            }
            PromptSource::Inline(_) | PromptSource::File { .. } => {
                panic!("{} should use dynamic prompt builder", def.id);
            }
        }
    }
}

#[test]
fn every_builtin_is_stamped_builtin_source() {
    for def in load_builtins().unwrap() {
        assert_eq!(def.source, DefinitionSource::Builtin);
    }
}

#[test]
fn vision_agent_loads_on_vision_hint() {
    // The vision sub-agent rides the multimodal `vision-v1` tier (via the
    // `vision` hint) so its model is image-capable, and it must be reachable
    // from the orchestrator's subagent allowlist.
    let def = find("vision_agent");
    assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "vision"));

    let orchestrator = find("orchestrator");
    assert!(
        orchestrator
            .subagents
            .iter()
            .any(|s| matches!(s, SubagentEntry::AgentId(id) if id == "vision_agent")),
        "orchestrator must list vision_agent in its subagents allowlist"
    );

    assert!(
        !BUILTINS
            .iter()
            .any(|builtin| builtin.id == "screen_awareness_agent"),
        "screen_awareness_agent must not remain a discoverable built-in"
    );
    assert!(
        !orchestrator.subagents.iter().any(
            |entry| matches!(entry, SubagentEntry::AgentId(id) if id == "screen_awareness_agent")
        ),
        "orchestrator must not expose a screen_awareness_agent delegate"
    );
    assert!(
        load_builtins()
            .expect("built-in TOML must parse")
            .iter()
            .all(|definition| definition.id != "screen_awareness_agent"),
        "screen_awareness_agent must not load into the built-in registry"
    );

    match def.tools {
        ToolScope::Named(ref tools) => assert_eq!(
            tools,
            &vec!["file_read".to_string(), "image_info".to_string()],
            "vision_agent must only inspect user-provided attached or on-disk images"
        ),
        ToolScope::Wildcard => {
            panic!("vision_agent must keep a narrow user-image tool allowlist")
        }
    }
}

#[test]
fn low_context_workers_use_burst_hint() {
    for id in [
        "researcher",
        "context_scout",
        // NOTE: `flow_memory_agent` is intentionally NOT listed here. It is
        // a `#[cfg(feature = "flows")]` agent, and an array literal can't
        // carry a per-element `cfg`; its burst hint is covered by the
        // gated `flow_memory_agent_is_read_only_worker_with_bounded_memory_belt`
        // test instead.
        "integrations_agent",
        "tools_agent",
        "crypto_agent",
        "scheduler_agent",
    ] {
        let def = find(id);
        assert!(
            matches!(def.model, ModelSpec::Hint(ref h) if h == "burst"),
            "{id} should use the burst worker tier"
        );
    }
}

#[test]
fn master_agent_has_coding_hint_and_named_tools() {
    let def = find("orchestrator");
    assert_eq!(def.display_name.as_deref(), Some("Master Agent"));
    assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "coding"));
    assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
    match def.tools {
        ToolScope::Named(tools) => {
            // spawn_subagent was removed in #1141. spawn_worker_thread is
            // disabled pending its UI (#1624) and unregistered, so the
            // named scope must not advertise it.
            assert!(
                !tools.iter().any(|t| t == "spawn_worker_thread"),
                "spawn_worker_thread is disabled (#1624) and must not be named"
            );
            // Sub-agent surface taught by prompt.md, deliberately three
            // tools (#5701): spawn, enumerate, resume. A sub-agent is
            // always async and its result is delivered back on an idle
            // system turn, so there is nothing to collect and nothing to
            // block on.
            for required in [
                "spawn_async_subagent",
                "list_subagents",
                "continue_subagent",
            ] {
                assert!(
                    tools.iter().any(|t| t == required),
                    "orchestrator must have sub-agent tool `{required}`"
                );
            }
            // The collection/fan-out/fleet surface these replaced. Each was
            // either a second way to say "spawn again" or a way to stall
            // the turn waiting for a result that arrives on its own.
            // Re-adding one means re-teaching it in prompt.md; don't do it
            // without that.
            for retired in [
                "wait",
                "wait_loop",
                "wait_subagent",
                "spawn_parallel_agents",
                "steer_subagent",
                "close_subagent",
            ] {
                assert!(
                    !tools.iter().any(|t| t == retired),
                    "retired sub-agent tool `{retired}` must not reappear (#5701)"
                );
            }
            assert!(
                !tools.iter().any(|t| t == "spawn_subagent"),
                "spawn_subagent must not appear — removed in #1141"
            );
            assert!(!tools.iter().any(|t| t == "call_memory_agent"));
            // The Master Agent owns the ordinary coding loop directly.
            // Keep its mutation surface intentionally small: one patch
            // mechanism for existing files, file_write for new files,
            // shell for execution, and native git operations.
            for direct in ["shell", "file_write", "apply_patch", "git_operations"] {
                assert!(
                    tools.iter().any(|t| t == direct),
                    "Master Agent must have direct coding tool `{direct}`"
                );
            }
            for forbidden in [
                "edit",
                "curl",
                "storage_set_visibility",
                "storage_delete_file",
            ] {
                assert!(
                    !tools.iter().any(|t| t == forbidden),
                    "Master Agent must NOT have redundant or lifecycle tool `{forbidden}`"
                );
            }
            // Inspect tools remain direct for the normal coding loop and
            // quick non-code lookups.
            for direct in [
                "file_read",
                "grep",
                "glob",
                "list",
                "web_search_tool",
                "web_fetch",
                "http_request",
            ] {
                assert!(
                    tools.iter().any(|t| t == direct),
                    "Master Agent must have direct inspect tool `{direct}`"
                );
            }
            // Direct memory surface (#4762): recall/store are the product's
            // core and must be first-class direct tools, not a sub-agent
            // spawn — a trivial recall or a single "remember this" must not
            // pay a blocking agentic round-trip (over-delegation, #4744) that
            // can hang or return a 0-char result with persistence unconfirmed.
            // Deep tree walks / reconciliation still delegate to
            // `retrieve_memory` / `manage_profile_memory`.
            for direct in ["memory_recall", "memory_store", "save_preference"] {
                assert!(
                    tools.iter().any(|t| t == direct),
                    "orchestrator must have direct memory tool `{direct}` (#4762)"
                );
            }
            // Memory-protocol close-out (#4116): a direct `memory_store` write
            // obliges an `update_memory_md` index reconcile, so the tool that
            // performs it must be in scope — otherwise the protocol's guidance
            // is unsatisfiable and MEMORY.md (loaded here) drifts from the store.
            assert!(
                tools.iter().any(|t| t == "update_memory_md"),
                "orchestrator must have `update_memory_md` to reconcile MEMORY.md \
                 after a direct memory_store (#4762)"
            );
        }
        ToolScope::Wildcard => panic!("orchestrator must have named tool allowlist"),
    }
    assert_eq!(def.max_iterations, 15);
    // Memory retrieval is on-demand (via the `agent_memory` subagent,
    // surfaced as `delegate_retrieve_memory`), not an eager pre-turn
    // pre-fetch. The allowlist entry is what makes that route reachable
    // (see the `agent_memory::tools` allowlist gate).
    assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
    assert!(
        def.subagents.iter().any(|entry| matches!(
            entry,
            SubagentEntry::AgentId(id) if id == "agent_memory"
        )),
        "orchestrator must allow `agent_memory` for on-demand retrieval"
    );
}

/// Regression guard for the `resolve_time` wiring. Agents that emit
/// timestamp arguments to downstream tools must keep the deterministic
/// time resolver in their allowlist — otherwise the model falls back to
/// hand-computing epoch seconds, which once produced a ~10-month-wrong
/// `oldest` and silently fetched the wrong Slack window. If any of these
/// drops `resolve_time`, this test fails loudly.
#[test]
fn time_sensitive_agents_expose_resolve_time() {
    let ids = vec![
        "orchestrator",
        "integrations_agent",
        "scheduler_agent",
        "task_manager_agent",
        "crypto_agent",
    ];
    for id in ids {
        let def = find(id);
        match def.tools {
            ToolScope::Named(tools) => assert!(
                tools.iter().any(|t| t == "resolve_time"),
                "{id} must keep `resolve_time` in its named tool allowlist"
            ),
            ToolScope::Wildcard => {
                // Wildcard agents inherit the full built-in surface, which
                // already includes resolve_time — nothing to assert here.
            }
        }
    }
}

#[test]
fn code_executor_is_sandboxed_and_keeps_safety_preamble() {
    let def = find("code_executor");
    assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
    assert!(!def.omit_safety_preamble);
    assert_eq!(def.max_iterations, 10);
    assert_eq!(
        def.effective_tokenjuice_compression(),
        AgentTokenjuiceCompression::Light
    );
}

#[test]
fn broad_agent_surfaces_expose_storage_transfer_not_lifecycle_tools() {
    for id in ["code_executor", "integrations_agent", "orchestrator"] {
        let def = find(id);
        match &def.tools {
            ToolScope::Named(tools) => {
                for required in [
                    "storage_upload_file",
                    "storage_download_file",
                    "storage_list_files",
                    "storage_get_link",
                ] {
                    assert!(
                        tools.iter().any(|t| t == required),
                        "{id} must expose storage transfer tool `{required}`"
                    );
                }
                for forbidden in ["storage_set_visibility", "storage_delete_file"] {
                    assert!(
                        !tools.iter().any(|t| t == forbidden),
                        "{id} must not expose storage lifecycle tool `{forbidden}`"
                    );
                }
            }
            ToolScope::Wildcard => panic!("{id} must have Named tool scope"),
        }
    }
}

#[test]
fn tool_maker_is_sandboxed_with_max_2_iterations() {
    let def = find("tool_maker");
    assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
    assert_eq!(def.max_iterations, 2);
    assert!(!def.omit_safety_preamble);
    assert_eq!(
        def.effective_tokenjuice_compression(),
        AgentTokenjuiceCompression::Light
    );
}

#[test]
fn skill_creator_is_sandboxed_and_has_node_tools() {
    let def = find("skill_creator");
    assert_eq!(def.sandbox_mode, SandboxMode::Sandboxed);
    assert_eq!(def.max_iterations, 10);
    assert!(!def.omit_safety_preamble);
    assert_eq!(
        def.effective_tokenjuice_compression(),
        AgentTokenjuiceCompression::Light
    );
    match &def.tools {
        ToolScope::Named(names) => {
            for required in ["node_exec", "npm_exec", "apply_patch", "update_memory_md"] {
                assert!(
                    names.iter().any(|name| name == required),
                    "skill_creator tool list missing `{required}`"
                );
            }
        }
        ToolScope::Wildcard => panic!("skill_creator must have named tool allowlist"),
    }
}

#[test]
fn critic_is_read_only() {
    let def = find("critic");
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    assert!(def.omit_safety_preamble);
}

/// Planner runs `composio_execute` so it can ground plans in real
/// integration data, but it must stay strictly read-only — issue
/// #685. `sandbox_mode = "read_only"` in `planner/agent.toml` is the
/// runtime hook that activates the agent-level gate inside
/// `ComposioExecuteTool::execute`; this test pins that contract so a
/// future TOML edit that drops the sandbox mode can never silently
/// turn the planner into a write-capable agent.
#[test]
fn planner_is_read_only_with_composio_meta_tools() {
    let def = find("planner");
    assert_eq!(
        def.sandbox_mode,
        SandboxMode::ReadOnly,
        "planner.sandbox_mode must be read_only — gates Write/Admin composio actions",
    );
    match &def.tools {
        ToolScope::Named(names) => {
            for required in [
                "composio_list_toolkits",
                "composio_list_connections",
                "composio_list_tools",
                "composio_execute",
            ] {
                assert!(
                    names.iter().any(|n| n == required),
                    "planner tool list missing `{required}` — composio meta-tools must \
                     all be present so the planner can inspect integrations under the \
                     read-only sandbox gate",
                );
            }
        }
        other => panic!("planner must use Named tool scope, got {other:?}"),
    }
}

/// The planner grounds plans in connected-MCP context the same way it
/// grounds in Composio — but read-only. It must carry the MCP *discovery*
/// tools (`status` / `installed_list` / `list_tools`, all
/// `PermissionLevel::ReadOnly`) and must NOT carry `mcp_registry_tool_call`
/// (no read-only gate exists for an arbitrary MCP tool call) nor the
/// install/connect mutators. Execution stays with `mcp_agent`.
#[test]
fn planner_has_readonly_mcp_discovery_not_execute() {
    let def = find("planner");
    assert_eq!(def.sandbox_mode, SandboxMode::ReadOnly);
    match &def.tools {
        ToolScope::Named(names) => {
            for required in [
                "mcp_registry_status",
                "mcp_registry_installed_list",
                "mcp_registry_list_tools",
            ] {
                assert!(
                    names.iter().any(|n| n == required),
                    "planner needs read-only MCP discovery tool `{required}`"
                );
            }
            for forbidden in [
                "mcp_registry_tool_call",
                "mcp_registry_connect",
                "mcp_registry_install",
                "mcp_registry_uninstall",
            ] {
                assert!(
                    !names.iter().any(|n| n == forbidden),
                    "planner must NOT have `{forbidden}` — it is read-only; MCP execution \
                     belongs to mcp_agent"
                );
            }
        }
        other => panic!("planner must use Named tool scope, got {other:?}"),
    }
}

#[test]
fn integrations_agent_tool_scope_honours_toml() {
    let def = find("integrations_agent");
    // Current TOML: `named = ["composio_list_tools", "file_read"]`.
    // Sub-agent runner additionally injects per-toolkit
    // ComposioActionTools at spawn time.
    match &def.tools {
        ToolScope::Named(names) => {
            assert!(names.iter().any(|n| n == "composio_list_tools"));
        }
        other => panic!("expected Named scope, got {other:?}"),
    }
    assert!(!def.omit_safety_preamble);
}

#[test]
fn tools_agent_is_registered() {
    let def = find("tools_agent");
    assert!(matches!(def.tools, ToolScope::Wildcard));
}

/// Two agents are deliberately missing from the orchestrator's subagent list.
///
/// Dropping an entry removes a synthesised `delegate_*` schema from every turn
/// without removing the agent — cheaper than packing it, because a packed
/// delegate still costs a row in the prompt's withheld-capability block.
///
/// This test pins registry membership only. It does NOT pin that either
/// agent is dispatchable through `spawn_async_subagent` — today it is not:
/// `execute_with_context_inner` additionally checks
/// `parent.allowed_subagent_ids`, which is derived from this very
/// `subagents.allowlist` (see `session/turn/tools.rs`), so a model that asks
/// for either id gets a clean allowlist error rather than a spawn. See the
/// comment above `[subagents]` in `orchestrator/agent.toml` for the tracked
/// follow-up. Do not read `registry.get(dropped).is_some()` below as "and
/// therefore spawnable" — it only proves the definition was not deleted.
#[test]
fn the_orchestrator_does_not_delegate_to_the_generalist_or_the_archivist() {
    let registry = crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
        .or_else(|| {
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::init_global_builtins().ok()?;
            crate::openhuman::agent::harness::definition::AgentDefinitionRegistry::global()
        })
        .expect("builtin agent definitions must load");
    let orchestrator = registry
        .get("orchestrator")
        .expect("orchestrator is builtin");

    let listed: Vec<&str> = orchestrator
        .subagents
        .iter()
        .filter_map(|entry| match entry {
            crate::openhuman::agent::harness::definition::SubagentEntry::AgentId(id) => {
                Some(id.as_str())
            }
            _ => None,
        })
        .collect();
    for dropped in ["tools_agent", "archivist"] {
        assert!(
            !listed.contains(&dropped),
            "`{dropped}` is back on the orchestrator's subagent list, which \
             re-adds its delegate schema to every turn"
        );
        // Still a real agent, resolvable by id in the registry — NOT a claim
        // that it is currently reachable via spawn_async_subagent (it isn't;
        // see the doc comment above).
        assert!(
            registry.get(dropped).is_some(),
            "`{dropped}` must stay registered — its definition should not be \
             deleted, only dropped from the orchestrator's advertised list"
        );
    }
}
