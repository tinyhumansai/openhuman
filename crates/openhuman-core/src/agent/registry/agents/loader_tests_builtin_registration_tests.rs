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
            // A worker with no `[subagents]` allowlist can never dispatch a
            // spawn, so listing the tool only advertised a dead route.
            assert!(
                !tools.iter().any(|t| t == "spawn_subagent"),
                "trigger_reactor cannot escalate by spawning; it must not list spawn_subagent"
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
    // #4291: when a delegated sub-agent (e.g. task_manager_agent) pauses on
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
    // The coding playbook is the `coding` skill's guide now (the
    // `code_executor` specialist it replaced is gone).
    let coding = include_str!("../../../tools/toolpacks/guides/coding.md");
    assert!(
        !coding.contains("sandboxed environment"),
        "guides/coding.md says 'sandboxed environment' generically — anchor in \
         the action sandbox path (see #3236)"
    );
    assert!(
        coding.contains("action sandbox") || coding.contains("action_dir"),
        "guides/coding.md must reference the action sandbox or action_dir (see #3236)"
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
    use crate::agent::prompts::{
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
fn vision_agent_loads_on_its_pinned_multimodal_model() {
    // The vision sub-agent used to ride the multimodal `vision-v1` tier (via
    // the `vision` hint), which is now deprecated — `vision-v1` silently
    // falls back to the chat default on managed routes (regression R4). It
    // is pinned to a dedicated OpenRouter passthrough model instead, and
    // must remain reachable from the orchestrator's subagent allowlist.
    let def = find("vision_agent");
    assert!(matches!(
        def.model,
        ModelSpec::Exact(ref m) if m == crate::config::MODEL_MEDIA_UNDERSTANDING
    ));

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
            // The unified `memory` tool handles direct recall, keyword
            // search, writes, and forgetting without a sub-agent round-trip.
            // Preferences retain their dedicated direct tool.
            for direct in ["memory", "save_preference"] {
                assert!(
                    tools.iter().any(|t| t == direct),
                    "orchestrator must have direct memory tool `{direct}` (#4762)"
                );
            }
        }
        ToolScope::Wildcard => panic!("orchestrator must have named tool allowlist"),
    }
    // #6958: a coding turn routinely needs 50-180 model calls (every other
    // harness on the DeepSWE sample did); 50 stopped all ten tasks before a
    // single source edit.
    assert_eq!(def.max_iterations, 200);
    // Memory retrieval is on-demand (via the `agent_memory` subagent,
    // surfaced as `delegate_retrieve_memory`), not an eager pre-turn
    // pre-fetch. The allowlist entry is what makes that route reachable
    // (see the `agent_memory::tools` allowlist gate).
    assert_eq!(def.trigger_memory_agent, TriggerMemoryAgent::Never);
}

/// Regression guard for the `resolve_time` wiring. Agents that emit
/// timestamp arguments to downstream tools must keep the deterministic
/// time resolver in their allowlist — otherwise the model falls back to
/// hand-computing epoch seconds, which once produced a ~10-month-wrong
/// `oldest` and silently fetched the wrong Slack window. If any of these
/// drops `resolve_time`, this test fails loudly.
#[test]
fn time_sensitive_agents_expose_resolve_time() {
    let ids = vec!["orchestrator", "task_manager_agent"];
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
fn broad_agent_surfaces_expose_storage_transfer_not_lifecycle_tools() {
    for id in ["orchestrator"] {
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
/// install/connect mutators. Execution stays with the orchestrator.
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
                     belongs to the orchestrator"
                );
            }
        }
        other => panic!("planner must use Named tool scope, got {other:?}"),
    }
}

/// The archivist is registered but deliberately not a chat delegate: the
/// post-commit session-memory extraction runs it by id
/// (`runtime_session.rs`), and a delegate would add its schema to every turn.
#[test]
fn the_orchestrator_does_not_delegate_to_the_archivist() {
    let orchestrator = find("orchestrator");
    assert!(
        !orchestrator
            .subagents
            .iter()
            .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == "archivist")),
        "`archivist` is back on the orchestrator's subagent list"
    );
    assert_eq!(
        find("archivist").id,
        "archivist",
        "archivist must stay registered"
    );
}

/// The specialists the inline skills replaced must not come back as
/// orchestrator delegates, and the ones kept for workflow runs must not
/// become chat delegates.
#[test]
fn the_orchestrator_lists_no_replaced_or_workflow_only_specialist() {
    let orchestrator = find("orchestrator");
    for id in [
        "planner",
        "critic",
        "code_executor",
        "settings_agent",
        "scheduler_agent",
        "crypto_agent",
        "mcp_agent",
        "tools_agent",
        "help",
        "skill_creator",
        "skill_executor",
        "integrations_agent",
        "tool_maker",
        "context_scout",
    ] {
        assert!(
            !orchestrator
                .subagents
                .iter()
                .any(|entry| matches!(entry, SubagentEntry::AgentId(listed) if listed == id)),
            "`{id}` must not be an orchestrator delegate"
        );
    }
}

#[test]
fn orchestrator_omits_removed_prompt_tools() {
    let orchestrator = find("orchestrator");
    let ToolScope::Named(tools) = &orchestrator.tools else {
        panic!("orchestrator must have a named tool scope");
    };
    for removed in ["request_plan_review", "plan_exit", "update_memory_md"] {
        assert!(
            !tools.iter().any(|tool| tool == removed),
            "`{removed}` must not appear in the orchestrator tool list"
        );
    }
    for removed in ["critic", "planner"] {
        assert!(
            !orchestrator
                .subagents
                .iter()
                .any(|entry| matches!(entry, SubagentEntry::AgentId(id) if id == removed)),
            "`{removed}` adds a delegate tool to the orchestrator prompt"
        );
    }
}
