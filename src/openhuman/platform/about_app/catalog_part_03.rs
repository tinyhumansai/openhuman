use super::*;

pub(super) const CAPABILITIES: &[Capability] = &[
Capability {
        id: "settings.developer_options",
        name: "Developer Options",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Open developer-focused panels for diagnostics, workflows, AI config, and memory tools.",
        how_to: "Settings > Developer Options",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "settings.debug_webhooks",
        name: "Debug Webhooks",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description:
            "Inspect Composio trigger history and find the daily JSONL archive files stored by the app.",
        how_to: "Settings > Developer Options > Webhooks",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "settings.manage_service",
        name: "Manage Desktop Service",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Install, start, stop, restart, uninstall, or inspect the optional desktop background service.",
        how_to: "Settings > Developer Options > Tauri Commands",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "settings.clear_app_data",
        name: "Log Out and Clear App Data",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Sign out and permanently clear local app data, including workflow data.",
        how_to: "Settings > Log Out & Clear App Data",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "settings.delete_all_data",
        name: "Delete All Data",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Delete all local data and reset the app from the destructive settings section.",
        how_to: "Settings > Delete All Data",
        status: CapabilityStatus::ComingSoon,
        privacy: None,
    },
    Capability {
        id: "automation.task_sources",
        name: "Task Sources",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Pull work items from GitHub, Notion, Linear, and ClickUp using per-source \
                      filters, then enrich them onto the agent's todo board and (for proactive \
                      sources) start an agent working on them.",
        how_to: "Settings > Task Sources",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "automation.discover_workflows",
        name: "Suggested Workflows (Flow Scout)",
        domain: "flows",
        category: CapabilityCategory::Automation,
        description: "A read-only discovery agent (\"Flow Scout\") reads your memory, past \
                      conversations, known people, connected apps, and existing flows to figure \
                      out which automations would actually help you, then proposes a handful of \
                      concrete, buildable workflow suggestions. Each card explains why it was \
                      suggested; \"Build this\" hands it to the workflow builder to author a real \
                      flow you review and save. Discovery never creates, enables, or runs a flow.",
        how_to: "Flows > Suggested for you > Discover",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "automation.flow_memory_node",
        name: "Memory Node (Flows)",
        domain: "flows",
        category: CapabilityCategory::Automation,
        description: "A `memory` node inside a saved workflow graph, giving the flow direct, \
                      in-graph memory access with no agent turn involved. It can recall/search/ \
                      read style-flavour/look up people from your durable, cross-flow memory \
                      (read-only — a flow can never write there) or from other flows' own \
                      memory (also read-only), and can remember/forget entries in its OWN \
                      private, flow-scoped memory namespace — never the user's personal memory, \
                      never another flow's. Every operation is gated by the flow's autonomy \
                      tier; a flow-scoped write can require human approval.",
        how_to: "Flows editor > add a `memory` node; set `config.operation` and `config.scope`.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "automation.flow_dedup_node",
        name: "Dedup Node (Flows)",
        domain: "flows",
        category: CapabilityCategory::Automation,
        description: "A `dedup` node inside a saved workflow graph, giving the flow durable \
                      exactly-once processing per item with no agent turn or extra plumbing \
                      involved. It drops an item whose per-item key was already committed by a \
                      prior successful run, and otherwise passes it through. Committing happens \
                      automatically: keys the node passes through are marked done only once the \
                      whole run finishes successfully; a failed/cancelled/interrupted/unknown (or \
                      any other non-success) run leaves them unmarked so the same items retry next \
                      time. Only the resolved per-item key value is stored, locally, in the flow's \
                      own private, flow-scoped state — never the item's full content, and never \
                      the user's personal memory. The key is whatever the workflow author's \
                      `config.key` expression resolves to, so it can carry item-derived data if \
                      keyed off a sensitive field — author flows to key off an opaque, \
                      non-sensitive stable id (an issue number, message id, url) rather than \
                      personal data.",
        how_to: "Flows editor > add a `dedup` node right after the item source; set config.key \
                 to a stable per-item id expression, e.g. \"=item.id\".",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "automation.view_cron_jobs",
        name: "View Cron Jobs",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Review scheduled jobs available to the runtime.",
        how_to: "Settings > Cron Jobs",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "automation.set_job_intervals",
        name: "Set Job Intervals",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Configure how often a scheduled job should run.",
        how_to: "Settings > Cron Jobs",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "automation.view_execution_history",
        name: "View Execution History",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Inspect past runs and results for scheduled jobs.",
        how_to: "Settings > Cron Jobs",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    // ── Proactive agents ─────────────────────────────────────────────────────
    Capability {
        id: "automation.morning_briefing",
        name: "Morning Briefing",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Daily proactive agent that reviews calendar, tasks, emails, and market context to deliver a morning summary.",
        how_to: "Automatic after onboarding (runs daily at 7 AM). Adjust schedule via Settings > Cron Jobs.",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "automation.crypto_agent",
        name: "Crypto Agent",
        domain: "automation",
        category: CapabilityCategory::Automation,
        description: "Dedicated wallet & market specialist sub-agent. The orchestrator \
                      routes transfers, swaps, contract calls, balance lookups, and \
                      exchange trading requests here. The agent enforces a read → \
                      simulate → confirm → execute flow, refuses to fabricate chain ids \
                      or token addresses, and gates every write call behind explicit \
                      user confirmation.",
        how_to: "Automatic — invoked by the orchestrator when a crypto wallet or market action is requested. Connect a wallet via Settings > Recovery Phrase first.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    // ── Update ──────────────────────────────────────────────────────────────
    // ── Mobile (iOS client) ─────────────────────────────────────────────────
    Capability {
        id: "mobile.device_pairing",
        name: "Device Pairing",
        domain: "devices",
        category: CapabilityCategory::Mobile,
        description: "Pair iOS phones with the desktop core via QR code. The desktop generates a \
                      short-lived pairing token; the iOS app scans the QR, completes an X25519 \
                      key agreement, and stores the session for reconnects.",
        how_to: "Settings > Devices > Pair iPhone",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "mobile.ios_client",
        name: "iOS Client",
        domain: "devices",
        category: CapabilityCategory::Mobile,
        description: "iOS app for chatting with your assistant on the go. Connects to the desktop \
                      core via LAN HTTP, an E2E-encrypted socket.io tunnel, or a cloud HTTP \
                      fallback — no Rust core ships on the device.",
        how_to: "Pair via Settings > Devices, then open the OpenHuman iOS app.",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "mobile.push_to_talk",
        name: "Push-to-Talk",
        domain: "devices",
        category: CapabilityCategory::Mobile,
        description: "Hold-to-talk voice input on iOS. Activates AVAudioEngine and \
                      SFSpeechRecognizer on the device; partial transcripts appear while \
                      speaking and the final transcript is sent as a chat message.",
        how_to: "Hold the microphone button on the iOS mascot screen.",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: false,
            data_kind: PrivacyDataKind::Raw,
            destinations: &[],
        }),
    },
    // ── Update ──────────────────────────────────────────────────────────────
    Capability {
        id: "update.check",
        name: "Check for Core Updates",
        domain: "update",
        category: CapabilityCategory::Settings,
        description: "Query GitHub Releases to see if a newer core binary is available. \
                      Available to the orchestrator agent as the `update_check` tool so the \
                      user can ask 'am I up to date?' in chat.",
        how_to: "Settings > Developer Options > Check for Updates, or ask the orchestrator in chat.",
        status: CapabilityStatus::Beta,
        privacy: GITHUB_RELEASES_METADATA,
    },
    Capability {
        id: "update.apply",
        name: "Apply Core Update",
        domain: "update",
        category: CapabilityCategory::Settings,
        description: "Download and stage a newer core binary. Desktop builds can self-restart; \
                      headless deployments can hand restart off to a supervisor. Exposed to \
                      the orchestrator agent as the `update_apply` tool, gated behind explicit \
                      user consent (the agent must confirm via `ask_user_clarification` before \
                      invoking) and the `config.update.rpc_mutations_enabled` policy switch.",
        how_to: "Settings > Developer Options > Apply Update, or confirm an in-chat update prompt from the orchestrator.",
        status: CapabilityStatus::Beta,
        privacy: GITHUB_RELEASES_METADATA,
    },
    Capability {
        id: "filesystem.access_mode",
        name: "Agent OS Access Mode",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Choose how much filesystem and shell access the agent has: Read-Only, \
                      Workspace, Trusted Roots (grant specific folders outside the workspace), \
                      or Full Access. Credential stores stay blocked in every mode.",
        how_to: "Settings → Agent OS access",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "agent.action_timeout",
        name: "Action Timeout",
        domain: "agent",
        category: CapabilityCategory::Settings,
        description: "Set how long a single tool or action may run before it is cancelled \
                      (1–3600 seconds, default 120). Increase it when a large local model is \
                      interrupted before finishing its response. Applies to the next tool call \
                      without a restart; the OPENHUMAN_TOOL_TIMEOUT_SECS env var still overrides it.",
        how_to: "Settings → Agent OS access → Action timeout",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "security.always_allow_tool",
        name: "Always Allow a Tool",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "On an approval prompt, choose \"Always allow\" to stop being asked for that \
                      tool. The choice is saved to your allow-list and persists across restarts; \
                      remove it any time under Settings → Agent OS access to be prompted again. \
                      Policy still blocks forbidden paths and high-risk commands regardless.",
        how_to: "Click \"Always allow\" on an approval prompt; manage the list in Settings → Agent OS access.",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "security.approval_history",
        name: "Approval History",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Review a read-only audit trail of past tool-approval decisions \
                      (Approve once / Always allow / Deny), newest first. Summaries are \
                      scrubbed of chat content and arguments are shown as redacted shape only.",
        how_to: "Settings → Agent OS access → View approval history",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "tool.detect_tools",
        name: "Detect Installed Tools",
        domain: "tools",
        category: CapabilityCategory::Settings,
        description: "Probe the host PATH to report which developer tools and language \
                      runtimes are installed (node, python, cargo, docker, git, …).",
        how_to: "Used by the agent automatically; gated by the tool toggle list.",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "tool.install_tool",
        name: "Install OS Packages",
        domain: "tools",
        category: CapabilityCategory::Settings,
        description: "Install OS or language packages (apt/dnf/brew/winget/pipx/npm/cargo). \
                      High impact: only available when Full access / tool installation is enabled.",
        how_to: "Enable in Settings → Agent OS access (Full access mode).",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "security.action_sandbox",
        name: "Action Sandbox",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Dedicated action directory for agent tools (shell, file, git), separate \
                      from internal application state. Agent tools default their working directory \
                      and path resolution to the action sandbox, preventing accidental modification \
                      of memory databases, session transcripts, tokens, and other internal state.",
        how_to: "Settings → Agent OS access",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "security.sandbox_backends",
        name: "Sandbox Execution Backends",
        domain: "security",
        category: CapabilityCategory::Settings,
        description: "Route agent tool execution (shell, filesystem, process) through sandbox \
                      backends — Docker containers or OS-level jails (Landlock/Seatbelt) — for \
                      reduced blast radius on remote, channel, cron, or background sessions. \
                      Configurable per agent/session/channel with safe defaults for non-main sessions.",
        how_to: "Set sandbox_mode = \"sandboxed\" in agent.toml, or configure runtime.kind = \
                 \"docker\" in the TOML config. Use openhuman.sandbox_status / \
                 openhuman.sandbox_resolve_policy RPC to inspect.",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.remember_preferences",
        name: "Remember Preferences",
        domain: "memory",
        category: CapabilityCategory::Intelligence,
        description: "Remember preferences you state in chat and apply them automatically — \
                      general preferences shape every reply (tone, language, standing habits); \
                      situational ones surface only when relevant to your current message.",
        how_to: "State a preference in chat, e.g. \"always reply in British English\" or \
                 \"when writing Rust, prefer Result over unwrap\".",
        status: CapabilityStatus::Stable,
        privacy: LOCAL_RAW,
    },
];
