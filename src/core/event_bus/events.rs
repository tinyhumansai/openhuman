//! Domain events for cross-module communication.
//!
//! Events carry full payloads so subscribers have everything they need without
//! secondary lookups. The broadcast channel clones each event per subscriber,
//! which is fine — richness beats round-trips.
//!
//! ## Workspace-scoped events
//!
//! Some events are scoped to a specific workspace directory and must be
//! validated by subscribers before acting on them.
//!
//! **Publisher contract**: when constructing a workspace-scoped event, the
//! publisher must populate `workspace_dir` with the active workspace path at
//! event creation time. This is typically available as `ctx.workspace_dir`
//! on the channel runtime context.
//!
//! **Subscriber contract**: subscribers that persist or mutate workspace-
//! specific data must compare the event's `workspace_dir` against their own
//! workspace binding and silently drop events that do not match. This prevents
//! stale in-flight events from a previous workspace from corrupting the newly
//! active workspace's state when the user switches workspaces (e.g. logs out
//! and back in) while events are in flight.
//!
//! **Current workspace-scoped variants**:
//! - [`DomainEvent::ChannelMessageReceived`]
//! - [`DomainEvent::ChannelMessageProcessed`]

/// Top-level domain event. Non-exhaustive so new variants can be added
/// without breaking existing match arms.
#[non_exhaustive]
#[derive(Debug, Clone)]
pub enum DomainEvent {
    // ── Agent ───────────────────────────────────────────────────────────
    /// An agent turn has started processing.
    AgentTurnStarted { session_id: String, channel: String },
    /// An agent turn completed with a final response.
    AgentTurnCompleted {
        session_id: String,
        text_chars: usize,
        iterations: usize,
    },
    /// An error occurred during agent processing.
    AgentError {
        session_id: String,
        message: String,
        recoverable: bool,
    },
    /// A sub-agent was dispatched via `spawn_subagent`.
    SubagentSpawned {
        /// Parent agent's session id.
        parent_session: String,
        /// Sub-agent definition id (e.g. `researcher`, `notion_specialist`, `fork`).
        agent_id: String,
        /// Spawn mode — `"typed"` or `"fork"`.
        mode: String,
        /// Per-spawn task id (UUID).
        task_id: String,
        /// Length of the prompt the parent passed in.
        prompt_chars: usize,
    },
    /// A sub-agent finished successfully.
    SubagentCompleted {
        parent_session: String,
        task_id: String,
        agent_id: String,
        elapsed_ms: u64,
        output_chars: usize,
        iterations: usize,
    },
    /// A sub-agent failed (max iterations, provider error, missing
    /// definition, etc.). The error string is suitable for logging
    /// and surfacing to the parent model.
    SubagentFailed {
        parent_session: String,
        task_id: String,
        agent_id: String,
        error: String,
    },
    /// High-level orchestration accepted a child agent for execution.
    AgentOrchestrationSpawned {
        session_id: String,
        orchestration_id: String,
        agent_id: String,
        parent_agent_id: Option<String>,
    },
    /// High-level orchestration observed a child agent completion.
    AgentOrchestrationCompleted {
        session_id: String,
        orchestration_id: String,
        agent_id: String,
        elapsed_ms: u64,
        output_chars: usize,
        iterations: usize,
    },
    /// High-level orchestration observed a child agent failure.
    AgentOrchestrationFailed {
        session_id: String,
        orchestration_id: String,
        agent_id: String,
        error: String,
    },
    /// High-level orchestration closed or cancelled a child agent.
    AgentOrchestrationClosed {
        session_id: String,
        orchestration_id: String,
        reason: Option<String>,
    },

    // ── Memory ──────────────────────────────────────────────────────────
    /// The configured embedding provider is unreachable or the requested model
    /// is not installed, so the memory pipeline fell back to an alternative.
    ///
    /// Published by `memory_store::factories` (once per process via the
    /// `OLLAMA_HEALTH_REPORTED` latch) so the UI can surface a user-visible
    /// warning with an actionable fix hint. The `message` field is a
    /// pre-formatted human-readable string safe to show in a notification.
    EmbeddingModelUnhealthy {
        /// Short provider slug, e.g. `"ollama"`.
        provider: String,
        /// The model that was intended but could not be reached / found,
        /// e.g. `"bge-m3"`.
        model: String,
        /// The provider that will serve embeddings for this session instead,
        /// e.g. `"cloud"`.
        fallback_provider: String,
        /// Human-readable explanation with an actionable fix,
        /// e.g. `"Local embedding model unreachable — falling back to cloud
        /// embeddings. Run \`ollama pull bge-m3\` to fix."`.
        message: String,
    },

