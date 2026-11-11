use super::*;
use tinytools_agent::dialect::ToolCallFormat;

#[test]
fn all_builtins_parse() {
    let defs = load_builtins().expect("built-in TOML must parse");
    // `load_builtins` filters feature-gated built-ins (e.g. `presentation_agent`
    // when `documents` is off), so compare against the same filtered count
    // rather than the raw `BUILTINS` length.
    let expected = BUILTINS.iter().filter(|b| builtin_enabled(b)).count();
    assert_eq!(defs.len(), expected);
}

#[test]
fn flow_builtin_entries_follow_the_flows_feature_gate() {
    let definitions = load_builtins().expect("built-in TOML must parse");
    for id in ["workflow_builder", "flow_discovery"] {
        assert_eq!(
            definitions.iter().any(|definition| definition.id == id),
            cfg!(feature = "flows"),
            "builtin `{id}` registration must follow the `flows` feature"
        );
    }
}

/// Pins the `presentation_agent` compile-time gate, both directions: it is
/// registered under the `documents` feature (its `generate_presentation`
/// deck tool lives there) and filtered out of the registry without it, so
/// slim builds never advertise a presentation specialist with no deck tool.
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
fn trigger_reactor_has_agentic_hint_and_narrow_tools() {
    let def = find("trigger_reactor");
    assert!(matches!(def.model, ModelSpec::Hint(ref h) if h == "agentic"));
    match &def.tools {
        ToolScope::Named(tools) => {
            assert!(
                tools.iter().any(|t| t == "memory"),
                "trigger_reactor needs the memory tool"
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
        !def.extensions.omit_memory_context,
        "trigger_reactor needs global memory/context"
    );
}

#[test]
fn orchestrator_resumes_paused_subagents_through_spawn_task_key_reuse() {
    let def = find("orchestrator");
    match &def.tools {
        ToolScope::Named(tools) => assert!(
            tools.iter().any(|t| t == "spawn_subagent"),
            "orchestrator must expose the single spawn tool for task_key reuse"
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
        !def.extensions.omit_memory_context,
        "trigger_triage needs global memory/context to reason about triggers"
    );
    assert!(def.extensions.omit_identity);
    assert!(def.extensions.omit_safety_preamble);
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
    let coding = include_str!("../../tools/toolpacks/guides/coding.md");
    assert!(
        !coding.contains("sandboxed environment"),
        "guides/coding.md says 'sandboxed environment' generically — anchor in \
         the action sandbox path (see #3236)"
    );
    assert!(
        coding.contains("action sandbox") || coding.contains("action_dir"),
        "guides/coding.md must reference the action sandbox or action_dir (see #3236)"
    );
}

#[test]
fn every_builtin_has_a_prompt_body() {
    use crate::agent::prompts::{ConnectedIntegration, PromptContext, PromptTool};
    let empty_tools: Vec<PromptTool<'_>> = Vec::new();
    let empty_integrations: Vec<ConnectedIntegration> = Vec::new();
    let empty_visible: std::collections::HashSet<String> = std::collections::HashSet::new();
    for def in load_builtins().unwrap() {
        match &def.system_prompt {
            PromptSource::Dynamic(()) => {
                let build = def
                    .extensions
                    .prompt_builder
                    .expect("dynamic prompt builder");
                let ctx = PromptContext {
                    workspace_dir: std::path::Path::new("."),
                    model_name: "test",
                    agent_id: &def.id,
                    flow_archetype_text: matches!(
                        def.id.as_str(),
                        "workflow_builder" | "flow_discovery"
                    )
                    .then_some("synthetic flow archetype for loader coverage"),
                    tools: &empty_tools,
                    workflows: &[],
                    dispatcher_instructions: "",
                    visible_tool_names: &empty_visible,
                    tool_call_format: ToolCallFormat::PFormat,
                    connected_integrations: &empty_integrations,
                    connected_identities_md: String::new(),
                    user_identity: None,
                    personality_roster: vec![],
                    agents_md_global: None,
                    agents_md_local: None,
                };
                let body =
                    build(&ctx).unwrap_or_else(|e| panic!("{} prompt build failed: {e}", def.id));
                assert!(!body.is_empty(), "{} has empty prompt", def.id);
            }
            PromptSource::Inline(_) | PromptSource::File { .. } | PromptSource::Verbatim(_) => {
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
        ModelSpec::Exact(ref m) if m == "openrouter/qwen/qwen3.5-flash-02-23"
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
            // Specialist routing uses the single asynchronous spawn tool.
            assert!(
                tools.iter().any(|t| t == "spawn_subagent"),
                "orchestrator must have the asynchronous sub-agent tool"
            );
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
                "list_subagents",
                "continue_subagent",
                "spawn_async_subagent",
            ] {
                assert!(
                    !tools.iter().any(|t| t == retired),
                    "retired sub-agent tool `{retired}` must not reappear (#5701)"
                );
            }
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
            // Direct memory surface: recall/store are the product's core and
            // must be one first-class direct tool, not a sub-agent spawn
            // (over-delegation, #4744).
            assert!(
                tools.iter().any(|t| t == "memory"),
                "orchestrator must have the direct `memory` tool"
            );
        }
        ToolScope::Wildcard => panic!("orchestrator must have named tool allowlist"),
    }
    // #6958: coding turns routinely need substantially more than fifty model calls.
    assert_eq!(def.max_iterations, 200);
    assert!(
        !def.extensions.omit_memory_context,
        "the user-facing agent opens new sessions with the memory context"
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
fn the_orchestrator_lists_no_replaced_specialist() {
    let orchestrator = find("orchestrator");
    for id in [
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
}
