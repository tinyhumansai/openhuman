//! The compiled-in pack table.
//!
//! Membership is a build-time decision, deliberately: a pack that config or RPC
//! could edit would let a caller move a dangerous tool out of the advertised
//! surface (or back into it) without review. Adding a pack is a source change.

use tinyagents_harness::tool::packs::{PackCatalog, ToolPack};

/// Every pack this build knows about.
///
/// Chosen by measured schema cost against how often the orchestrator actually
/// needs them: together ~5.9k tokens of the Master Agent's tool-schema budget,
/// idle in the large majority of turns. Measured with tiktoken `o200k_base`
/// against a real `agent dump-all`, not estimated.
///
/// Frequency of use is the whole criterion — see
/// `DELIBERATELY_UNPACKED_FLEET_TOOLS` below for the family that is expensive
/// but must stay advertised.
pub const PACKS: &[ToolPack] = &[
    ToolPack {
        id: "workflows",
        summary: "Saved automation workflows: build, discover, run, inspect runs.",
        tools: &[
            "build_workflow",
            "discover_workflows",
            "run_workflow",
            "await_workflow",
            "describe_workflow",
            "list_workflows",
            "list_workflow_runs",
            "read_workflow_run_log",
            // Flow authoring and inspection. Owned by `workflow_builder` /
            // `flow_discovery`; any other agent that lists them reaches them
            // through `use_skill`.
            "propose_workflow",
            "revise_workflow",
            "edit_workflow",
            "validate_workflow",
            "save_workflow",
            "create_workflow",
            "duplicate_flow",
            "dry_run_workflow",
            "list_flows",
            "get_flow",
            "get_flow_history",
            "get_flow_run",
            "list_flow_runs",
            "list_flow_connections",
            "cancel_flow_run",
            "resume_flow_run",
            "suggest_workflows",
            "search_tool_catalog",
            "get_tool_contract",
            "get_tool_output_sample",
            "list_node_kinds",
            "get_node_kind_contract",
            "list_agent_definitions",
            "list_connectable_toolkits",
        ],
        // The orchestrator is NOT an owner: it builds and discovers through
        // `spawn_async_subagent` (prompt.md) and reaches the rest via `use_skill`,
        // so owning the pack put all of it on its wire for nothing.
        owners: &["workflow_builder", "flow_discovery"],
        guide: "",
    },
    ToolPack {
        id: "web3",
        summary: "Crypto wallet and market actions: quotes, swaps, bridges, contract calls, x402.",
        // `wallet_balances`, `wallet_network_defaults`, `wallet_supported_assets`,
        // `wallet_encode_erc20_transfer` and `wallet_execute_prepared` are NOT
        // listed: they exist as `wallet.*` RPC methods but have no agent Tool
        // wrapper, and `render_pack_filtered` skips an unresolvable name
        // silently — so listing them only made the rendered menu quietly short.
        tools: &[
            "wallet_status",
            "wallet_chain_status",
            "wallet_prepare_transfer",
            "wallet_tx_status",
            "wallet_tx_receipt",
            "wallet_lookup_tx",
            "web3_swap_routes",
            "web3_swap_quote",
            "web3_swap_execute",
            "web3_bridge_quote",
            "web3_bridge_execute",
            "web3_dapp_call",
            "web3_dapp_execute",
            "x402_request",
        ],
        owners: &[],
        guide: include_str!("guides/web3.md"),
    },
    ToolPack {
        id: "mcp",
        // The orchestrator carries a narrow named set of MCP tools directly;
        // keep those schemas visible without exposing every server tool.
        summary: "MCP servers: search the catalog, connect, disconnect, check status, call tools.",
        tools: &[
            "mcp_registry_status",
            "mcp_registry_search",
            "mcp_registry_get",
            "mcp_registry_installed_list",
            "mcp_registry_list_tools",
            "mcp_registry_connect",
            "mcp_registry_disconnect",
            "mcp_registry_tool_call",
            "mcp_registry_uninstall",
        ],
        // The orchestrator owns it so the four registry tools it names are not
        // withheld; its `deferred_tools` then takes them off its wire, so they
        // are found through `tool_search` like the catalogue readers (which are
        // `Deferred` for every agent) and stay callable by name.
        owners: &["orchestrator", "planner"],
        guide: include_str!("guides/mcp.md"),
    },
    ToolPack {
        id: "composio",
        summary: "Composio toolkits: list connections, list and execute actions.",
        // `composio_connect` is deliberately not a member: it is the
        // orchestrator's inline connect card. Packed, it sat in a pack that
        // `ops::closed_by_direct_handoff` closes to the orchestrator (the
        // planner, one `plan` hand-off away, owns this pack), so the prompt's
        // "raise a connect card" route was a tool the model could not reach.
        //
        // `composio_list_toolkits` is unpacked for the same reason, one bug
        // later. It answers "what can I connect?" — the backend allowlist as
        // `{toolkits, catalog:[{slug, name, description, categories}]}` — and
        // the orchestrator owns that conversation. Packed, it was `Deny`ed to
        // the orchestrator by the rule above, and the only escapes were a
        // `use_skill` that answers "no tools available" and a `plan` hand-off
        // whose description ("break a task into a DAG of subtasks") gives a
        // model no reason to associate it with a catalogue lookup. Observed:
        // asked to list Composio apps, the orchestrator tried `use_skill`,
        // then six `tool_search` calls, then scraped docs.composio.dev and
        // reported a marketing figure of "1,552+ apps" instead of this
        // install's real 119.
        //
        // Its own owners keep it by DECLARING it (`workflow_builder`, `planner`
        // and `morning_briefing` agent.toml).
        tools: &[
            "composio",
            "composio_authorize",
            "composio_execute",
            "composio_list_connections",
            "composio_list_tools",
        ],
        owners: &["workflow_builder", "planner", "morning_briefing"],
        guide: "",
    },
    ToolPack {
        id: "skills",
        // The install hand-off (`setup_skills`) is a member like the other
        // packed hand-offs (`build_workflow`, `manage_tasks`): it cost ~270
        // tokens on every orchestrator request for a family used a few times a
        // week. Packed, it no longer closes this pack to the orchestrator
        // (`ops::closed_by_direct_handoff` keys on UNPACKED hand-offs), so the
        // listing offers the hand-off beside the raw registry tools. Running an
        // installed skill is the orchestrator's own `run_workflow`.
        summary: "Skills: install or find agent skills (setup_skills hands the whole request to the installer), browse registries, read resources.",
        tools: &[
            "setup_skills",
            // In the pack, not outside it. A search tool advertised while the
            // tool it hands off to (`describe_workflow`) stays
            // withheld would cost 748 B on every wildcard agent to produce an id
            // the agent then cannot act on without a `load_skill` anyway. One
            // recovery step for the whole family beats a doorway to a locked
            // room. The orchestrator prompt names it for the same reason it
            // already names `describe_workflow` and `skill_registry_browse` —
            // those are packed too.
            "skill_search",
            "skill_registry_browse",
            "skill_registry_search",
            "skill_registry_install",
            "skill_registry_sources",
            "skill_registry_uninstall",
            "skill_runtime_resolve_runtimes",
            "install_workflow_from_url",
            "uninstall_workflow",
            "read_workflow_resource",
        ],
        owners: &[
            "skill_setup",
            // `workflow_builder` owns exactly ONE tool from this pack —
            // `read_workflow_resource`, which fetches a page of the
            // `flow-authoring` builtin skill, the reference manual its own
            // system prompt points it at. Ownership here advertises nothing
            // else: its belt is `ToolScope::Named`, so `visible` holds only the
            // 30 tools its `agent.toml` lists, and the other ten members of
            // this pack are not among them. Without it the manual costs a
            // `load_skill` round trip before every read, and the recovery pair
            // (`load_skill` + `use_skill`, 3,137 B) lands on the belt in place
            // of the tool's own ~500 B — measured, not estimated.
            "workflow_builder",
        ],
        guide: "",
    },
    ToolPack {
        id: "documents",
        summary: "Build a slide deck or a .docx/.pptx document as a workspace artifact.",
        tools: &[
            "generate_document",
            "generate_presentation",
            // The delegate, not just the leaf tools: an orchestrator that can
            // see `make_presentation` pays its schema on every turn to route a
            // request that arrives in a small minority of them, and the pack
            // it would route into is already withheld.
            "make_presentation",
        ],
        owners: &["presentation_agent"],
        guide: "",
    },
    ToolPack {
        id: "audio",
        summary: "Generate a spoken podcast from text and optionally email the audio.",
        tools: &[
            "audio_generate_podcast",
            "audio_email_podcast",
            "audio_generate_and_email_podcast",
        ],
        owners: &[],
        guide: "",
    },
    ToolPack {
        id: "system",
        summary: "OpenHuman settings: config, health, diagnostics, costs, service, proxy, credentials, app updates.",
        tools: &[
            "config_snapshot",
            "config_get_client_config",
            "config_get_autonomy",
            "config_get_search",
            "config_get_runtime_flags",
            "config_resolve_api_url",
            "config_get_data_paths",
            "doctor_health",
            "doctor_models",
            "health_snapshot",
            "health_system_info",
            "dashboard_model_health",
            "cost_get_dashboard",
            "cost_get_daily_history",
            "cost_get_summary",
            "security_policy_info",
            "service_status",
            "service_start",
            "service_stop",
            "service_restart",
            "service_shutdown",
            "service_install",
            "service_uninstall",
            "daemon_host_prefs_get",
            "daemon_host_prefs_set",
            "proxy_config",
            "session_state",
            "credential_list",
            "oauth_connect_url",
            "oauth_list",
            "update_check",
            "update_apply",
        ],
        owners: &[],
        guide: include_str!("guides/system.md"),
    },
    ToolPack {
        id: "coding",
        summary: "Code and repositories: search, edit, run scripts, test, lint, review a diff, git.",
        // `shell` covers every one of these for an agent that has it, so on a
        // belt that also carries `shell` the family is duplicate surface
        // charged on every turn. It stays one `use_skill` away, and the
        // specialists below keep it advertised because inspecting files IS
        // their loop rather than an occasional step inside it.
        //
        // `apply_patch` is deliberately NOT here. Editing an existing file
        // through a shell heredoc is the failure mode the patch tool exists to
        // prevent, so it is not duplicate surface in the way a `cat` is.
        //
        // `file_write` left for the same reason, and a measured one. It is the
        // only tool on any belt that can CREATE a file: `apply_patch` and `edit`
        // both canonicalize an existing target, and `shell` means a heredoc.
        // Packed, it was not one `use_skill` away either — every owner below is
        // in the orchestrator's `[subagents]` allowlist, so
        // `ops::closed_by_direct_handoff` DENIED the whole pack to the agent
        // whose own `agent.toml` says "`file_write` creates new files". The
        // life-scenario benchmark caught the result: four of six tasks produced
        // no file at all, and `meal-plan` burned eleven rounds discovering it
        // had no writer. One ~300 B schema per turn is the right price for the
        // single most common assistant task.
        //
        // `file_read` left too, because the harness itself tells the model to
        // call it. Every oversized tool result is replaced by a
        // `[tool_result_preview]` whose `read_with:` line is
        // `file_read {"path": …}` (`agent/harness/tool_result_artifacts`), and
        // the orchestrator is the agent that receives most of those previews
        // (Gmail listings, catalogue dumps, search results). Packed, the same
        // rule DENIED it, and the bare-name router (`packed_tool_route`) does
        // not route a denied tool, so the call answered `unknown tool`. Observed
        // on v0.64.0: asked to list ten emails, the orchestrator followed the
        // preview, got `unknown tool file_read`, then invented `ranges` and
        // `tool_read_file`, misused `desktop_continue_goal` and `plan`, and ran
        // out of iterations without reading its own result.
        //
        // The rest of the family is `Deferred` rather than on any belt:
        // `tool_search` finds one of them, and this skill hands out the whole
        // loop with its playbook. It replaced the `code_executor` / `critic` /
        // `tool_maker` specialists, whose value was that playbook.
        // `lsp` is not listed: it registers only behind its capability gate,
        // and every pack member must resolve in a default build. It is
        // `Deferred` too, so `tool_search` still finds it when enabled.
        tools: &[
            "grep",
            "glob",
            "list",
            "git_operations",
            "edit",
            "curl",
            "read_diff",
            "run_linter",
            "run_tests",
        ],
        // `planner` and `critic` are workflow-run workers, not chat
        // delegates; inspecting files is their loop, so they keep the family.
        owners: &["planner", "critic", "image_agent", "video_agent", "vision_agent"],
        guide: include_str!("guides/coding.md"),
    },
    ToolPack {
        id: "storage",
        summary: "Workspace file storage: upload, download, list, shareable link.",
        tools: &[
            "storage_upload_file",
            "storage_download_file",
            "storage_list_files",
            "storage_get_link",
        ],
        owners: &[],
        guide: "",
    },
    ToolPack {
        id: "scheduling",
        summary: "Reminders and scheduled jobs: create, list, update, remove, run, inspect.",
        tools: &["cron"],
        owners: &[],
        guide: include_str!("guides/scheduling.md"),
    },
    ToolPack {
        id: "media",
        summary: "Images and clips: generate, or read (describe, OCR, charts, UI elements).",
        tools: &[
            "create_image",
            "create_video",
            // Reading an image, not making one, but it is the same belt from
            // the model's point of view: the request that reaches for it names
            // a picture either way.
            "analyze_image",
            "media_generate_image",
            "media_generate_video",
            "media_list_models",
        ],
        owners: &["image_agent", "video_agent", "vision_agent"],
        guide: "",
    },
    ToolPack {
        id: "tasks",
        summary: "Task sources, workflows, artifacts: add, preview, fetch, update, remove.",
        tools: &["manage_tasks"],
        owners: &["task_manager_agent"],
        guide: "",
    },
    ToolPack {
        id: "goals",
        // Everything about goals EXCEPT closing one.
        //
        // `goal_get` / `goal_set` are the agent-owned completion contract for
        // one conversation thread.
        //
        // `goal_complete` is deliberately NOT a member. Closing a goal is the
        // one goal operation an agent reaches for reactively, at the end of
        // work it has just finished, and a `use_skill` round trip at that
        // moment buys nothing: the alternative to a visible `goal_complete` is
        // an objective that silently stays open and keeps driving autonomous
        // continuation. Same reasoning as `DELIBERATELY_UNPACKED_FLEET_TOOLS`.
        summary: "This thread's objective: read and set it.",
        tools: &["goal_get", "goal_set"],
        owners: &[],
        guide: "",
    },
    ToolPack {
        id: "docs",
        summary: "OpenHuman's own product docs: how a feature works, setup steps, where a setting lives.",
        tools: &["gitbooks_search", "gitbooks_get_page"],
        owners: &[],
        guide: include_str!("guides/docs.md"),
    },
];