    /// A memory entry was stored.
    MemoryStored {
        key: String,
        category: String,
        namespace: String,
    },
    /// A memory recall query completed.
    MemoryRecalled { query: String, hit_count: usize },
    /// A memory sync was requested for a specific channel or all channels.
    ///
    /// Published by `openhuman.memory_sync_channel` (channel_id = Some(...)) and
    /// `openhuman.memory_sync_all` (channel_id = None). No consumers exist yet —
    /// this variant is a hook for future ingestion subscribers to react to pull
    /// requests. See `src/openhuman/memory/ops.rs` for the RPC handlers.
    MemorySyncRequested { channel_id: Option<String> },
    /// A high-level memory sync orchestration stage changed.
    ///
    /// Emitted by the `memory` domain so the frontend can surface progress
    /// across request → fetch → store → queue → ingest → complete.
    MemorySyncStageChanged {
        trigger: String,
        stage: String,
        provider: Option<String>,
        connection_id: Option<String>,
        detail: Option<String>,
    },
    /// A memory ingestion job started running on the local extraction LLM.
    /// Ingestion is singleton — this fires once, then a matching
    /// [`Self::MemoryIngestionCompleted`] follows when the job finishes.
    MemoryIngestionStarted {
        document_id: String,
        title: String,
        namespace: String,
        queue_depth: usize,
    },
    /// A memory ingestion job finished (successfully or with an error).
    MemoryIngestionCompleted {
        document_id: String,
        namespace: String,
        success: bool,
        elapsed_ms: u64,
        queue_depth: usize,
    },

    // ── Channels ────────────────────────────────────────────────────────
    /// An inbound channel message from the transport layer, ready for processing.
    ///
    /// `sender`, `reply_target`, and `thread_ts` are carried alongside
    /// `channel` so the agent loop can derive per-sender conversation keys
    /// the same way `channels::context::conversation_history_key` does for
    /// other inbound paths — keying on `channel` alone collapses distinct
    /// senders inside a shared channel into one cached session.
    ChannelInboundMessage {
        event_name: String,
        channel: String,
        message: String,
        #[doc = "Originating user/account id within the channel. `None` for legacy publishers that don't surface it."]
        sender: Option<String>,
        #[doc = "Direct-message peer or group thread the reply should go to. `None` when the channel does not distinguish."]
        reply_target: Option<String>,
        #[doc = "Slack/Discord thread anchor when the message is in-thread. `None` for top-level messages."]
        thread_ts: Option<String>,
        raw_data: serde_json::Value,
    },
    /// A message was received on a channel.
    ChannelMessageReceived {
        channel: String,
        message_id: String,
        sender: String,
        reply_target: String,
        content: String,
        thread_ts: Option<String>,
        /// Workspace directory active when this event was published.
        /// Subscribers that persist data must reject events whose
        /// `workspace_dir` does not match their own workspace binding.
        workspace_dir: std::path::PathBuf,
    },
    /// A channel message was fully processed (LLM response sent or error).
    ChannelMessageProcessed {
        channel: String,
        message_id: String,
        sender: String,
        reply_target: String,
        content: String,
        thread_ts: Option<String>,
        response: String,
        elapsed_ms: u64,
        success: bool,
        /// Workspace directory active when this event was published.
        /// Subscribers that persist data must reject events whose
        /// `workspace_dir` does not match their own workspace binding.
        workspace_dir: std::path::PathBuf,
    },
    /// A reaction event was received from a channel transport.
    ChannelReactionReceived {
        channel: String,
        sender: String,
        target_message_id: String,
        emoji: String,
    },
    /// A reaction update was sent to a channel transport.
    ChannelReactionSent {
        channel: String,
        target_message_id: String,
        emoji: String,
        success: bool,
    },
    /// A channel connected successfully.
    ChannelConnected { channel: String },
    /// A channel disconnected.
    ChannelDisconnected { channel: String, reason: String },

    // ── Cron ────────────────────────────────────────────────────────────
    /// A cron job was triggered for execution.
    CronJobTriggered {
        job_id: String,
        job_name: String,
        job_type: String,
    },
    /// A cron job completed execution.
    CronJobCompleted {
        job_id: String,
        success: bool,
        output: String,
    },
    /// A cron job requests delivery of its output to a channel.
    CronDeliveryRequested {
        job_id: String,
        channel: String,
        target: String,
        output: String,
    },

    /// A proactive message (morning briefing, welcome, cron output, etc.)
    /// needs to be delivered to the user. The channels module routes it to
    /// the user's active channel.
    ProactiveMessageRequested {
        /// Identifies the source (e.g. `"cron:morning_briefing"`, `"cron:welcome"`).
        source: String,
        /// The message content to deliver.
        message: String,
        /// Optional job name for display/threading purposes.
        job_name: Option<String>,
    },

