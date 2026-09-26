//! The compiled-in pack table.
//!
//! Membership is a build-time decision, deliberately: a pack that config or RPC
//! could edit would let a caller move a dangerous tool out of the advertised
//! surface (or back into it) without review. Adding a pack is a source change.

use super::types::ToolPack;

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
        owners: &["workflow_builder", "flow_discovery"],
    },
    ToolPack {
        id: "crypto",
        summary: "Crypto wallet and market actions: quotes, swaps, bridges, contract calls, x402.",
        // `wallet_balances`, `wallet_network_defaults`, `wallet_supported_assets`,
        // `wallet_encode_erc20_transfer` and `wallet_execute_prepared` are NOT
        // listed: they exist as `wallet.*` RPC methods but have no agent Tool
        // wrapper, and `render_pack_filtered` skips an unresolvable name
        // silently — so listing them only made the rendered menu quietly short.
        tools: &[
            "do_crypto",
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
        owners: &["crypto_agent"],
    },
    ToolPack {
        id: "integrations",
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
        owners: &["mcp_agent", "planner", "orchestrator"],
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
        // Its own owners keep it by DECLARING it: `planner/agent.toml:72` and
        // `workflow_builder/agent.toml:109`. `integrations_agent` reached it
        // only through this pack, so it now declares it too — unpacking must
        // not quietly take a capability from a second consumer.
        tools: &[
            "composio",
            "composio_authorize",
            "composio_execute",
            "composio_list_connections",
            "composio_list_tools",
        ],
        owners: &["integrations_agent", "workflow_builder", "planner"],
    },
    ToolPack {
        id: "skills",
        // The install and run hand-offs (`setup_skills`, `run_skill`) are not
        // members: they are the orchestrator's direct route into this family.
        // See `DELIBERATELY_UNPACKED_HANDOFFS`.
        summary: "Skills: search installed, browse and install from registries, read resources.",
        tools: &[
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
            // The delegate into `skill_creator`, which owns this pack.
            "create_skill",
        ],
        owners: &[
            "skill_setup",
            "skill_executor",
            "skill_creator",
            "context_scout",
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
    },
    ToolPack {
        id: "system",
        summary: "OpenHuman health, diagnostics, costs, services, proxy, read-only config.",
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
            // The delegate into this same family. `settings_agent` owns the
            // pack, so it keeps seeing the whole belt including this.
            "manage_settings",
        ],
        owners: &["settings_agent"],
    },
    ToolPack {
        id: "files",
        summary: "Files and repositories: grep, glob, list, git.",
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
        tools: &["grep", "glob", "list", "git_operations"],
        owners: &[
            "code_executor",
            "critic",
            "planner",
            "skill_creator",
            "skill_executor",
            "tool_maker",
            "image_agent",
            "video_agent",
            "vision_agent",
            "integrations_agent",
        ],
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
        // Not ownerless: `code_executor` and `integrations_agent` both declare
        // the family on their own belts, and an agent that uploads its own
        // artifacts should not pay a `use_skill` round trip to hand one back.
        owners: &["code_executor", "integrations_agent"],
    },
    ToolPack {
        id: "scheduling",
        summary: "Reminders and scheduled jobs: create, list, update, remove, run, inspect.",
        tools: &[
            "schedule_task",
            "cron_add",
            "cron_list",
            "cron_update",
            "cron_remove",
            "cron_run",
            "cron_runs",
        ],
        owners: &["scheduler_agent"],
    },
    ToolPack {
        id: "profile",
        summary: "The user's profile: record preferences, edit persona and people graph.",
        // The delegate and the two raw tools belong together because they are
        // one question from the model's side — "remember this about the user" —
        // split only by how much editing it needs.
        tools: &[
            "save_preference",
            "remember_preference",
            "manage_profile_memory",
        ],
        owners: &["profile_memory_agent"],
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
    },
    ToolPack {
        id: "tasks",
        summary: "Task sources, workflows, artifacts: add, preview, fetch, update, remove.",
        tools: &["manage_tasks"],
        owners: &["task_manager_agent"],
    },
    ToolPack {
        id: "goals",
        // Everything about goals EXCEPT closing one.
        //
        // Two different surfaces live here, and the pack is the seam that lets
        // the model find either: `goals` is the user's durable long-term
        // objectives held in memory, while `goal_get` / `goal_set` are the
        // agent-owned completion contract for one conversation thread and are
        // assigned to `goals_agent`. Long-term goals remain user-reachable
        // through `memory_goals.*`.
        //
        // `goal_complete` is deliberately NOT a member. Closing a goal is the
        // one goal operation an agent reaches for reactively, at the end of
        // work it has just finished, and a `use_skill` round trip at that
        // moment buys nothing: the alternative to a visible `goal_complete` is
        // an objective that silently stays open and keeps driving autonomous
        // continuation. Same reasoning as `DELIBERATELY_UNPACKED_FLEET_TOOLS`.
        summary: "Long-term goals and this thread's objective: read, add, edit.",
        tools: &["goals", "goal_get", "goal_set"],
        owners: &["goals_agent"],
    },
    ToolPack {
        id: "app_update",
        summary: "Check for and apply OpenHuman application updates.",
        tools: &["update_check", "update_apply"],
        owners: &["settings_agent"],
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

/// The skill hand-offs are deliberately not packed (#6302).
///
/// `setup_skills` and `run_skill` are the orchestrator's route into the
/// skills family. Packed, they sat in the same listing as the raw
/// `skill_registry_*` tools, one `use_skill` round trip
/// away, and a live account showed the cost: across 11 turns the orchestrator
/// called the raw tools itself, guessed at tool names, and never handed off.
/// Handing off is the most common thing it does with these families, so the
/// `collapsed_delegation.rs` argument applies: frequency of use decides, and
/// delegation should not pay a round trip.
///
/// With a hand-off on the belt, `ops::closed_by_direct_handoff` closes the
/// owning pack's raw tools to the caller, so the hand-off is its only route.
/// The other packed hand-offs (`do_crypto`, `build_workflow`,
/// `discover_workflows`, `make_presentation`, ...) stay packed: each is its own
/// token-cost decision, and the same closing rule takes effect for any of them
/// as soon as it is unpacked and listed here.
#[cfg(test)]
pub(crate) const DELIBERATELY_UNPACKED_HANDOFFS: &[&str] = &["setup_skills", "run_skill"];

pub fn pack(id: &str) -> Option<&'static ToolPack> {
    PACKS.iter().find(|p| p.id == id)
}

/// The pack owning `tool`, if any.
pub fn pack_for_tool(tool: &str) -> Option<&'static ToolPack> {
    PACKS.iter().find(|p| p.owns(tool))
}