/// The fleet tools are deliberately NOT a pack, and this is worth stating
/// because they look like an obvious 1.6k-token candidate.
///
/// `steer_subagent`, `wait_subagent`, `close_subagent`, `list_subagents`,
/// `continue_subagent`, `wait`, `wait_loop` and `spawn_parallel_agents` are
/// needed *reactively*, mid-turn — exactly when an async worker returns or
/// pauses on `ask_user_clarification`. A load round-trip at that moment is the
/// worst possible time to add one, and a `continue_subagent` the model cannot
/// see is the known infinite-re-delegation failure mode (#4291): the only
/// continuation left is a fresh stateless sub-agent that asks the same
/// question again.
#[cfg(test)]
pub(crate) const DELIBERATELY_UNPACKED_FLEET_TOOLS: &[&str] = &[
    "steer_subagent",
    "wait_subagent",
    "close_subagent",
    "list_subagents",
    "continue_subagent",
    "wait",
    "wait_loop",
    "spawn_parallel_agents",
];

/// The lookup surface over [`PACKS`] handed to the generic `use_skill` tool
/// (`tinyagents_harness::tool::packs`). `NOT_FOUND_MARKER` is the host's status
/// vocabulary, so a "no such skill / tool" result classifies as `NotFound`.
pub const CATALOG: PackCatalog = PackCatalog::new(PACKS, crate::tools::status::NOT_FOUND_MARKER);