    // ── Skills ──────────────────────────────────────────────────────────
    /// A skill was loaded into the runtime.
    SkillLoaded { skill_id: String, runtime: String },
    /// A skill was stopped.
    SkillStopped { skill_id: String },
    /// A skill failed to start.
    SkillStartFailed { skill_id: String, error: String },
    /// A skill tool was executed.
    SkillExecuted {
        skill_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        result: Option<String>,
        success: bool,
        elapsed_ms: u64,
    },

    // ── Tools ───────────────────────────────────────────────────────────
    /// A tool execution started.
    ToolExecutionStarted {
        tool_name: String,
        session_id: String,
    },
    /// A tool execution completed.
    ToolExecutionCompleted {
        tool_name: String,
        session_id: String,
        success: bool,
        elapsed_ms: u64,
    },

    // ── Approval ────────────────────────────────────────────────────────
    /// Agent attempted a tool call that produces an external side
    /// effect; awaiting user approval. Published by `ApprovalGate`
    /// before parking the tool-call future. Issue #1339.
    ///
    /// Note: this variant intentionally does not carry a `session_id`.
    /// Session provenance is internal to `ApprovalGate`; downstream
    /// surfaces (frontend approval card, audit log readers, web channel
    /// bridge) only need the request correlation id plus optional chat
    /// thread/client routing.
    ApprovalRequested {
        /// Unique id used to correlate the decision back to the
        /// parked future.
        request_id: String,
        /// Tool name being gated (e.g. `"composio"`, `"pushover"`).
        tool_name: String,
        /// Short human-readable summary of the action, redacted of
        /// PII/secrets/message bodies (counts/shape only).
        action_summary: String,
        /// Redacted JSON arguments — also stripped of raw user content.
        args_redacted: serde_json::Value,
        /// Chat thread the gated call belongs to, when the turn originated
        /// from a chat channel — lets the web channel route a `yes`/`no`
        /// reply back to this request. `None` for non-chat callers.
        thread_id: Option<String>,
        /// Socket.IO client id (room) to surface the approval question to,
        /// when known. `None` for non-chat callers.
        client_id: Option<String>,
    },
    /// User decided a pending approval. Published by `approval_decide`
    /// RPC handler after the gate's parked future resolves.
    ApprovalDecided {
        request_id: String,
        tool_name: String,
        /// `"approve_once"`, `"approve_always_for_tool"`, or `"deny"`.
        decision: String,
    },

    // ── Webhooks ────────────────────────────────────────────────────────
    /// An incoming webhook request from the transport layer, ready for routing.
    WebhookIncomingRequest {
        request: crate::openhuman::webhooks::WebhookRequest,
        raw_data: serde_json::Value,
    },
    /// A webhook was received and routed to a skill.
    WebhookReceived {
        tunnel_id: String,
        skill_id: String,
        method: String,
        path: String,
        correlation_id: String,
    },
    /// A webhook tunnel was registered to a skill.
    WebhookRegistered {
        tunnel_id: String,
        skill_id: String,
        tunnel_name: Option<String>,
    },
    /// A webhook tunnel was unregistered from a skill.
    WebhookUnregistered { tunnel_id: String, skill_id: String },
    /// A webhook request was fully processed (includes timing and status).
    WebhookProcessed {
        tunnel_id: String,
        skill_id: String,
        method: String,
        path: String,
        correlation_id: String,
        status_code: u16,
        elapsed_ms: u64,
        error: Option<String>,
    },

    // ── Composio ────────────────────────────────────────────────────────
    /// A Composio trigger webhook arrived via the backend socket.io bridge
    /// and is ready for domain-specific dispatch.
    ComposioTriggerReceived {
        /// Toolkit slug, e.g. `"gmail"`.
        toolkit: String,
        /// Trigger slug, e.g. `"GMAIL_NEW_GMAIL_MESSAGE"`.
        trigger: String,
        /// Composio trigger event id (from backend metadata.id).
        metadata_id: String,
        /// Composio trigger UUID (from backend metadata.uuid).
        metadata_uuid: String,
        /// Provider-specific trigger payload.
        payload: serde_json::Value,
    },
    /// A Composio connection OAuth handoff was initiated (connectUrl returned).
    ComposioConnectionCreated {
        toolkit: String,
        connection_id: String,
        connect_url: String,
    },
    /// A Composio connection was removed.
    ComposioConnectionDeleted {
        toolkit: String,
        connection_id: String,
    },
    /// A Composio action was executed (success or failure) via the backend.
    ComposioActionExecuted {
        tool: String,
        success: bool,
        error: Option<String>,
        cost_usd: f64,
        elapsed_ms: u64,
    },
    /// The user changed the Composio routing configuration — either the
    /// mode (`"backend"` ↔ `"direct"`) flipped, or the direct-mode API
    /// key was stored / cleared. Subscribers should treat any cached
    /// tenant-scoped Composio state (connections, toolkit allowlists,
    /// tool catalogues) as stale and re-fetch on next access. Published
    /// by `composio_set_api_key` / `composio_clear_api_key`.
    ComposioConfigChanged {
        /// New routing mode after the change (`"backend"` or `"direct"`).
        mode: String,
        /// Whether a direct-mode API key is now present in the encrypted
        /// store. The key itself is never carried on the event.
        api_key_set: bool,
    },

