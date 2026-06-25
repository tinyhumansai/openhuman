use super::super::types::{
    Capability, CapabilityCategory, CapabilityPrivacy, CapabilityStatus, PrivacyDataKind,
};

const LOCAL_RAW: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: false,
    data_kind: PrivacyDataKind::Raw,
    destinations: &[],
});

const DERIVED_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Derived,
    destinations: &["OpenHuman backend", "TinyHumans Neocortex"],
});

// Vision sub-agent ships the attached image (raw pixels) to the managed
// multimodal model for analysis.
const IMAGE_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["OpenHuman backend", "TinyHumans Neocortex"],
});

const LOCAL_CREDENTIALS: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: false,
    data_kind: PrivacyDataKind::Credentials,
    destinations: &[],
});

const DIAGNOSTICS_TO_BACKEND: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Diagnostics,
    destinations: &["OpenHuman backend"],
});

const MODEL_DOWNLOAD: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["Hugging Face"],
});

// Self-update flows talk to GitHub Releases directly, not the OpenHuman
// backend. The outbound payload is metadata only (release list query for
// `update.check`, asset download URL request for `update.apply`) so
// `data_kind: Metadata` is the right label — but the destination must
// reflect that this is a third-party host, otherwise the capability
// catalog under-reports where the user's request actually goes.
const GITHUB_RELEASES_METADATA: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["GitHub Releases"],
});

// GitHub repo memory source: the reader queries a repository's activity
// (commits / issues / PRs) directly against the GitHub API — via the `gh`
// CLI when available, otherwise the public REST API — not through the
// OpenHuman backend. The *outbound* payload is metadata (which repo, which
// activity, pagination) plus whatever auth `gh` carries; the fetched content
// is archived locally under the vault and only its embeddings travel onward
// (covered by the embedding-provider capability). Mirrors the
// `GITHUB_RELEASES_METADATA` shape — third-party GitHub host, metadata-class
// outbound — so the Privacy surface reflects that the request leaves the
// device to a destination distinct from the managed backend.
const GITHUB_REPO_SOURCE: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["GitHub API (api.github.com)"],
});

const SEARXNG_RAW_TO_CONFIGURED_INSTANCE: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Raw,
    destinations: &["Configured SearXNG instance"],
});

// Direct-mode Composio: the user's API key and tool arguments leave the
// device — they are sent to backend.composio.dev, not the OpenHuman backend.
// LOCAL_CREDENTIALS was incorrect here because leaves_device must be true.
const COMPOSIO_DIRECT_CREDENTIALS: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Credentials,
    destinations: &["Composio (backend.composio.dev)"],
});

const POLYMARKET_MARKET_DATA: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Metadata,
    destinations: &["Polymarket Gamma API", "Polymarket CLOB API"],
});

const POLYMARKET_TRADING_DATA: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Derived,
    destinations: &["Polymarket CLOB API"],
});

// "Test Connection" on the Embeddings settings panel routes a small probe
// payload to *whichever provider the user has selected* — not just the
// managed cloud default. `DERIVED_TO_BACKEND` only enumerates the managed
// path (OpenHuman backend / Neocortex), which under-reports the actual
// privacy surface when the user has switched to OpenAI / Cohere / a
// self-hosted endpoint. The catalog needs to list every reachable
// destination so the Privacy surface can render the full set instead of
// implying probes always stay on the managed path.
const EMBEDDING_PROBE_TO_CONFIGURED_PROVIDER: Option<CapabilityPrivacy> = Some(CapabilityPrivacy {
    leaves_device: true,
    data_kind: PrivacyDataKind::Derived,
    destinations: &[
        "OpenHuman backend / TinyHumans Neocortex (managed cloud default)",
        "OpenAI API (api.openai.com)",
        "Cohere API (api.cohere.com)",
        "User-configured OpenAI-compatible endpoint (custom:<url>)",
    ],
});

