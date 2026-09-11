use super::*;

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
        id: "voice.stt_engine",
        name: "Speech Recognition Engine",
        domain: "voice",
        category: CapabilityCategory::Conversation,
        description: "Choose which hosted engine transcribes your speech. \"Backend\" uses \
                      OpenHuman's transcription proxy and needs no setup; ElevenLabs and OpenAI \
                      call the provider directly with your own API key. Audio always leaves the \
                      device — the bundled offline whisper.cpp engine was removed, so there is \
                      no local option.",
        how_to: "Settings → Voice → Speech recognition engine",
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
        id: "conversation.terminal_chat",
        name: "Tabbed Terminal UI",
        domain: "tui",
        category: CapabilityCategory::Conversation,
        description: "Operate OpenHuman from a terminal through four tabs: live core logs, \
                      orchestrator chat, safe configuration, and account settings. Bare \
                      `openhuman` opens it on an interactive non-container host; `openhuman tui` \
                      (alias `chat`) forces it. The chat streams replies, thinking, and tools live.",
        how_to: "Run `openhuman`, or `openhuman tui` to force the UI. Use Tab/Shift+Tab or Alt+1-4 \
                 to switch Logs, Chat, Config, and Settings. `--thread <id>` resumes a chat and \
                 `--new` starts one. Settings accepts a one-time login token and supports account \
                 refresh and logout. Use `openhuman --no-tui` to suppress automatic launch.",
        status: CapabilityStatus::Beta,
        privacy: DERIVED_TO_BACKEND,
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
        id: "conversation.plan_review",
        name: "Plan Review",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Pause an interactive turn for review whenever the assistant proposes a thread-scoped plan (a multi-step to-do list with its objective). Review the whole plan once above the composer, then Approve to run it, Reject to discard it, or send feedback to have the assistant revise and re-propose — nothing executes until you approve. Background and scheduled runs are never gated.",
        how_to: "Conversations > review the plan card above the composer when the assistant lays out a multi-step plan",
        status: CapabilityStatus::Beta,
        privacy: None,
    },
    Capability {
        id: "conversation.subagent_mascots",
        name: "Subagent Mascots",
        domain: "conversation",
        category: CapabilityCategory::Conversation,
        description: "Show delegated sub-agents as colored mascots with compact activity bubbles and running, completed, or failed states.",
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
        id: "intelligence.image_generation",
        name: "Image Generation",
        domain: "agent",
        category: CapabilityCategory::Intelligence,
        description: "Delegate image creation to a dedicated image sub-agent — generate images from a text prompt, or edit/restyle reference images, using hosted GMI models (Seedream / SeedEdit). Results are saved to the workspace.",
        how_to: "Ask the assistant to generate, draw, or edit an image",
        status: CapabilityStatus::Beta,
        privacy: MEDIA_GEN_TO_BACKEND,
    },
    Capability {
        id: "intelligence.video_generation",
        name: "Video Generation",
        domain: "agent",
        category: CapabilityCategory::Intelligence,
        description: "Delegate short-video creation to a dedicated video sub-agent — text-to-video or animate a reference image using hosted GMI models (Seedance / Veo). Generation is asynchronous; the finished clip is saved to the workspace.",
        how_to: "Ask the assistant to generate a video or animate an image",
        status: CapabilityStatus::Beta,
        privacy: MEDIA_GEN_TO_BACKEND,
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
        id: "intelligence.agents_md_instructions",
        name: "AGENTS.md Project Instructions",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Load configurable standing instructions from AGENTS.md files into the agent's \
            system prompt — OpenHuman's analog of Claude Code's CLAUDE.md / Codex's AGENTS.md. Two \
            layers are read once at session start: a global layer from the OpenHuman workspace \
            (<workspace_dir>/AGENTS.md) and a project layer from the folder the agent is operating \
            in (<action_dir>/AGENTS.md, or a sub-agent's isolated worktree). The global layer is \
            injected first, the project layer second (project instructions take precedence). \
            Missing or empty files are silently skipped, and each layer is capped so a large file \
            can't crowd out the rest of the prompt. On by default; disable via \
            `agent.agents_md_enabled = false`.",
        how_to: "Create an AGENTS.md file in your OpenHuman workspace and/or your project's action \
            directory. Toggle off with `agent.agents_md_enabled = false` in config.toml.",
        status: CapabilityStatus::Stable,
        privacy: AGENTS_MD_TO_INFERENCE_PROVIDER,
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
        id: "intelligence.coding_session_memory",
        name: "Coding-Agent Session Memory",
        domain: "memory_sources",
        category: CapabilityCategory::Intelligence,
        description: "Discover local Codex and Claude Code session histories, retain only human-authored decisions and corrections, and distill them into a durable TinyCortex persona memory pack. Tool output, reasoning, developer prompts, and subagent traffic are excluded before inference.",
        how_to: "Brain > Sources > Coding-agent sessions > Ingest new sessions. Programmatic: openhuman.memory_sources_coding_session_status and openhuman.memory_sources_ingest_coding_sessions (RPC).",
        status: CapabilityStatus::Beta,
        privacy: CODING_SESSION_TO_BACKEND,
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
        how_to: "Connections → API keys → Embeddings",
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
        how_to: "Connections → API keys → Embeddings → Test Connection",
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
        how_to: "Connections > OAuth > Slack",
        status: CapabilityStatus::Beta,
        privacy: LOCAL_RAW,
    },
    Capability {
        id: "intelligence.clickup_memory_ingest",
        name: "ClickUp Memory Ingestion",
        domain: "intelligence",
        category: CapabilityCategory::Intelligence,
        description: "Incrementally sync ClickUp tasks assigned to the authenticated user into the Memory Tree on a 30-minute cadence, with an initial backfill on first connect. Only tasks the user is directly assigned to are ingested. Driven by an authenticated ClickUp connection (OAuth via Composio).",
        how_to: "Connections > OAuth > ClickUp",
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
];