    // ── Triage ──────────────────────────────────────────────────────────
    //
    // Published by `crate::openhuman::agent::triage` when an external
    // trigger (Composio webhook today, cron / webhook / other sources
    // later) has been classified by the trigger-triage agent. The
    // `source` field is a short slug like `"composio"` / `"cron"` so the
    // events stay source-agnostic — any module that calls
    // `agent::triage::run_triage` will publish these.
    /// A trigger event was evaluated by the triage agent and assigned
    /// one of the four actions (drop / acknowledge / react / escalate).
    TriggerEvaluated {
        /// Where the trigger came from — `"composio"`, `"cron"`, …
        source: String,
        /// Source-specific stable id for this trigger occurrence.
        external_id: String,
        /// Human-friendly label, e.g. `"composio/gmail/GMAIL_NEW_GMAIL_MESSAGE"`.
        display_label: String,
        /// The classifier's action as a short string
        /// (`"drop"` / `"acknowledge"` / `"react"` / `"escalate"`).
        decision: String,
        /// `true` if the triage turn ran on the local LLM, `false` if it
        /// ran on the remote default provider.
        used_local: bool,
        /// Wall-clock time from envelope receipt to published decision.
        latency_ms: u64,
    },
    /// Triage decided to hand the trigger off to another agent
    /// (`trigger_reactor` for `react`, `orchestrator` for `escalate`).
    /// Only fires for `react` / `escalate` — `drop` / `acknowledge` get
    /// only a [`Self::TriggerEvaluated`] event.
    TriggerEscalated {
        source: String,
        external_id: String,
        display_label: String,
        /// Agent definition id the trigger was handed off to.
        target_agent: String,
    },
    /// Triage failed entirely — both local and remote attempts errored,
    /// or the classifier reply could not be parsed after retry. Hooks
    /// ops dashboards and future alerting.
    TriggerEscalationFailed {
        source: String,
        external_id: String,
        reason: String,
    },

    // ── Tree Summarizer ──────────────────────────────────────────────────
    /// An hour leaf was created from buffered data.
    TreeSummarizerHourCompleted {
        namespace: String,
        node_id: String,
        token_count: u32,
    },
    /// A tree node summary was updated during propagation.
    TreeSummarizerPropagated {
        namespace: String,
        node_id: String,
        level: String,
        token_count: u32,
    },
    /// A full tree rebuild completed.
    TreeSummarizerRebuildCompleted { namespace: String, total_nodes: u64 },

    // ── Notification ────────────────────────────────────────────────────
    /// An integration notification was ingested from an embedded webview.
    NotificationIngested {
        id: String,
        provider: String,
        account_id: Option<String>,
    },
    /// An integration notification's triage scoring completed.
    NotificationTriaged {
        id: String,
        provider: String,
        /// One of: "drop", "acknowledge", "react", "escalate"
        action: String,
        importance_score: f32,
        latency_ms: u64,
        /// True when the triage result was actually routed to the orchestrator path.
        routed: bool,
    },

    // ── Device pairing ──────────────────────────────────────────────────
    /// A mobile device completed the X25519 handshake and is now paired.
    DevicePaired {
        channel_id: String,
        device_pubkey: String,
        label: Option<String>,
    },
    /// A paired device's tunnel session was revoked.
    DeviceRevoked { channel_id: String },
    /// The backend tunnel reported the peer (device) came online.
    DevicePeerOnline { channel_id: String },
    /// The backend tunnel reported the peer (device) went offline.
    DevicePeerOffline { channel_id: String },
    /// An encrypted tunnel frame arrived from the device.
    DeviceTunnelFrame {
        channel_id: String,
        payload_b64: String,
    },
    /// The backend acknowledged `tunnel:register` with channel credentials.
    DeviceTunnelRegistered {
        channel_id: String,
        pairing_token: String,
        session_token: String,
    },