/// Every packed tool name across all packs.
pub fn all_packed_tool_names() -> Vec<&'static str> {
    PACKS.iter().flat_map(|p| p.tools.iter().copied()).collect()
}

/// Every packed tool name that applies to `agent_id`.
///
/// A pack is skipped entirely for agents listed as its owners — see
/// [`ToolPack::owners`]. The orchestrator owns the MCP integrations pack so
/// its small named MCP tool set remains directly callable.
pub fn packed_tool_names_for_agent(agent_id: &str) -> Vec<&'static str> {
    PACKS
        .iter()
        .filter(|p| !p.is_owner(agent_id))
        .flat_map(|p| p.tools.iter().copied())
        .collect()
}

/// The always-on index: one line per pack, rendered into `use_skill`'s own
/// description so the model can pick a pack without a round trip.
pub fn pack_index_markdown() -> String {
    pack_index_markdown_filtered(&|_| true)
}

/// The pack index, limited to packs this session can call at least one tool in.
///
/// A pack with nothing callable is not an answer to "which skills can I load",
/// and advertising it costs a round trip: the model loads it, learns it cannot
/// use it, and comes back. The capability does not disappear — a pack's owners
/// reach the model through their own `delegate_*` tools, whose `when_to_use`
/// descriptions are already on the wire and are what the model should call
/// anyway. Keeping the pack listed here would duplicate that routing on every
/// single turn.
pub fn pack_index_markdown_filtered(is_callable: &dyn Fn(&str) -> bool) -> String {
    let mut out = String::new();
    for p in PACKS {
        if !p.tools.iter().any(|t| is_callable(t)) {
            continue;
        }
        out.push_str(&format!("- `{}` — {}\n", p.id, p.summary));
    }
    out
}

/// Pack ids with at least one tool this session can call — the `skill` enum
/// `load_skill` should actually offer.
pub fn callable_pack_ids(is_callable: &dyn Fn(&str) -> bool) -> Vec<&'static str> {
    PACKS
        .iter()
        .filter(|p| p.tools.iter().any(|t| is_callable(t)))
        .map(|p| p.id)
        .collect()
}