pub(super) const CAPABILITIES: &[Capability] = &[
    Capability {
        id: "conversation.create",
        name: "Create Conversations",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Start a new conversation thread with the assistant.",
        how_to: "Conversations",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "conversation.send_text",
        name: "Send Text Messages",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Send typed messages to the assistant in a conversation.",
        how_to: "Conversations > Message composer",
        status: CapabilityStatus::Stable,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "conversation.prompt_injection_guard",
        name: "Prompt Injection Guard",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Detect and block prompt-injection attempts before agent/model execution.",
        how_to: "Conversations > Message composer",
        status: CapabilityStatus::Stable,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "conversation.send_voice",
        name: "Send Voice Messages",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Record or attach voice input and send it as a message.",
        how_to: "Conversations > Voice input",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "voice.ptt",
        name: "Global push-to-talk",
        domain: "voice",
        category: CapabilityCategory::Conversation,
        description: "Hold a global hotkey from anywhere on the desktop to dictate into the \
                      active chat thread. Press opens the mic, release commits the transcript, \
                      and an always-on-top overlay shows listening/idle state without stealing \
                      focus. Cross-platform via tauri-plugin-global-shortcut (macOS, Windows, \
                      Linux/X11); requires microphone access and a global shortcut binding. \
                      Optional speak_reply plays the agent's response through local TTS.",
        how_to: "Settings → Voice → Push-to-Talk: pick a shortcut, grant microphone access, \
                 then hold the configured hotkey from any window.",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "conversation.inline_autocomplete",
        name: "Inline Autocomplete",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Show predictive inline text suggestions while you type.",
        how_to: "Settings > Inline Autocomplete",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "conversation.copy_messages",
        name: "Copy Messages",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Copy individual assistant or user messages for reuse elsewhere.",
        how_to: "Conversations > Message actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "conversation.delete_conversations",
        name: "Delete Conversations",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Remove existing conversation threads from the app.",
        how_to: "Conversations > Thread actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "conversation.suggested_questions",
        name: "Suggested Questions",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Offer prompt suggestions to help continue a conversation.",
        how_to: "Home or Conversations > Suggested prompts",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "conversation.tool_execution_timeline",
        name: "Tool Execution Timeline",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Show the sequence of tool calls and actions used to answer a request.",
        how_to: "Conversations > Tool timeline",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "conversation.background_monitors",
        name: "Background Monitors",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Start, inspect, and stop bounded background command monitors that stream new events into active agent work.",
        how_to: "Conversations > ask the assistant to monitor a command or status source",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "conversation.subagent_mascots",
        name: "Subagent Mascots",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Show delegated sub-agents as colored companion mascots with compact activity bubbles and running, completed, or failed states.",
        how_to: "Human > ask the assistant to delegate work to sub-agents",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.vision_subagent",
        name: "Vision Sub-agent",
        domain: "agent",
        category: CapabilityCategory::Intelligence,
        description: "Delegate image / screenshot understanding to a dedicated vision sub-agent — describe, OCR, read charts/diagrams, compare images, or locate UI elements. Rides the multimodal `vision-v1` tier so attached images are always analyzed.",
        how_to: "Attach an image in chat, or ask the assistant to look at a screenshot / image file",
        status: CapabilityStatus::Beta,
        privacy: IMAGE_TO_BACKEND,
    },
    Capability {
        id: "conversation.label_filter",
        name: "Thread Label Filters",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Filter the thread list by label (Work, Briefing, Notification) using the tab bar at the top of the thread list.",
        how_to: "Conversations > Label tabs",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.analyze_actionable_items",
        name: "Analyze Actionable Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Extract and summarize actionable items from your activity and conversations.",
        how_to: "Intelligence",
        status: CapabilityStatus::Stable,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "intelligence.filter_actionable_items",
        name: "Filter Actionable Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Search and filter actionable items to focus on what matters now.",
        how_to: "Intelligence > Filters and search",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.mark_actionable_item_complete",
        name: "Mark Items Complete",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Mark an actionable item as completed.",
        how_to: "Intelligence > Item actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.dismiss_actionable_item",
        name: "Dismiss Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Dismiss irrelevant or already handled actionable items.",
        how_to: "Intelligence > Item actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.snooze_actionable_item",
        name: "Snooze Items",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Temporarily hide an actionable item until later.",
        how_to: "Intelligence > Item actions",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "intelligence.undo_action",
        name: "Undo Item Actions",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Undo a recent complete, dismiss, or snooze action.",
        how_to: "Intelligence > Undo snackbar or item history",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.agentmemory_backend",
        name: "agentmemory Memory Backend",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Opt-in Memory trait backend that delegates every store/recall/get/list/forget \
            call to a locally-running agentmemory REST server. Selected via \
            `memory.backend = \"agentmemory\"` in config.toml. Allows users who self-host \
            agentmemory across Claude Code, Cursor, Codex, and OpenCode to share a single durable \
            memory store. Default backend remains sqlite; selecting agentmemory is non-breaking.",
        how_to: "Set `memory.backend = \"agentmemory\"` in config.toml. \
            See gitbooks/features/obsidian-wiki/agentmemory-backend.md for setup and config keys.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.memory_workspace",
        name: "Memory Workspace",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Inspect or debug the app's memory workspace and stored knowledge.",
        how_to: "Settings > Memory Debug",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.tool_scoped_memory",
        name: "Tool-Scoped Memory Rules",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Store durable, tool-specific rules and corrections that survive context \
            compression. Critical-priority rules (e.g. 'never email Sarah') are pinned into the \
            system prompt at session start. Captured automatically from user edicts and repeated \
            tool failures; also writable programmatically via the memory.tool_rule_* RPC surface.",
        how_to: "Automatic — user edicts are captured after every turn. Manage via \
            memory.tool_rule_put / memory.tool_rule_list / memory.tool_rule_delete (RPC).",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.long_term_goals",
        name: "Long-term Goals",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "An editable list of the assistant's durable long-term goals for working with \
            you, stored locally in MEMORY_GOALS.md (capped ~500 tokens). A background goals agent \
            keeps the list fresh: it runs when the conversation context is summarized, and on first \
            run populates initial goals from context. Items can be added/edited/deleted explicitly \
            via RPC or agent tools.",
        how_to: "Automatic — refreshed on context summarization. Manage via \
            memory_goals.list / memory_goals.add / memory_goals.edit / memory_goals.delete / \
            memory_goals.reflect (RPC), or the goals_* agent tools.",
        status: CapabilityStatus::Beta,
        // Enrichment runs a cloud agentic model, so goal/context text can leave
        // the device during a reflect pass (CRUD/storage stays local).
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "conversation.thread_goal",
        name: "Thread Goal",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "A single, thread-scoped goal (Codex-style \"completion contract\") the \
            assistant keeps pursuing across turns, interrupts, resumes, and budget boundaries — \
            distinct from the long-term goals list and the per-thread task board. Stored locally \
            (one goal per thread), with a lifecycle (active/paused/budget_limited/complete) and an \
            optional token budget. The active goal is injected into context each turn; the context \
            scout proposes a goal on a fresh thread (only if none is set) and the orchestrator can \
            set/refine it. When enabled, idle threads can autonomously continue toward the goal.",
        how_to: "Set/edit via the goal chip above the composer in Conversations, or the \
            thread_goals.* RPC (get/set/complete/pause/resume/clear); the assistant manages it via \
            the goal_set / goal_get / goal_complete tools. Autonomous continuation is opt-in via \
            heartbeat.goal_continuation_enabled.",
        status: CapabilityStatus::Beta,
        // Goal CRUD/storage is local; autonomous continuation (opt-in) runs a
        // cloud agentic model, so objective/context can leave the device then.
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "intelligence.memory_tree_retrieval",
        name: "Memory Tree Retrieval (chat)",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Ask questions about your ingested email/chat/document memory in chat. The orchestrator can resolve names to canonical ids, query summaries by source/topic/global window, drill into details, and cite raw chunks.",
        how_to: "Chat > ask the assistant about people, conversations, or windows",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.memory_pipeline_doctor",
        name: "Memory Pipeline Doctor",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Diagnose why the memory tree / wiki is empty or stalled. Walks each pipeline stage (embeddings config, scheduler gate, job queue, extraction/recall degradation, summary-tree precondition) and reports the single first blocking cause with an actionable fix, plus counters and extraction coverage. The agent can run it on itself; a typed 'first blocking cause' is surfaced in the Memory status panel, and jobs that failed under a now-fixed config can be requeued on demand via the `memory_tree_retry_failed` RPC.",
        how_to: "Memory status panel shows the cause + fix; or ask the agent to diagnose memory; or `openhuman-core` RPC `memory_tree_doctor`",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.github_repo_memory_source",
        name: "GitHub Repo Memory Source",
        domain: "memory_sources",
        category: CapabilityCategory::Intelligence,
        description: "Sync a GitHub repository's project activity — commits, issues, and \
            pull requests (not source code) — into your memory. Items are archived verbatim \
            under a browsable, repo-grouped vault layout \
            (raw/github-com-<owner>-<repo>/{commits,issues,prs}/) and ingested into the \
            memory tree for recall. Contributors are surfaced as @handle entities, and \
            commit messages plus closed/merged issues & PRs get a priority boost so \
            high-signal history leads at summary time. Pulls up to 2000 items of each type \
            per sync by default, overridable per source via max_commits / max_issues / \
            max_prs.",
        how_to: "Settings > Memory & Data > Memory Sources — add a GitHub repository URL. \
            Programmatic: openhuman.memory_sources_add (RPC).",
        status: CapabilityStatus::Beta,
        privacy: GITHUB_REPO_SOURCE,
    },
    Capability {
        id: "intelligence.memory_source_sync_controls",
        name: "Memory Source Sync Defaults & Controls",
        domain: "memory_sources",
        category: CapabilityCategory::Intelligence,
        description: "Connected memory sources are enabled by default with conservative, \
            per-kind sync caps so the first sync stays cheap (e.g. Gmail ~100 recent emails, \
            GitHub repo 10 PRs / 10 issues / 50 commits, RSS 20 items). Each source row exposes \
            an inline settings panel to adjust the limit fields that apply to its kind \
            (max_items, sync_depth_days, max_prs/issues/commits, since_days). \
            An \"All In\" action enables every source and removes the caps to build the richest \
            memory graph, then triggers a full sync. Already-connected sources are migrated to \
            the new defaults once.",
        how_to: "Intelligence > Memory Sources — toggle a source, open its gear for per-source \
            limits, or use \"All In\". Programmatic: openhuman.memory_sources_update and \
            openhuman.memory_sources_apply_all_in (RPC).",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.memory_sync_schedule",
        name: "Memory Sync Schedule",
        domain: "config",
        category: CapabilityCategory::Intelligence,
        description: "Pick a single global cadence for how often all opted-in memory sources \
            auto-sync, presented like a backup schedule (\"Last synced … · Sync every …\"). \
            Presets are every 4h / 12h / 24h, plus \"Manual only\" which disables background \
            auto-sync entirely (you can still sync on demand). The chosen interval overrides each \
            provider's built-in cadence but is floored at it, so syncs never run more often than \
            the provider intends — handy for keeping credit spend predictable. Unset defaults to \
            every 24h.",
        how_to: "Intelligence > Memory Sources — choose a Sync every… preset or Manual only. \
            Programmatic: openhuman.config_get_memory_sync_settings / \
            openhuman.config_update_memory_sync_settings (RPC); ops override via the \
            OPENHUMAN_MEMORY_SYNC_INTERVAL_SECS env var (0 = manual).",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.embedding_provider_config",
        name: "Configure Embedding Provider",
        domain: "embeddings",
        category: CapabilityCategory::Intelligence,
        description:
            "Pick which embedding provider drives semantic search across your memory: \
             managed cloud (default, Voyage-backed via api.tinyhumans.ai), OpenAI, \
             Cohere, local Ollama, or a custom OpenAI-compatible endpoint. API keys \
             are stored encrypted via the local keyring under `embeddings:<slug>`; \
             model name and embedding dimensions are tunable per provider. The \
             legacy `inference_embed` RPC is aliased to `embeddings_embed` so \
             existing callers continue to work.",
        how_to: "Settings > AI > Embeddings",
        status: CapabilityStatus::Beta,
        // Privacy depends on the selected provider — see
        // `intelligence.embedding_provider_test` for the per-provider data
        // destinations. The configuration surface itself only writes to the
        // local keyring and config, so leaving this `None` (treat-as-unknown)
        // would under-report; we annotate the credential side here and the
        // network side on the test action.
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "intelligence.embedding_provider_test",
        name: "Test Embedding Provider",
        domain: "embeddings",
        category: CapabilityCategory::Intelligence,
        description:
            "Verify a configured embedding provider before committing it to \
             memory ingestion. Sends a small one-shot embed request and reports \
             the model, dimensions, and any auth/error surface so a \
             misconfigured key doesn't get discovered halfway through a 50k \
             chunk backfill.",
        how_to: "Settings > AI > Embeddings > Test Connection",
        // The probe payload routes to whichever provider the user has
        // selected — managed cloud (default), OpenAI, Cohere, or a custom
        // OpenAI-compatible endpoint. Using `DERIVED_TO_BACKEND` here would
        // under-report by only listing the managed path; the dedicated
        // constant enumerates every reachable destination so the Privacy
        // surface renders the full set.
        status: CapabilityStatus::Beta,
        privacy: EMBEDDING_PROBE_TO_CONFIGURED_PROVIDER,
    },
    Capability {
        id: "intelligence.mcp_server",
        name: "MCP Server",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Expose a curated OpenHuman tool surface over stdio MCP or Streamable HTTP/SSE for MCP-compatible clients.",
        how_to: "Run `openhuman-core mcp` (stdio) or `openhuman-core mcp --transport http --port 9300` for remote clients.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.searxng_search",
        name: "SearXNG Search",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Search a configured self-hosted SearXNG instance from agent and MCP tools, returning normalized title, URL, snippet, and source results.",
        how_to: "Set `[searxng] enabled = true` and `base_url` in config.toml, or use OPENHUMAN_SEARXNG_* environment variables.",
        status: CapabilityStatus::Beta,
        privacy: SEARXNG_RAW_TO_CONFIGURED_INSTANCE,
    },
    Capability {
        id: "intelligence.tool_registry",
        name: "Tool Registry",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Discover OpenHuman's MCP stdio tools and controller-backed tools from one local registry, including versions, routes, input/output schemas, allowed agents, and health state.",
        how_to: "Call openhuman.tool_registry_list over core JSON-RPC, or openhuman.tool_registry_get with a tool_id such as memory.search.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.orchestrator_worker_thread",
        name: "Worker Thread Delegation",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "When a delegated sub-task is long or complex, the orchestrator can route it into a fresh worker-labeled conversation thread instead of flooding the parent thread. The user opens the worker thread from the thread list (or via the reference card in the parent) to read the sub-agent's full transcript.",
        how_to: "Conversations > tap the worker reference card in the parent thread, or open the worker-labeled thread from the thread list",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "intelligence.workflow_orchestration",
        name: "Workflow Orchestration",
        domain: "workflow_runs",
        category: CapabilityCategory::Intelligence,
        description: "Run declarative multi-agent workflows such as parallel research with cross-checking: a question is decomposed into angles, researched in parallel, adversarially cross-checked, and synthesized into one cited report. Watch each phase progress with its child agent results, stop or resume a run, and read the final synthesis. High-cost / high-concurrency runs require explicit approval before starting.",
        how_to: "Intelligence > Orchestration > pick a workflow and Start",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "intelligence.agent_library",
        name: "Agents Library",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Browse safe display metadata for registered agent definitions, compare worker capabilities, and start a one-off task with an explicitly selected agent.",
        how_to: "Intelligence > Agent Tasks > Agents Library",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "intelligence.worktree_manager",
        name: "Agent Worktrees",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Inspect and clean up the isolated git worktrees that parallel sub-agents check out under <repo>/.claude/worktrees. Each row shows the worktree's branch, dirty state, and changed files, plus a cross-worktree overlap warning when two workers touched the same file. Open, diff, or remove a worktree (a dirty worktree requires an explicit discard confirmation; the worker branch is preserved).",
        how_to: "Intelligence > Worktrees",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.slack_memory_ingest",
        name: "Slack Memory Ingestion",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Backfill the last 6 days of Slack history into the memory tree and keep it up to date by flushing each closed 6-hour UTC bucket. Driven by an authenticated Slack connection (OAuth via Composio).",
        how_to: "Settings > Messaging Channels > Slack",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.clickup_memory_ingest",
        name: "ClickUp Memory Ingestion",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Incrementally sync ClickUp tasks assigned to the authenticated user into the Memory Tree on a 30-minute cadence, with an initial backfill on first connect. Only tasks the user is directly assigned to are ingested. Driven by an authenticated ClickUp connection (OAuth via Composio).",
        how_to: "Settings > Connections > ClickUp",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.notifications_dismiss",
        name: "Dismiss Notifications",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Dismiss low-value notifications from the intelligence inbox.",
        how_to: "Notifications > Item actions",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.notifications_mark_acted",
        name: "Mark Notifications Acted",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Mark a notification as acted upon after taking follow-up action.",
        how_to: "Notifications > Item actions",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "intelligence.notifications_stats",
        name: "View Notification Stats",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "View aggregate unread, unscored, and provider/action notification stats.",
        how_to: "Notifications > Summary cards",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "workflows.discover",
        name: "Discover Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Browse available workflows that can extend the app.",
        how_to: "Intelligence > Workflows",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "workflows.install",
        name: "Install Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Install a workflow into the local workspace.",
        how_to: "Intelligence > Workflows > Install",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "workflows.configure",
        name: "Configure Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Open workflow setup and update workflow-specific configuration.",
        how_to: "Intelligence > Workflows > Setup or Settings > Connections",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "workflows.connection_status",
        name: "Monitor Workflow Connection Status",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "See whether a workflow-backed integration is connected, offline, or needs setup.",
        how_to: "Intelligence > Workflows or Settings > Connections",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "workflows.sync_manual",
        name: "Manually Sync Workflow Data",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Trigger a manual data sync for a workflow integration.",
        how_to: "Intelligence > Workflows > Workflow card > Sync",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "workflows.run_apify_actors",
        name: "Run Apify Actors",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Launch Apify scrapers and automation actors, then inspect run status and collected results.",
        how_to: "Conversations > Ask the assistant to run an Apify actor",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "workflows.tinyfish_web_automation",
        name: "TinyFish Web Automation",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description:
            "Search the web, render JavaScript-heavy pages, and run goal-based browser automations through TinyFish.",
        how_to: "Conversations > Ask the assistant to search, fetch, or automate a website with TinyFish",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "workflows.toggle_enabled",
        name: "Enable or Disable Workflows",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Turn individual workflows on or off without uninstalling them.",
        how_to: "Settings > Developer Options > Workflows",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "workflows.open_connections_hub",
        name: "Open Connections Hub",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Browse the dedicated connections hub for external workflow-backed integrations.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    // ── Composio direct mode (BYO API key) ──────────────────────────
    //
    // Composio shipped with two integration paths:
    //   1. Backend-proxied (default) — calls through api.tinyhumans.ai;
    //      backend owns the Composio API key, billing, allowlist, and
    //      HMAC-verified trigger fan-out via socket.io.
    //   2. Direct (BYO key) — core calls backend.composio.dev directly
    //      with the user's own key. Sovereign / offline-friendly, but
    //      tool execution only — real-time trigger webhooks are NOT
    //      routed in direct mode (they still require the backend).
    Capability {
        id: "composio.direct_mode",
        name: "Composio Direct Mode (BYO API Key)",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description:
            "Route Composio tool calls directly to backend.composio.dev with your own API key, \
             bypassing the OpenHuman backend proxy. Tool execution only — trigger webhooks still \
             require backend mode.",
        how_to: "Settings > Skills > Composio > Direct mode",
        status: CapabilityStatus::Beta,
        privacy: COMPOSIO_DIRECT_CREDENTIALS,
    },
    Capability {
        id: "composio.direct_mode_triggers_gap",
        name: "Composio Triggers (Direct Mode — Limited)",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description:
            "Composio real-time trigger webhooks (Gmail new-message, Slack new-message, …) \
             currently arrive over wss://api.tinyhumans.ai/socket.io and require backend mode. \
             Direct-mode users get synchronous tool execution but not async trigger push in \
             this release.",
        how_to: "Switch to Backend mode to receive triggers, or wait for the direct trigger sink follow-up",
        status: CapabilityStatus::ComingSoon,
        privacy: None,
    },
    Capability {
        id: "workflows.connect_google",
        name: "Connect Google",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Connect Google services for email, contacts, and calendar workflows.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::ComingSoon,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "workflows.connect_notion",
        name: "Connect Notion",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Connect Notion for workspace sync and productivity workflows.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::ComingSoon,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "workflows.connect_web3_wallet",
        name: "Connect Web3 Wallet",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Set up local EVM, BTC, Solana, and Tron wallet identities from one recovery phrase.",
        how_to: "Settings > Crypto > Recovery Phrase or Settings > Connections",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "workflows.wallet_execution",
        name: "Wallet Execution Tools",
        domain: "wallet",
        category: CapabilityCategory::Workflows,
        description: "Read addresses and balances, prepare/confirm/execute native + token transfers (ERC20/SPL/TRC20/BEP20), and inspect transactions (status, receipt, lookup) across the connected wallet (EVM, BTC, Solana, Tron). Quote-first; signing stays local.",
        how_to: "Use wallet.* RPC methods (balances, prepare_transfer, execute_prepared, tx_status, tx_receipt, lookup_tx) via the agent or core_rpc_relay, or via Settings > Crypto > Wallet Balances.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "workflows.web3_defi",
        name: "Web3 Swaps & Bridges",
        domain: "web3",
        category: CapabilityCategory::Workflows,
        description: "Quote and execute cross-chain swaps and bridges (deBridge) plus generic EVM dapp contract calls, built on the local wallet's signing. EVM/Solana(/BTC); signing stays local.",
        how_to: "Use web3_swap.* / web3_bridge.* / web3_dapp.* RPC methods (quote/execute, web3_swap.routes) via the agent or core_rpc_relay.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "workflows.x402_payments",
        name: "x402 Machine Payments",
        domain: "x402",
        category: CapabilityCategory::Workflows,
        description: "Automatic HTTP 402 payment handling for machine-payable APIs via the x402 protocol. When an API returns 402 Payment Required, the agent pays with USDC on Solana using the local wallet and retries. Budget enforcement with per-request, daily, and monthly caps.",
        how_to: "Use x402.* RPC methods (get_summary, list_payments, update_budget) to manage spending. Payments happen automatically when the http_request tool encounters a 402 with a PAYMENT-REQUIRED header.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "workflows.connect_crypto_exchange",
        name: "Connect Crypto Exchange",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Connect supported exchanges for trading and portfolio workflows.",
        how_to: "Settings > Connections",
        status: CapabilityStatus::ComingSoon,
        privacy: None,
    },
    Capability {
        id: "workflows.polymarket_readonly",
        name: "Polymarket Read-Only Browse",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Browse Polymarket markets, events, orderbooks, and prices via Gamma + CLOB APIs.",
        how_to: "Conversations > ask the assistant to browse Polymarket (tool: polymarket).",
        status: CapabilityStatus::Beta,
        privacy: POLYMARKET_MARKET_DATA,
    },
    Capability {
        id: "workflows.polymarket_trading",
        name: "Polymarket Trading",
        domain: "workflows",
        category: CapabilityCategory::Workflows,
        description: "Place and cancel Polymarket limit orders with EIP-712 signing, authenticated account reads, and explicit approval for writes.",
        how_to: "Conversations > ask the assistant to trade on Polymarket (tool: polymarket; set `approved=true` for write actions).",
        status: CapabilityStatus::Beta,
        privacy: POLYMARKET_TRADING_DATA,
    },
    Capability {
        id: "local_ai.download_model",
        name: "Download Local Models",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Download and bootstrap local AI runtimes and model bundles.",
        how_to: "Settings > Local AI Model",
        status: CapabilityStatus::Beta,
        privacy: MODEL_DOWNLOAD,
    },
    Capability {
        id: "local_ai.configure_provider",
        name: "Configure Local Provider",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Select Ollama, LM Studio, MLX, or a generic local OpenAI-compatible server as the local model provider and configure the endpoint.",
        how_to: "Settings > AI > providers, or use provider strings: ollama:<model>, lmstudio:<model>, mlx:<model>, local-openai:<model>",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.manage_model_assets",
        name: "Manage Model Assets",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Inspect asset status and download specific chat, vision, embedding, STT, or TTS assets.",
        how_to: "Settings > Local AI Model > Advanced > Capability Assets",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.model_context_check",
        name: "Model Context Requirement Check",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Diagnostics report each installed Ollama model's native context window and reject any model below the minimum the memory layer requires (so short-context models can't silently truncate and corrupt recall).",
        how_to: "Settings > Local AI Model > Run Diagnostics",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.embed_text",
        name: "Generate Text Embeddings",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Create local vector embeddings for text input.",
        how_to: "Settings > Local AI Model > Advanced > Test Embeddings",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "local_ai.speech_to_text",
        name: "Speech Recognition (Local)",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description:
            "Transcribe audio into text using local whisper.cpp via the voice STT factory. \
             Pick the model size (tiny / base / small / medium / large-v3-turbo) in \
             Settings > Voice; the factory routes through WHISPER_BIN or the in-process engine.",
        how_to: "Settings > Voice > STT Provider = Whisper",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.text_to_speech",
        name: "Text to Speech (Local)",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description:
            "Synthesize speech locally with Piper via the voice TTS factory. PIPER_BIN points \
             at the binary; the voice .onnx ships with the installer. Returns a synthetic \
             viseme timeline (full forced-alignment lives behind the cloud provider for now).",
        how_to: "Settings > Voice > TTS Provider = Piper",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.vision_processing",
        name: "Vision Processing",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Run vision prompts against images using a local multimodal model.",
        how_to: "Settings > Local AI Model > Advanced > Test Vision Prompt",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.direct_prompting",
        name: "Direct Model Prompting",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description: "Send a direct prompt to the local model without using the cloud API.",
        how_to: "Settings > Local AI Model > Advanced > Test Custom Prompt",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "local_ai.whisper_installer",
        name: "Whisper Installer (Local STT)",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description:
            "One-click download of the whisper.cpp GGML model (and on Windows the whisper-cli \
             binary) into the workspace so local Speech-to-Text runs without manual setup. \
             Streams to disk via a .part file + atomic rename so a crash never leaves a corrupt \
             model behind.",
        how_to: "Settings > Voice > Voice Providers > Install Whisper",
        status: CapabilityStatus::Beta,
        privacy: MODEL_DOWNLOAD,
    },
    Capability {
        id: "local_ai.piper_installer",
        name: "Piper Installer (Local TTS)",
        domain: "local_ai",
        category: CapabilityCategory::LocalAI,
        description:
            "One-click download of the Piper binary archive and the bundled en_US-lessac-medium \
             voice (.onnx + .onnx.json) into the workspace so local Text-to-Speech runs without \
             manual setup. Atomic rename guarantees no half-written voice files are ever read \
             by the runtime.",
        how_to: "Settings > Voice > Voice Providers > Install Piper",
        status: CapabilityStatus::Beta,
        privacy: MODEL_DOWNLOAD,
    },
    Capability {
        id: "local_ai.python_runtime_installer",
        name: "Managed Python Runtime",
        domain: "runtime_python",
        category: CapabilityCategory::LocalAI,
        description:
            "Download and reuse an OpenHuman-managed CPython runtime for Python-backed local integrations such as MCP servers, with a system-Python override reserved for development.",
        how_to: "Configured by the core `runtime_python` module; future UI surfaces can expose install state and overrides.",
        status: CapabilityStatus::Beta,
        privacy: MODEL_DOWNLOAD,
    },
    Capability {
        id: "team.create",
        name: "Create Teams",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Create a team and start collaborating with shared billing and members.",
        how_to: "Settings > Team",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.join_via_invite_code",
        name: "Join Teams via Invite Code",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Join an existing team using an invite code.",
        how_to: "Invites > Redeem an Invite Code",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.switch_active_team",
        name: "Switch Active Team",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Switch which team is currently active in the app.",
        how_to: "Settings > Team",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.leave",
        name: "Leave Teams",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Leave a team that you no longer want to participate in.",
        how_to: "Settings > Team",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.manage_members",
        name: "Manage Team Members",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Review members and change team roles when you have permission.",
        how_to: "Settings > Team > Manage team > Members",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.generate_invite_codes",
        name: "Generate Invite Codes",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "Create invite codes to bring new members into a team.",
        how_to: "Settings > Team > Manage team > Invites",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "team.track_invite_usage",
        name: "Track Invite Usage",
        domain: "team",
        category: CapabilityCategory::Team,
        description: "View invite usage counts, limits, and revoke team invites.",
        how_to: "Settings > Team > Manage team > Invites",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.login_oauth",
        name: "Login via OAuth",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Sign in with the app's supported provider-based authentication flow.",
        how_to: "Welcome",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.onboarding_setup",
        name: "Onboarding Setup",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Walk through onboarding to configure initial permissions and preferences.",
        how_to: "Onboarding",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.configure_tool_access",
        name: "Configure Tool Access",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Choose which built-in tools OpenHuman can use on your behalf during setup.",
        how_to: "Onboarding > Enable Tools",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.backup_recovery_phrase",
        name: "Back Up Recovery Phrase",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Generate and save a recovery phrase used to secure and restore encrypted app data.",
        how_to: "Onboarding > Recovery Phrase",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.import_recovery_phrase",
        name: "Import Recovery Phrase",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Import an existing recovery phrase to restore encrypted app data.",
        how_to: "Onboarding > Recovery Phrase > I already have a recovery phrase",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "auth.logout",
        name: "Logout",
        domain: "auth",
        category: CapabilityCategory::Auth,
        description: "Sign out of the current session.",
        how_to: "Settings > Log out",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.toggle_monitoring",
        name: "Enable or Disable Screen Monitoring",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Turn desktop screen intelligence capture on or off.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "screen_intelligence.manage_accessibility_permissions",
        name: "Manage Accessibility Permissions",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Review and grant the accessibility permissions required for desktop assistance.",
        how_to: "Onboarding > Screen permissions or Settings > Accessibility Automation",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.review_vision_data",
        name: "Review Vision Data",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Inspect the captured screen intelligence and related vision summaries.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "screen_intelligence.configure_capture_fps",
        name: "Configure Capture FPS",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Tune the screen capture frame rate used by screen intelligence.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.app_whitelist",
        name: "Whitelist Apps for Capture",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Allow screen intelligence only for selected applications.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "screen_intelligence.app_blacklist",
        name: "Blacklist Apps from Capture",
        domain: "screen_intelligence",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Exclude selected applications from screen intelligence capture.",
        how_to: "Settings > Screen Intelligence",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.connect_platform",
        name: "Connect Messaging Platforms",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Connect supported messaging platforms such as Telegram, Discord, or Slack.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.telegram_remote_control",
        name: "Telegram Remote Control",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description:
            "Operate OpenHuman from Telegram with slash commands: /status, /sessions, /new, and /help.",
        how_to: "Settings > Messaging Channels > Telegram (connect), then message the bot",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.disconnect_platform",
        name: "Disconnect Messaging Platforms",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Disconnect a previously configured messaging platform.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.test_credentials",
        name: "Test Channel Credentials",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Validate platform credentials or connection state before using a channel.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.set_default_channel",
        name: "Set Default Messaging Channel",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Choose which messaging channel should be used by default.",
        how_to: "Settings > Messaging Channels",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "channels.whatsapp_read_messages",
        name: "Read WhatsApp Messages",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Read and search WhatsApp Web conversations and messages after connecting WhatsApp in OpenHuman. Data is stored locally only and never transmitted.",
        how_to: "Connect WhatsApp Web via Channels, then ask the agent to read or summarise your messages.",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "channels.mcp_registry_browse",
        name: "Browse MCP Server Registry",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Search and discover MCP servers from the Smithery.ai and official modelcontextprotocol registries.",
        how_to: "Skills > MCP > Browse catalog",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Metadata,
            destinations: &["Smithery.ai registry API", "modelcontextprotocol registry API"],
        }),
    },
    Capability {
        id: "channels.mcp_server_install",
        name: "Install MCP Servers",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Install MCP servers locally — both local stdio subprocesses and hosted HTTP-remote servers. Required env vars are stored encrypted and never included in logs or responses. Can also be done conversationally via the MCP setup assistant.",
        how_to: "Skills > MCP > Browse catalog > Install, or ask the assistant to \"set up the <name> MCP server\"",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_CREDENTIALS,
    },
    Capability {
        id: "channels.mcp_server_connect",
        name: "Connect / Reconfigure MCP Servers",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Spawn and manage MCP server connections (stdio subprocess or HTTP-remote). Reconfigure stored env vars and reconnect without uninstalling.",
        how_to: "Skills > MCP > select a server > Connect / Reconfigure",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Derived,
            destinations: &["Configured MCP endpoint(s)"],
        }),
    },
    Capability {
        id: "channels.mcp_tool_call",
        name: "Invoke MCP Server Tools",
        domain: "channels",
        category: CapabilityCategory::Channels,
        description: "Call tools exposed by connected MCP servers. Tools are surfaced to the agent and runnable from the tool playground.",
        how_to: "Skills > MCP > select a connected server > Tools > Try, or ask the assistant in Chat",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Derived,
            destinations: &["Configured MCP endpoint(s)"],
        }),
    },
    Capability {
        id: "settings.configure_ai",
        name: "Configure AI",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Configure managed, local, custom, and built-in BYOK LLM providers, including SumoPod and other OpenAI-compatible gateways, plus per-workload routing preferences.",
        how_to: "Settings > AI",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "settings.persona_pack",
        name: "Persona Pack",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Personalize the assistant as one identity: set a display name and description, edit or reset the SOUL.md personality prompt, and reach mascot avatar and voice settings — all from a single Persona surface.",
        how_to: "Settings > Persona",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "settings.manage_privacy_analytics",
        name: "Manage Privacy and Analytics",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Control privacy, analytics, and related data handling preferences. \
            When enabled, anonymous crash reports are sent to Sentry and anonymous usage \
            analytics (page views, feature engagement) are sent to Google Analytics. \
            No personal data, messages, or credentials are ever included.",
        how_to: "Settings > Privacy (direct route)",
        status: CapabilityStatus::Stable,
        privacy: DIAGNOSTICS_TO_BACKEND,
    },
    Capability {
        id: "settings.view_billing",
        name: "View Billing",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Open subscription, included usage, and pay-as-you-go billing views for your active team.",
        how_to: "Settings > Billing & Usage",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "settings.manage_subscription_plan",
        name: "Manage Subscription Plan",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Upgrade plans or open the billing portal to manage subscription-backed usage tiers.",
        how_to: "Settings > Billing & Usage",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "settings.manage_credits",
        name: "Manage Credits",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "View pay-as-you-go credit balances, top up overage credits, and configure auto-recharge.",
        how_to: "Settings > Billing & Usage",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
    Capability {
        id: "settings.add_payment_methods",
        name: "Add Payment Methods",
        domain: "settings",
        category: CapabilityCategory::Settings,
        description: "Add or manage saved payment methods for billing and auto-recharge.",
        how_to: "Settings > Billing & Usage > Payment Methods",
        status: CapabilityStatus::Stable,
        privacy: None,
    },
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
    // ── Meet ────────────────────────────────────────────────────────────────
    Capability {
        id: "meet.join_call",
        name: "Join Google Meet Calls",
        domain: "meet",
        category: CapabilityCategory::Channels,
        description: "Join a Google Meet call as an anonymous guest in a dedicated CEF webview \
                      window with an isolated profile. The agent automatically dismisses the \
                      device-check, types its display name, and clicks Ask-to-join via CDP; the \
                      host admits the agent from the Meet waiting room.",
        how_to: "Intelligence > Calls",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Metadata,
            destinations: &["Google Meet"],
        }),
    },
    Capability {
        id: "meet_agent.live_loop",
        name: "Live Meet Agent — Listen + Speak",
        domain: "meet_agent",
        category: CapabilityCategory::Automation,
        description: "While the agent is in a Google Meet call, it listens to the other \
                      participants by tapping the embedded webview's audio output, runs \
                      VAD-segmented speech-to-text, decides whether to respond, and speaks \
                      back through a virtual microphone the embedded Chromium reads as if \
                      it were a real input device. No system audio permission required — \
                      capture and playback both stay inside the CEF process.",
        how_to: "Automatic once a Meet call is open via Intelligence > Calls.",
        status: CapabilityStatus::Beta,
        privacy: Some(CapabilityPrivacy {
            leaves_device: true,
            data_kind: PrivacyDataKind::Derived,
            destinations: &["Google Meet", "ElevenLabs (STT/TTS via hosted backend)"],
        }),
    },
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
    // ── Desktop Companion ────────────────────────────────────────────
    Capability {
        id: "companion.session",
        name: "Desktop Companion Session",
        domain: "desktop_companion",
        category: CapabilityCategory::ScreenIntelligence,
        description: "Start a Clicky-style companion session that ties hotkey activation, \
                      microphone capture, screen context, LLM reasoning, speech synthesis, \
                      and visual pointing into a single interaction loop.",
        how_to: "Settings > Companion, or activate via the configured hotkey.",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
    },
    Capability {
        id: "companion.pointing",
        name: "Visual Pointing",
        domain: "desktop_companion",
        category: CapabilityCategory::ScreenIntelligence,
        description: "The companion LLM can embed [POINT:x,y:label:screenN] tags to \
                      visually point at UI elements on screen via the overlay.",
        how_to: "Automatic during companion sessions when the LLM identifies a UI target.",
        status: CapabilityStatus::Beta,
        privacy: None,
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