    // ── Memory tree ─────────────────────────────────────────────────────
    /// A document (chat batch, email thread, or standalone document) was
    /// fully canonicalised and its chunks written to the memory tree.
    ///
    /// Emitted by `memory::tree::ingest::persist()` after the chunk upsert
    /// and extract-job enqueue complete. Subscribers (Phase 2 producers such
    /// as the email-signature parser) react to this to inspect the
    /// canonicalised content.
    DocumentCanonicalized {
        /// The source identifier passed to the ingest call (e.g. `"gmail:abc"`,
        /// `"conversations:agent"`).
        source_id: String,
        /// Kind of content — `"chat"`, `"email"`, `"document"`.
        source_kind: String,
        /// Number of chunks written to `vector_chunks` in this ingest.
        chunks_written: usize,
        /// IDs of the chunks that were written.
        chunk_ids: Vec<String>,
        /// Wall-clock seconds since epoch when canonicalisation completed.
        canonicalized_at: f64,
        /// Last ≤ 2 048 characters of the canonicalised markdown body.
        ///
        /// Populated for `email` and `document` sources so that lightweight
        /// subscribers (e.g. the email-signature parser) can inspect trailing
        /// content without hitting disk. `None` for `chat` sources where the
        /// content is conversational and doesn't contain signature-style structure.
        body_preview: Option<String>,
    },

    // ── Learning ─────────────────────────────────────────────────────────
    /// The stability detector finished a full cache rebuild cycle.
    ///
    /// Emitted by `learning::stability_detector` (Phase 3) after writing
    /// the new snapshot to `user_profile_facets`. Subscribers (Phase 4
    /// `profile_md_renderer`) react to re-render the `PROFILE.md` managed
    /// blocks.
    CacheRebuilt {
        /// Number of facets added in this cycle.
        added: usize,
        /// Number of facets evicted (below τ_evict threshold) in this cycle.
        evicted: usize,
        /// Number of facets unchanged / carried over.
        kept: usize,
        /// Total facets in the cache after the rebuild.
        total_size: usize,
        /// Wall-clock seconds since epoch when the rebuild completed.
        rebuilt_at: f64,
    },

    // ── Desktop Companion ──────────────────────────────────────────────
    /// A desktop companion session was started.
    CompanionSessionStarted { session_id: String, ttl_secs: u64 },
    /// The companion transitioned to a new state.
    CompanionStateChanged {
        session_id: String,
        state: String,
        previous_state: String,
    },
    /// A desktop companion session ended.
    CompanionSessionEnded {
        session_id: String,
        reason: String,
        turn_count: usize,
    },

    // ── MCP Clients ─────────────────────────────────────────────────────
    /// A new MCP server was installed from the Smithery registry.
    McpServerInstalled {
        server_id: String,
        qualified_name: String,
    },
    /// An MCP server subprocess connected and completed the initialize handshake.
    McpServerConnected { server_id: String, tool_count: u32 },
    /// An MCP server subprocess was disconnected or terminated.
    McpServerDisconnected {
        server_id: String,
        reason: Option<String>,
    },
    /// An MCP client tool was invoked.
    McpClientToolExecuted {
        server_id: String,
        tool_name: String,
        success: bool,
        elapsed_ms: u64,
    },
    /// The MCP setup agent asked the user for a secret value. The UI
    /// subscribes to this and renders a native prompt; on submit it calls
    /// `openhuman.mcp_setup_submit_secret`. `ref_id` is the opaque handle
    /// returned to the agent; the raw secret value never traverses this
    /// event.
    McpSetupSecretRequested {
        ref_id: String,
        key_name: String,
        prompt: String,
    },

    // ── System lifecycle ────────────────────────────────────────────────
    /// A system component started up.
    SystemStartup { component: String },
    /// A system component is shutting down.
    SystemShutdown { component: String },
    /// A restart of the current core process was requested.
    SystemRestartRequested { source: String, reason: String },
    /// A graceful shutdown of the current core process was requested.
    /// Distinct from [`Self::SystemShutdown`] (per-component shutdown
    /// notification) — this variant asks the running process to exit.
    SystemShutdownRequested { source: String, reason: String },
    /// The `[autonomy]` block (agent access mode / filesystem permissions) was
    /// changed at runtime. Live sessions should rebuild their `SecurityPolicy`
    /// from the persisted config before the next turn.
    AutonomyConfigChanged,
    /// A component's health status changed.
    HealthChanged {
        component: String,
        healthy: bool,
        message: Option<String>,
    },
    /// A component restart was observed.
    HealthRestarted { component: String },

