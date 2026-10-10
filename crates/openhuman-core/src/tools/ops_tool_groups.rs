//! Domain-family classification used to filter the callable tool catalog.

/// Classify an agent tool into its [`DomainGroup`](crate::core::all::DomainGroup)
/// by its `name()`, so [`all_tools_with_runtime`](super::all_tools_with_runtime) can drop tools whose family is
/// disabled under the ambient [`DomainSet`](crate::core::runtime::DomainSet).
///
/// Named-family tools are matched here; everything without a domain family
/// defaults to `Platform`. Under `harness()`, the Agent/Memory/Threads/Config/
/// Security tools remain while gate-family and generic Platform tools drop.
/// (Names verified against each Tool impl's `fn name()` on 2026-07-13.)
pub(crate) fn tool_group(name: &str) -> crate::core::all::DomainGroup {
    use crate::core::all::DomainGroup;

    // Gate families with a domain-exclusive name prefix are matched by prefix
    // (not an exact list) so a NEW tool in the family auto-gates instead of
    // silently defaulting to Platform and leaking under a custom DomainSet
    // (#4808 maintainer review). Web3 = wallet_/web3_/x402_, Media = media_,
    // Mcp = mcp_ (below). Families without a clean prefix (Skills/Flows) keep
    // their exact lists; `no_gate_family_tool_silently_defaults_to_platform`
    // guards the prefix families.
    const SKILLS: &[&str] = &[
        "run_workflow",
        "await_workflow",
        "list_workflows",
        "create_skill",
        "describe_workflow",
        "read_workflow_resource",
        "list_workflow_runs",
        "read_workflow_run_log",
        "install_workflow_from_url",
        "uninstall_workflow",
        "skill_registry_browse",
        "skill_registry_search",
        "skill_registry_install",
        "skill_registry_sources",
        "skill_registry_uninstall",
        "skill_runtime_resolve_runtimes",
    ];
    // Flows has no clean tool-name prefix, so it MUST list every flow-owned
    // tool explicitly — a missing name falls through to `Platform` below and
    // stays callable under a custom `DomainSet { platform: true, flows: false }`,
    // leaking the flows surface past the runtime gate (#4808 review; #4797
    // maintainer review). Keep this in lockstep with the `#[cfg(feature =
    // "flows")]` registrations in `all_tools_with_runtime` above — the same 28
    // names asserted by `default_tools_omits_flows_tools_when_feature_off`.
    const FLOWS: &[&str] = &[
        "propose_workflow",
        "revise_workflow",
        "edit_workflow",
        "validate_workflow",
        "get_flow_history",
        "dry_run_workflow",
        "save_workflow",
        "suggest_workflows",
        "run_flow",
        "list_flow_runs",
        "resume_flow_run",
        "cancel_flow_run",
        "create_workflow",
        "duplicate_flow",
        "list_flows",
        "get_flow",
        "get_flow_run",
        "list_flow_connections",
        "search_tool_catalog",
        "get_tool_contract",
        "get_tool_output_sample",
        "list_agent_definitions",
        "list_connectable_toolkits",
        "list_node_kinds",
        "get_node_kind_contract",
        // Per-flow sandboxed memory (issue #5173) — `flow_` prefixed, not
        // `memory_`, so it does NOT fall under the `memory_` prefix check
        // below and must be listed here explicitly like every other
        // flow-owned tool.
        "flow_memory_recall",
        "flow_memory_remember",
    ];
    // Voice family agent tools (audio_toolkit) — no `voice_`/`tts_`/`stt_`
    // prefix, so they must be listed explicitly or they fall through to
    // Platform and stay callable when Voice is gated off (#4808 review).
    const VOICE: &[&str] = &[
        "audio_generate_podcast",
        "audio_email_podcast",
        "audio_generate_and_email_podcast",
    ];
    // Threads: thread_* / todo_* handled by prefix below; these are the extras.
    // Monitor + proactive-notify tools (Automation family).
    const MONITORS: &[&str] = &[
        "monitor",
        "monitor_list",
        "monitor_read",
        "monitor_stop",
        "notify_user",
    ];
    const THREADS_EXTRA: &[&str] = &["goal_get", "goal_set", "goal_complete"];

    // MCP: every MCP tool name is `mcp_` prefixed (mcp_registry_*,
    // mcp_call_tool, mcp_list_servers, mcp_list_tools).
    if name.starts_with("mcp_") {
        return DomainGroup::Mcp;
    }
    // Web3: wallet_/web3_/x402_ are all Web3-exclusive prefixes.
    if name.starts_with("wallet_") || name.starts_with("web3_") || name.starts_with("x402_") {
        return DomainGroup::Web3;
    }
    if SKILLS.contains(&name) {
        return DomainGroup::Skills;
    }
    if FLOWS.contains(&name) {
        return DomainGroup::Flows;
    }
    // Media generation: `media_` prefix (media_generate_image/video, media_list_models).
    if name.starts_with("media_") {
        return DomainGroup::Media;
    }
    // Voice family: explicit audio_* podcast tools plus the defensive
    // voice_/tts_/stt_ prefixes for any future tool.
    if VOICE.contains(&name)
        || name.starts_with("voice_")
        || name.starts_with("tts_")
        || name.starts_with("stt_")
    {
        return DomainGroup::Voice;
    }
    // Memory family (harness-kept): the single `memory` tool.
    if name == crate::memory::MEMORY_TOOL_NAME {
        return DomainGroup::Memory;
    }
    // Threads family (harness-kept): thread_* + per-thread goal + search.
    // `thread_` is kept as a prefix even though the `thread_*` agent-tool
    // family was removed: `goal_*` and the THREADS_EXTRA entries still
    // classify here, and a future threads tool should land in Threads rather
    // than falling through to Platform.
    if name.starts_with("thread_") || THREADS_EXTRA.contains(&name) {
        return DomainGroup::Threads;
    }
    // Harness families realigned out of Platform.
    if name.starts_with("artifact_")
        || name.starts_with("learning_")
        || name.contains("subagent")
        || matches!(
            name,
            "ask_user_clarification"
                | "delegate"
                | "delegate_graph"
                | "delegate_to_personality"
                | "todo"
                | "wait"
                | "wait_loop"
                | "request_plan_review"
                | "plan_exit"
                | "spawn_parallel_agents"
        )
    {
        return DomainGroup::Agent;
    }
    if name.starts_with("config_") || name.starts_with("workspace_") {
        return DomainGroup::Config;
    }
    if name.starts_with("security_")
        || name.starts_with("credential_")
        || name.starts_with("session_")
        || name.starts_with("oauth_")
    {
        return DomainGroup::Security;
    }
    // ── Families carved out of Platform by the DomainGroup realignment ──────
    // Each of these previously fell through to Platform, which meant the tool
    // stayed callable when its family was gated off under a custom DomainSet —
    // leak the #4808 review flagged. Keep these in
    // lockstep with the `push(...)` tags in `core::all`.
    //
    // Automation: scheduled jobs (`cron_*`) plus the monitor +
    // proactive-notify surface.
    if name.starts_with("cron_") || name == "schedule" || MONITORS.contains(&name) {
        return DomainGroup::Automation;
    }
    // Integrations: every external connector reached on the user's behalf.
    if name.starts_with("composio")
        || name == "web_search_tool"
        || name == "web_answer_tool"
        || name == "web_contents_tool"
        || name == "search"
        || name.starts_with("tinyfish_")
        || name.starts_with("exa_")
        || name.starts_with("gemini_")
        || name.starts_with("parallel_")
        || name.starts_with("brave_")
        || name.starts_with("querit_")
        || name.starts_with("tavily_")
        || name.starts_with("seltz_")
        || name.starts_with("searxng_")
        || name.starts_with("google_places_")
        || name.starts_with("stock_")
        || name.starts_with("storage_")
        || name.starts_with("task_source_")
        // Hosting: `hosting_` is a domain-exclusive prefix, so a NEW hosting
        // tool auto-gates rather than falling through to Platform and staying
        // callable under a custom DomainSet. `hosting_launch_site` uploads a
        // workspace directory to a third party and can provision a paid
        // database, so it must not outlive its family's gate.
        || name.starts_with("hosting_")
    {
        return DomainGroup::Integrations;
    }
    // Hosted: clients of the TinyHumans backend. The `billing_` / `team_` /
    // `referral_` prefixes were removed with those agent-tool families; their
    // controllers stay registered for the dashboard, which does not route
    // through this classifier.
    if name.starts_with("orchestration_") {
        return DomainGroup::Hosted;
    }
    // Desktop: shell-facing surfaces.
    if name.starts_with("dashboard_") || name.starts_with("desktop_") {
        return DomainGroup::Desktop;
    }
    // Inference: the CCR retrieval surface. Matched against the crate's own
    // constant list rather than a name prefix — the live tool is
    // `juice_retrieve`, and `tokenjuice_retrieve` / `retrieve_tool_output`
    // are migration aliases, so a prefix rule silently missed the real one.
    if crate::inference::tokenjuice::RECOVERY_TOOL_NAMES.contains(&name)
        || crate::inference::tokenjuice::is_repl_tool(name)
    {
        return DomainGroup::Inference;
    }
    // Everything else — shell/file and other kernel utilities — is Platform:
    // present under full(), absent under harness()/none().
    DomainGroup::Platform
}