pub fn pack(id: &str) -> Option<&'static ToolPack> {
    CATALOG.pack(id)
}

/// The pack owning `tool`, if any.
pub fn pack_for_tool(tool: &str) -> Option<&'static ToolPack> {
    CATALOG.pack_for_tool(tool)
}

/// Every packed tool name across all packs.
pub fn all_packed_tool_names() -> Vec<&'static str> {
    CATALOG.all_packed_tool_names()
}

/// Every packed tool name that applies to `agent_id`.
///
/// A pack is skipped entirely for agents listed as its owners — see
/// [`ToolPack::owners`]. The orchestrator owns the MCP integrations pack so
/// its small named MCP tool set remains directly callable.
pub fn packed_tool_names_for_agent(agent_id: &str) -> Vec<&'static str> {
    CATALOG.packed_tool_names_for_agent(agent_id)
}

/// The always-on index: one line per pack, rendered into `use_skill`'s own
/// description so the model can pick a pack without a round trip.
pub fn pack_index_markdown() -> String {
    CATALOG.pack_index_markdown()
}

/// The pack index, limited to packs this session can call at least one tool in.
pub fn pack_index_markdown_filtered(is_callable: &dyn Fn(&str) -> bool) -> String {
    CATALOG.pack_index_markdown_filtered(is_callable)
}

/// Pack ids with at least one tool this session can call — the `skill` enum
/// `use_skill` should actually offer.
pub fn callable_pack_ids(is_callable: &dyn Fn(&str) -> bool) -> Vec<&'static str> {
    CATALOG.callable_pack_ids(is_callable)
}