    // ── Auth ────────────────────────────────────────────────────────────
    /// The local app session is no longer valid — typically detected when
    /// the backend returns 401 to an LLM inference call or a JSON-RPC
    /// method. Subscribers tear down the session and pause background
    /// LLM-bound work until the user signs back in.
    ///
    /// `source` is a short slug (e.g. `"llm_provider.openhuman_backend"`,
    /// `"jsonrpc.invoke_method"`) so subscribers and logs can attribute
    /// the trigger. `reason` is the sanitized error message that caused
    /// detection (already redacted by the call site) — surfaced to logs,
    /// never to Sentry or the UI verbatim.
    SessionExpired { source: String, reason: String },

    // ── Task sources ─────────────────────────────────────────────────────
    /// A task source completed a fetch pass.
    TaskSourceFetched {
        source_id: String,
        provider: String,
        fetched: usize,
        routed: usize,
        skipped: usize,
    },
    /// A single external task was ingested and routed onto the board.
    TaskSourceTaskIngested {
        source_id: String,
        provider: String,
        external_id: String,
        title: String,
        urgency: f32,
    },
    /// A task source fetch pass failed.
    TaskSourceFetchFailed {
        source_id: String,
        provider: String,
        error: String,
    },
    /// A task-board card needs human plan approval before the dispatcher will
    /// execute it (emitted when `autonomy.require_task_plan_approval` is on and
    /// the dispatcher parks a `todo` card at `awaiting_approval`).
    ///
    /// Surfacing: the parked card is persisted with status `awaiting_approval`,
    /// so the kanban board renders it with inline Approve/Reject on the next
    /// board fetch/refresh — that is the current (poll-based) surface and the
    /// reason this telemetry event has no dedicated subscriber yet. A realtime
    /// socket bridge (à la `ApprovalRequested` → `approval_request`) is a
    /// deliberate follow-up; emitting the event now lets that bridge attach
    /// without a schema change.
    TaskPlanAwaitingApproval { card_id: String, thread_id: String },
}

impl DomainEvent {
    /// Returns the domain name for routing and filtering.
    pub fn domain(&self) -> &'static str {
        match self {
            Self::AgentTurnStarted { .. }
            | Self::AgentTurnCompleted { .. }
            | Self::AgentError { .. }
            | Self::SubagentSpawned { .. }
            | Self::SubagentCompleted { .. }
            | Self::SubagentFailed { .. }
            | Self::AgentOrchestrationSpawned { .. }
            | Self::AgentOrchestrationCompleted { .. }
            | Self::AgentOrchestrationFailed { .. }
            | Self::AgentOrchestrationClosed { .. } => "agent",

            Self::EmbeddingModelUnhealthy { .. }
            | Self::MemoryStored { .. }
            | Self::MemoryRecalled { .. }
            | Self::MemorySyncRequested { .. }
            | Self::MemorySyncStageChanged { .. }
            | Self::MemoryIngestionStarted { .. }
            | Self::MemoryIngestionCompleted { .. }
            | Self::DocumentCanonicalized { .. } => "memory",

            Self::CacheRebuilt { .. } => "learning",

            Self::ChannelInboundMessage { .. }
            | Self::ChannelMessageReceived { .. }
            | Self::ChannelMessageProcessed { .. }
            | Self::ChannelReactionReceived { .. }
            | Self::ChannelReactionSent { .. }
            | Self::ChannelConnected { .. }
            | Self::ChannelDisconnected { .. } => "channel",

            Self::CronJobTriggered { .. }
            | Self::CronJobCompleted { .. }
            | Self::CronDeliveryRequested { .. }
            | Self::ProactiveMessageRequested { .. } => "cron",

            Self::SkillLoaded { .. }
            | Self::SkillStopped { .. }
            | Self::SkillStartFailed { .. }
            | Self::SkillExecuted { .. } => "skill",

            Self::ToolExecutionStarted { .. } | Self::ToolExecutionCompleted { .. } => "tool",

            Self::WebhookIncomingRequest { .. }
            | Self::WebhookReceived { .. }
            | Self::WebhookRegistered { .. }
            | Self::WebhookUnregistered { .. }
            | Self::WebhookProcessed { .. } => "webhook",

            Self::ComposioTriggerReceived { .. }
            | Self::ComposioConnectionCreated { .. }
            | Self::ComposioConnectionDeleted { .. }
            | Self::ComposioActionExecuted { .. }
            | Self::ComposioConfigChanged { .. } => "composio",

            Self::TriggerEvaluated { .. }
            | Self::TriggerEscalated { .. }
            | Self::TriggerEscalationFailed { .. } => "triage",

            Self::TreeSummarizerHourCompleted { .. }
            | Self::TreeSummarizerPropagated { .. }
            | Self::TreeSummarizerRebuildCompleted { .. } => "tree_summarizer",

            Self::NotificationIngested { .. } | Self::NotificationTriaged { .. } => "notification",

            Self::DevicePaired { .. }
            | Self::DeviceRevoked { .. }
            | Self::DevicePeerOnline { .. }
            | Self::DevicePeerOffline { .. }
            | Self::DeviceTunnelFrame { .. }
            | Self::DeviceTunnelRegistered { .. } => "device",

            Self::CompanionSessionStarted { .. }
            | Self::CompanionStateChanged { .. }
            | Self::CompanionSessionEnded { .. } => "companion",

            Self::SystemStartup { .. }
            | Self::SystemShutdown { .. }
            | Self::SystemRestartRequested { .. }
            | Self::SystemShutdownRequested { .. }
            | Self::AutonomyConfigChanged
            | Self::HealthChanged { .. }
            | Self::HealthRestarted { .. } => "system",

            Self::SessionExpired { .. } => "auth",

            Self::TaskSourceFetched { .. }
            | Self::TaskSourceTaskIngested { .. }
            | Self::TaskSourceFetchFailed { .. } => "task_sources",

            Self::TaskPlanAwaitingApproval { .. } => "agent",

            Self::ApprovalRequested { .. } | Self::ApprovalDecided { .. } => "approval",

            Self::McpServerInstalled { .. }
            | Self::McpServerConnected { .. }
            | Self::McpServerDisconnected { .. }
            | Self::McpClientToolExecuted { .. }
            | Self::McpSetupSecretRequested { .. } => "mcp_client",
        }
    }

    /// Stable variant name without payload (avoids Debug format coupling).
    pub fn variant_name(&self) -> &'static str {
        match self {
            Self::AgentTurnStarted { .. } => "AgentTurnStarted",
            Self::AgentTurnCompleted { .. } => "AgentTurnCompleted",
            Self::AgentError { .. } => "AgentError",
            Self::SubagentSpawned { .. } => "SubagentSpawned",
            Self::SubagentCompleted { .. } => "SubagentCompleted",
            Self::SubagentFailed { .. } => "SubagentFailed",
            Self::AgentOrchestrationSpawned { .. } => "AgentOrchestrationSpawned",
            Self::AgentOrchestrationCompleted { .. } => "AgentOrchestrationCompleted",
            Self::AgentOrchestrationFailed { .. } => "AgentOrchestrationFailed",
            Self::AgentOrchestrationClosed { .. } => "AgentOrchestrationClosed",
            Self::MemoryStored { .. } => "MemoryStored",
            Self::MemoryRecalled { .. } => "MemoryRecalled",
            Self::MemorySyncRequested { .. } => "MemorySyncRequested",
            Self::MemorySyncStageChanged { .. } => "MemorySyncStageChanged",
            Self::MemoryIngestionStarted { .. } => "MemoryIngestionStarted",
            Self::MemoryIngestionCompleted { .. } => "MemoryIngestionCompleted",
            Self::DocumentCanonicalized { .. } => "DocumentCanonicalized",
            Self::CacheRebuilt { .. } => "CacheRebuilt",
            Self::ChannelInboundMessage { .. } => "ChannelInboundMessage",
            Self::ChannelMessageReceived { .. } => "ChannelMessageReceived",
            Self::ChannelMessageProcessed { .. } => "ChannelMessageProcessed",
            Self::ChannelReactionReceived { .. } => "ChannelReactionReceived",
            Self::ChannelReactionSent { .. } => "ChannelReactionSent",
            Self::ChannelConnected { .. } => "ChannelConnected",
            Self::ChannelDisconnected { .. } => "ChannelDisconnected",
            Self::CronJobTriggered { .. } => "CronJobTriggered",
            Self::CronJobCompleted { .. } => "CronJobCompleted",
            Self::CronDeliveryRequested { .. } => "CronDeliveryRequested",
            Self::ProactiveMessageRequested { .. } => "ProactiveMessageRequested",
            Self::SkillLoaded { .. } => "SkillLoaded",
            Self::SkillStopped { .. } => "SkillStopped",
            Self::SkillStartFailed { .. } => "SkillStartFailed",
            Self::SkillExecuted { .. } => "SkillExecuted",
            Self::ToolExecutionStarted { .. } => "ToolExecutionStarted",
            Self::ToolExecutionCompleted { .. } => "ToolExecutionCompleted",
            Self::WebhookIncomingRequest { .. } => "WebhookIncomingRequest",
            Self::WebhookReceived { .. } => "WebhookReceived",
            Self::WebhookRegistered { .. } => "WebhookRegistered",
            Self::WebhookUnregistered { .. } => "WebhookUnregistered",
            Self::WebhookProcessed { .. } => "WebhookProcessed",
            Self::ComposioTriggerReceived { .. } => "ComposioTriggerReceived",
            Self::ComposioConnectionCreated { .. } => "ComposioConnectionCreated",
            Self::ComposioConnectionDeleted { .. } => "ComposioConnectionDeleted",
            Self::ComposioActionExecuted { .. } => "ComposioActionExecuted",
            Self::ComposioConfigChanged { .. } => "ComposioConfigChanged",
            Self::TriggerEvaluated { .. } => "TriggerEvaluated",
            Self::TriggerEscalated { .. } => "TriggerEscalated",
            Self::TriggerEscalationFailed { .. } => "TriggerEscalationFailed",
            Self::TreeSummarizerHourCompleted { .. } => "TreeSummarizerHourCompleted",
            Self::TreeSummarizerPropagated { .. } => "TreeSummarizerPropagated",
            Self::TreeSummarizerRebuildCompleted { .. } => "TreeSummarizerRebuildCompleted",
            Self::NotificationIngested { .. } => "NotificationIngested",
            Self::NotificationTriaged { .. } => "NotificationTriaged",
            Self::DevicePaired { .. } => "DevicePaired",
            Self::DeviceRevoked { .. } => "DeviceRevoked",
            Self::DevicePeerOnline { .. } => "DevicePeerOnline",
            Self::DevicePeerOffline { .. } => "DevicePeerOffline",
            Self::DeviceTunnelFrame { .. } => "DeviceTunnelFrame",
            Self::DeviceTunnelRegistered { .. } => "DeviceTunnelRegistered",
            Self::CompanionSessionStarted { .. } => "CompanionSessionStarted",
            Self::CompanionStateChanged { .. } => "CompanionStateChanged",
            Self::CompanionSessionEnded { .. } => "CompanionSessionEnded",
            Self::SystemStartup { .. } => "SystemStartup",
            Self::SystemShutdown { .. } => "SystemShutdown",
            Self::SystemRestartRequested { .. } => "SystemRestartRequested",
            Self::SystemShutdownRequested { .. } => "SystemShutdownRequested",
            Self::AutonomyConfigChanged => "AutonomyConfigChanged",
            Self::HealthChanged { .. } => "HealthChanged",
            Self::HealthRestarted { .. } => "HealthRestarted",
            Self::SessionExpired { .. } => "SessionExpired",
            Self::ApprovalRequested { .. } => "ApprovalRequested",
            Self::ApprovalDecided { .. } => "ApprovalDecided",
            Self::McpServerInstalled { .. } => "McpServerInstalled",
            Self::McpServerConnected { .. } => "McpServerConnected",
            Self::McpServerDisconnected { .. } => "McpServerDisconnected",
            Self::McpClientToolExecuted { .. } => "McpClientToolExecuted",
            Self::McpSetupSecretRequested { .. } => "McpSetupSecretRequested",
            Self::EmbeddingModelUnhealthy { .. } => "EmbeddingModelUnhealthy",
            Self::TaskSourceFetched { .. } => "TaskSourceFetched",
            Self::TaskSourceTaskIngested { .. } => "TaskSourceTaskIngested",
            Self::TaskSourceFetchFailed { .. } => "TaskSourceFetchFailed",
            Self::TaskPlanAwaitingApproval { .. } => "TaskPlanAwaitingApproval",
        }
    }

    /// Best-effort agent/session hint for display (not all events carry one).
    pub fn agent_hint(&self) -> Option<&str> {
        match self {
            Self::AgentTurnStarted { session_id, .. }
            | Self::AgentTurnCompleted { session_id, .. }
            | Self::AgentError { session_id, .. } => Some(session_id.as_str()),
            Self::SubagentSpawned { agent_id, .. }
            | Self::SubagentCompleted { agent_id, .. }
            | Self::SubagentFailed { agent_id, .. }
            | Self::AgentOrchestrationSpawned { agent_id, .. }
            | Self::AgentOrchestrationCompleted { agent_id, .. }
            | Self::AgentOrchestrationFailed { agent_id, .. } => Some(agent_id.as_str()),
            Self::AgentOrchestrationClosed {
                orchestration_id, ..
            } => Some(orchestration_id.as_str()),
            Self::ChannelMessageReceived { channel, .. }
            | Self::ChannelConnected { channel, .. }
            | Self::ChannelDisconnected { channel, .. } => Some(channel.as_str()),
            Self::ToolExecutionStarted { tool_name, .. }
            | Self::ToolExecutionCompleted { tool_name, .. } => Some(tool_name.as_str()),
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
