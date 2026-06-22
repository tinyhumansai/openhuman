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

/// Voice-domain events.
#[non_exhaustive]
#[derive(Clone, Debug)]
pub enum VoiceEvent {
    /// A PTT session committed a transcript to a thread. Carries only
    /// length/timing — never the raw text, per the PII-safe logging rule.
    PttTranscriptCommitted {
        thread_id: String,
        session_id: u64,
        text_len: usize,
        held_ms: u64,
        finalized_by_watchdog: bool,
    },
}

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
    /// A sub-agent called `ask_user_clarification` and paused, waiting
    /// for the orchestrator to relay the user's answer via
    /// `continue_subagent`.
    SubagentAwaitingUser {
        parent_session: String,
        task_id: String,
        agent_id: String,
        question: String,
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

    // ── Subconscious orchestrator ───────────────────────────────────────
    /// A subconscious trigger finished gate evaluation (promote or drop).
    /// Observability only — lets dashboards see ingestion volume and the
    /// gate's promote/drop ratio without reading logs.
    SubconsciousTriggerProcessed {
        /// Trigger source family (`cron` / `user_message` / …).
        source: String,
        /// Gate decision (`promote` / `drop`).
        decision: String,
        /// Whether the trigger was promoted into the long-lived session.
        promoted: bool,
        /// Gate evaluation latency in milliseconds.
        latency_ms: u64,
    },

    // ── Run Queue ──────────────────────────────────────────────────────
    /// A message was queued into the active-run queue instead of interrupting.
    RunQueueMessageQueued {
        thread_id: String,
        mode: String,
        queue_depth: usize,
    },
    /// A queued steer/collect message was delivered to the engine at an
    /// iteration boundary.
    RunQueueMessageDelivered {
        thread_id: String,
        mode: String,
        iteration: u32,
    },
    /// A queued followup message was dispatched as a fresh turn after the
    /// current turn completed.
    RunQueueFollowupDispatched {
        thread_id: String,
        followup_count: usize,
    },
    /// The active turn was interrupted by a new message (default behavior).
    RunQueueInterrupted {
        thread_id: String,
        cancelled_request_id: String,
    },

    // ── Monitor ───────────────────────────────────────────────────────
    /// A background monitor changed lifecycle state.
    MonitorStatusChanged {
        monitor_id: String,
        status: String,
        thread_id: Option<String>,
        description: String,
    },
    /// A background monitor emitted one bounded stdout/stderr line.
    MonitorLine {
        monitor_id: String,
        thread_id: Option<String>,
        timestamp_ms: u64,
        stream: String,
        line: String,
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
    ///
    /// `source_id` is the originating memory-source id (from
    /// `memory_sources`) when the event can be attributed to a specific
    /// source row. The frontend prefers this over `connection_id` for
    /// per-row indicator matching (see RC#2, issue #3295). Set to `None`
    /// when the event originates from a non-memory-source sync path (e.g. a
    /// channel-provider ingest) — `connection_id` remains unchanged for
    /// those callers.
    MemorySyncStageChanged {
        trigger: String,
        stage: String,
        provider: Option<String>,
        connection_id: Option<String>,
        detail: Option<String>,
        /// Originating memory-source id for frontend per-row indicator
        /// matching. `None` when the event is not attributable to a
        /// specific `MemorySourceEntry`.
        source_id: Option<String>,
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

    // ── Memory Diff ─────────────────────────────────────────────────────
    /// A snapshot of a memory source's chunk state was captured.
    MemoryDiffSnapshotTaken {
        snapshot_id: String,
        source_id: String,
        source_kind: String,
        item_count: usize,
        trigger: String,
    },
    /// A diff was computed between two snapshots.
    MemoryDiffComputed {
        source_id: String,
        from_snapshot_id: Option<String>,
        to_snapshot_id: String,
        added: usize,
        removed: usize,
        modified: usize,
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
    WorkflowLoaded { skill_id: String, runtime: String },
    /// A skill was stopped.
    WorkflowStopped { skill_id: String },
    /// A skill failed to start.
    WorkflowStartFailed { skill_id: String, error: String },
    /// A skill tool was executed.
    WorkflowExecuted {
        skill_id: String,
        tool_name: String,
        arguments: serde_json::Value,
        result: Option<String>,
        success: bool,
        elapsed_ms: u64,
    },
    /// The set of installed skills/workflows changed (install / uninstall /
    /// create). Lets a live agent session refresh its `## Installed Skills`
    /// catalogue mid-conversation instead of waiting for a restart. `reason`
    /// is a short tag for logs (e.g. `"install"`, `"uninstall"`, `"create"`).
    WorkflowsChanged { reason: String },

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

    // ── Artifacts ───────────────────────────────────────────────────────
    /// An artifact transitioned to [`ArtifactStatus::Ready`] — file
    /// is on disk and ready to be downloaded. Published by
    /// [`crate::openhuman::artifacts::store::finalize_artifact`].
    /// Bridged to the web channel as an `artifact_ready` socket event
    /// when the publishing turn carries an `APPROVAL_CHAT_CONTEXT`
    /// (see [`crate::openhuman::approval::ApprovalChatContext`]).
    /// Sub-task #2779 of #1535.
    ArtifactReady {
        /// UUID of the artifact record.
        artifact_id: String,
        /// Lowercase variant of `ArtifactKind` (`presentation`,
        /// `document`, `image`, `other`).
        kind: String,
        /// Human-readable title (also the on-disk filename stem).
        title: String,
        /// Absolute workspace root the artifact belongs to (matches
        /// the `workspace_dir` parameter passed to
        /// `finalize_artifact`). Bound to the event so a subscriber
        /// firing AFTER the user switched workspaces can detect the
        /// mismatch and drop the surface — `path` is workspace-
        /// relative and would otherwise resolve into the wrong
        /// `<workspace>/artifacts/` tree.
        workspace_dir: String,
        /// Relative path under `<workspace>/artifacts/`, e.g.
        /// `"<uuid>/deck.pptx"`. The absolute path is reachable via
        /// `ai_get_artifact` so the renderer never needs the
        /// workspace root.
        path: String,
        /// Final on-disk file size in bytes.
        size_bytes: u64,
        /// Chat thread the artifact belongs to, when the producing
        /// turn carried an `APPROVAL_CHAT_CONTEXT`. `None` for CLI /
        /// cron / sub-agent paths — no client to fan out to.
        thread_id: Option<String>,
        /// Socket.IO client id (room) to surface the card to, when
        /// known. `None` for non-chat callers.
        client_id: Option<String>,
    },
    /// An artifact transitioned to [`ArtifactStatus::Failed`] — the
    /// producer surfaced a reason and the UI should render a
    /// retry-hint card instead of a download. Bridged the same way
    /// as [`Self::ArtifactReady`]. Sub-task #2779 of #1535.
    ArtifactFailed {
        artifact_id: String,
        kind: String,
        title: String,
        /// Absolute workspace root the artifact belongs to — see
        /// [`Self::ArtifactReady::workspace_dir`] for rationale.
        workspace_dir: String,
        /// Producer-supplied failure reason. Already truncated by the
        /// producer (e.g. `PresentationError::truncate_stderr`).
        error: String,
        thread_id: Option<String>,
        client_id: Option<String>,
    },
    /// An artifact record has been **created** (`ArtifactStatus::Pending`)
    /// but no bytes are on disk yet — the producing tool has only just
    /// reserved the row. Published by
    /// [`crate::openhuman::artifacts::store::create_artifact`].
    /// Bridged to the web channel as an `artifact_pending` socket event
    /// so the frontend can render an in-progress / "Generating…" card the
    /// moment the tool dispatches, instead of waiting until the file
    /// arrives via [`Self::ArtifactReady`]. The pending card is replaced
    /// in place when the matching `ArtifactReady` / `ArtifactFailed`
    /// event with the same `artifact_id` arrives. Sub-task #3162 of #1535.
    ArtifactPending {
        /// UUID of the freshly-created artifact record.
        artifact_id: String,
        /// Lowercase variant of `ArtifactKind` (`presentation`,
        /// `document`, `image`, `other`).
        kind: String,
        /// Human-readable title (also the on-disk filename stem).
        title: String,
        /// Absolute workspace root the artifact belongs to — see
        /// [`Self::ArtifactReady::workspace_dir`] for rationale.
        workspace_dir: String,
        /// Relative path under `<workspace>/artifacts/` where the file
        /// *will* land. The frontend uses it to render a stable card key
        /// so subsequent `ArtifactReady` can swap the same surface in
        /// place without flicker.
        path: String,
        /// Chat thread the artifact belongs to, when the producing turn
        /// carried an `APPROVAL_CHAT_CONTEXT`. `None` for CLI / cron /
        /// sub-agent paths — no client to fan out to.
        thread_id: Option<String>,
        /// Socket.IO client id (room) to surface the card to, when known.
        /// `None` for non-chat callers.
        client_id: Option<String>,
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
    /// The connected Composio toolkit set changed (connect/revoke/config flip).
    ///
    /// `toolkits` is the currently-active, sanitised slug list that should
    /// drive orchestrator delegation schema rebuilds.
    ComposioIntegrationsChanged { toolkits: Vec<String> },
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

    /// Fine-grained progress during the memory tree build pipeline.
    /// Emitted at each sub-phase so the frontend can show detailed status.
    MemoryTreeBuildProgress {
        /// Which phase: "extract", "append", "seal", "flush", "embed"
        phase: String,
        /// Sub-step within the phase (e.g. "loading", "summarising", "persisting")
        step: String,
        /// Tree scope when available (e.g. "github:org/repo")
        tree_scope: Option<String>,
        /// Tree level being processed (0 = leaves, 1+ = summaries)
        level: Option<u32>,
        /// Number of items being processed in this step
        item_count: Option<u32>,
        /// Human-readable detail
        detail: Option<String>,
    },

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
    /// A remote MCP server returned a tool whose `description` or
    /// `title` failed the input-validation scan and was dropped from
    /// the registry before reaching the agent LLM context. Surfaced for
    /// audit / observability only; carries no payload content because
    /// the rejected text could itself be a vector.
    McpToolRejected {
        /// Registered MCP server name the tool came from.
        server: String,
        /// Remote tool name as advertised by the server.
        tool: String,
        /// Short pattern / rule code from the validator (e.g.
        /// `"override.ignore_previous"`). Never the rejected payload.
        reason: String,
    },

    /// An `OPENHUMAN_APPROVAL_GATE=0` env override was observed but
    /// IGNORED because the host is the Tauri desktop shell. The gate is
    /// always installed under the desktop host; this event lets the UI
    /// surface a one-shot info banner so the user sees the override was
    /// rejected. Audit-only; carries no payload content.
    ApprovalGateOverrideIgnored {
        /// Host tag (currently always `"tauri-shell"` — added for forward
        /// compatibility when more desktop hosts land).
        host: String,
    },
    /// The approval gate was NOT installed because an
    /// `OPENHUMAN_APPROVAL_GATE=0` env override was honored on a
    /// standalone host (CLI / Docker). Surfaces the elevated-privilege
    /// state so any connected dashboard can flag it; the desktop UI
    /// banner subscribes to this variant.
    ApprovalGateDisabled {
        /// Host tag (`"cli"` or `"docker"`).
        host: String,
        /// Short reason code so downstream consumers can switch on the
        /// cause without parsing free-text logs. Currently always
        /// `"env-override"`.
        reason: String,
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
    /// The agent's filesystem roots (currently the `action_dir` sandbox) were
    /// changed at runtime via `config.update_agent_paths`. The live
    /// `SecurityPolicy` is hot-swapped in-band; this broadcast lets other
    /// listeners observe the change.
    AgentPathsChanged,
    /// A component's health status changed.
    HealthChanged {
        component: String,
        healthy: bool,
        message: Option<String>,
    },
    /// A component restart was observed.
    HealthRestarted { component: String },

    // ── Keyring ─────────────────────────────────────────────────────────
    /// The OS keyring is unavailable and no user consent for local fallback
    /// has been recorded. Published once (deduplicated) when a secret
    /// operation hits the consent gate. The frontend surfaces a consent
    /// dialog in response.
    KeyringConsentRequired,
    /// A secret field failed to decrypt (rotated master key, corrupted
    /// ciphertext, keychain reset). Published so the frontend can surface
    /// a recovery prompt instead of silently clearing the field.
    KeyringDecryptFailed { field_name: String, reason: String },

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

    // ── Voice ────────────────────────────────────────────────────────────
    /// A voice domain event (PTT, transcription lifecycle, etc.).
    Voice(VoiceEvent),

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
    /// A stale or wedged task run was reclaimed — the card moved back to
    /// `todo` (re-dispatchable) or `blocked` (max reclaim count exceeded).
    TaskRunReclaimed {
        run_id: String,
        card_id: String,
        thread_id: String,
        reason: String,
    },

    // ── Backend Meet Bot ──────────────────────────────────────────────
    /// Backend gmeet bot successfully joined the meeting.
    BackendMeetJoined {
        meet_url: String,
        correlation_id: Option<String>,
    },
    /// Backend gmeet bot left the meeting.
    BackendMeetLeft {
        reason: String,
        correlation_id: Option<String>,
    },
    /// Backend gmeet bot produced a spoken reply.
    BackendMeetReply {
        transcript: String,
        reply: String,
        emotion: String,
        correlation_id: Option<String>,
    },
    /// Backend gmeet bot needs the harness to execute a tool instruction.
    BackendMeetHarness {
        transcript: String,
        instruction: String,
        emotion: String,
        correlation_id: Option<String>,
    },
    /// Backend gmeet bot sent the full meeting transcript on close.
    BackendMeetTranscript {
        turns: Vec<BackendMeetTurn>,
        duration_ms: u64,
        correlation_id: Option<String>,
    },
    /// Backend gmeet bot emitted an error.
    BackendMeetError {
        error: String,
        correlation_id: Option<String>,
    },
    /// Backend gmeet bot detected a wake-phrase command from a participant.
    BackendMeetInCallRequest {
        correlation_id: Option<String>,
        speaker: String,
        command_text: String,
        recent_transcript: Vec<BackendMeetTurn>,
        timestamp_ms: u64,
    },
    /// Core asked the backend bot to speak into the call (`bot:speak`).
    /// Published for observability after the Socket.IO emit succeeds.
    BackendMeetSpeak {
        text: String,
        correlation_id: Option<String>,
    },
    /// An approval was parked during a live-meeting orchestrator turn
    /// (issue #3513). The meeting bus speaks the prompt into the call;
    /// the decision arrives by voice ("Hey Tiny, approve") or the
    /// standard thread approval card — first response wins.
    InCallApprovalRequested {
        request_id: String,
        tool_name: String,
        action_summary: String,
        correlation_id: Option<String>,
    },
    /// A Google Calendar event with a Meet link was detected and the
    /// auto-join policy is "ask" — the UI should prompt the user.
    MeetAutoJoinPrompt {
        meet_url: String,
        event_title: String,
    },
    /// A new meeting session was created (Pending) after a calendar Meet
    /// link was detected and the auto-join prompt was surfaced (issue #3507).
    MeetingSessionCreated {
        meeting_id: String,
        meet_url: String,
        title: String,
        /// Origin of the session: "calendar" | "manual" | "api".
        source: String,
    },
    /// Auto-join was triggered for a meeting — either policy == Always or the
    /// user clicked a join action on the auto-join prompt (issue #3507).
    MeetingAutoJoinTriggered {
        meeting_id: String,
        meet_url: String,
        listen_only: bool,
        correlation_id: String,
    },
    /// Reserved for PR-4: a post-meeting summary was generated from the
    /// transcript (action items, key decisions, etc.).
    MeetingSummaryGenerated {
        thread_id: String,
        correlation_id: Option<String>,
        summary: String,
    },
    /// A JSON message arrived on a tinyplace WebSocket stream.
    /// Published by the stream manager's recv loop. Carries the raw
    /// server-sent JSON value (inbox item, conversation message, etc.)
    /// so the Socket.IO bridge can forward it to the renderer.
    TinyPlaceStreamMessage {
        /// Stream identifier (e.g. `"inbox"`, `"conversation:abc123"`).
        stream_id: String,
        /// Stream kind for routing.
        kind: String,
        /// The raw JSON message from the tinyplace server.
        message: serde_json::Value,
    },
    /// A tinyplace WebSocket stream changed lifecycle status.
    /// Published by the stream manager on connect, disconnect, and failure.
    TinyPlaceStreamStatusChanged {
        /// Stream identifier.
        stream_id: String,
        /// New status: `"connecting"`, `"connected"`, `"disconnected"`, `"failed"`.
        status: String,
    },
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct BackendMeetTurn {
    pub role: String,
    pub content: String,
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
            | Self::SubagentAwaitingUser { .. }
            | Self::AgentOrchestrationSpawned { .. }
            | Self::AgentOrchestrationCompleted { .. }
            | Self::AgentOrchestrationFailed { .. }
            | Self::AgentOrchestrationClosed { .. }
            | Self::RunQueueMessageQueued { .. }
            | Self::RunQueueMessageDelivered { .. }
            | Self::RunQueueFollowupDispatched { .. }
            | Self::RunQueueInterrupted { .. } => "agent",

            Self::MonitorStatusChanged { .. } | Self::MonitorLine { .. } => "monitor",

            Self::EmbeddingModelUnhealthy { .. }
            | Self::MemoryStored { .. }
            | Self::MemoryRecalled { .. }
            | Self::MemorySyncRequested { .. }
            | Self::MemorySyncStageChanged { .. }
            | Self::MemoryIngestionStarted { .. }
            | Self::MemoryIngestionCompleted { .. }
            | Self::DocumentCanonicalized { .. }
            | Self::MemoryDiffSnapshotTaken { .. }
            | Self::MemoryDiffComputed { .. } => "memory",

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

            Self::WorkflowLoaded { .. }
            | Self::WorkflowStopped { .. }
            | Self::WorkflowStartFailed { .. }
            | Self::WorkflowExecuted { .. }
            | Self::WorkflowsChanged { .. } => "workflow",

            Self::ToolExecutionStarted { .. } | Self::ToolExecutionCompleted { .. } => "tool",

            Self::WebhookIncomingRequest { .. }
            | Self::WebhookReceived { .. }
            | Self::WebhookRegistered { .. }
            | Self::WebhookUnregistered { .. }
            | Self::WebhookProcessed { .. } => "webhook",

            Self::ComposioTriggerReceived { .. }
            | Self::ComposioConnectionCreated { .. }
            | Self::ComposioConnectionDeleted { .. }
            | Self::ComposioIntegrationsChanged { .. }
            | Self::ComposioActionExecuted { .. }
            | Self::ComposioConfigChanged { .. } => "composio",

            Self::TriggerEvaluated { .. }
            | Self::TriggerEscalated { .. }
            | Self::TriggerEscalationFailed { .. } => "triage",

            Self::TreeSummarizerHourCompleted { .. }
            | Self::TreeSummarizerPropagated { .. }
            | Self::TreeSummarizerRebuildCompleted { .. }
            | Self::MemoryTreeBuildProgress { .. } => "tree_summarizer",

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
            | Self::AgentPathsChanged
            | Self::HealthChanged { .. }
            | Self::HealthRestarted { .. } => "system",

            Self::KeyringConsentRequired | Self::KeyringDecryptFailed { .. } => "keyring",

            Self::SessionExpired { .. } => "auth",

            Self::TaskSourceFetched { .. }
            | Self::TaskSourceTaskIngested { .. }
            | Self::TaskSourceFetchFailed { .. } => "task_sources",

            Self::TaskPlanAwaitingApproval { .. } | Self::TaskRunReclaimed { .. } => "agent",

            Self::SubconsciousTriggerProcessed { .. } => "subconscious",

            Self::Voice(_) => "voice",

            Self::ApprovalRequested { .. }
            | Self::ApprovalDecided { .. }
            | Self::ApprovalGateOverrideIgnored { .. }
            | Self::ApprovalGateDisabled { .. } => "approval",

            Self::ArtifactReady { .. }
            | Self::ArtifactFailed { .. }
            | Self::ArtifactPending { .. } => "artifact",

            Self::McpServerInstalled { .. }
            | Self::McpServerConnected { .. }
            | Self::McpServerDisconnected { .. }
            | Self::McpClientToolExecuted { .. }
            | Self::McpSetupSecretRequested { .. }
            | Self::McpToolRejected { .. } => "mcp_client",

            Self::BackendMeetJoined { .. }
            | Self::BackendMeetLeft { .. }
            | Self::BackendMeetReply { .. }
            | Self::BackendMeetHarness { .. }
            | Self::BackendMeetTranscript { .. }
            | Self::BackendMeetError { .. }
            | Self::BackendMeetInCallRequest { .. }
            | Self::BackendMeetSpeak { .. }
            | Self::InCallApprovalRequested { .. }
            | Self::MeetAutoJoinPrompt { .. }
            | Self::MeetingSessionCreated { .. }
            | Self::MeetingAutoJoinTriggered { .. }
            | Self::MeetingSummaryGenerated { .. } => "agent_meetings",

            Self::TinyPlaceStreamMessage { .. } | Self::TinyPlaceStreamStatusChanged { .. } => {
                "tinyplace"
            }
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
            Self::SubagentAwaitingUser { .. } => "SubagentAwaitingUser",
            Self::AgentOrchestrationSpawned { .. } => "AgentOrchestrationSpawned",
            Self::AgentOrchestrationCompleted { .. } => "AgentOrchestrationCompleted",
            Self::AgentOrchestrationFailed { .. } => "AgentOrchestrationFailed",
            Self::AgentOrchestrationClosed { .. } => "AgentOrchestrationClosed",
            Self::SubconsciousTriggerProcessed { .. } => "SubconsciousTriggerProcessed",
            Self::RunQueueMessageQueued { .. } => "RunQueueMessageQueued",
            Self::RunQueueMessageDelivered { .. } => "RunQueueMessageDelivered",
            Self::RunQueueFollowupDispatched { .. } => "RunQueueFollowupDispatched",
            Self::RunQueueInterrupted { .. } => "RunQueueInterrupted",
            Self::MonitorStatusChanged { .. } => "MonitorStatusChanged",
            Self::MonitorLine { .. } => "MonitorLine",
            Self::MemoryStored { .. } => "MemoryStored",
            Self::MemoryRecalled { .. } => "MemoryRecalled",
            Self::MemorySyncRequested { .. } => "MemorySyncRequested",
            Self::MemorySyncStageChanged { .. } => "MemorySyncStageChanged",
            Self::MemoryIngestionStarted { .. } => "MemoryIngestionStarted",
            Self::MemoryIngestionCompleted { .. } => "MemoryIngestionCompleted",
            Self::DocumentCanonicalized { .. } => "DocumentCanonicalized",
            Self::MemoryDiffSnapshotTaken { .. } => "MemoryDiffSnapshotTaken",
            Self::MemoryDiffComputed { .. } => "MemoryDiffComputed",
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
            Self::WorkflowLoaded { .. } => "WorkflowLoaded",
            Self::WorkflowStopped { .. } => "WorkflowStopped",
            Self::WorkflowStartFailed { .. } => "WorkflowStartFailed",
            Self::WorkflowExecuted { .. } => "WorkflowExecuted",
            Self::WorkflowsChanged { .. } => "WorkflowsChanged",
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
            Self::ComposioIntegrationsChanged { .. } => "ComposioIntegrationsChanged",
            Self::ComposioActionExecuted { .. } => "ComposioActionExecuted",
            Self::ComposioConfigChanged { .. } => "ComposioConfigChanged",
            Self::TriggerEvaluated { .. } => "TriggerEvaluated",
            Self::TriggerEscalated { .. } => "TriggerEscalated",
            Self::TriggerEscalationFailed { .. } => "TriggerEscalationFailed",
            Self::TreeSummarizerHourCompleted { .. } => "TreeSummarizerHourCompleted",
            Self::TreeSummarizerPropagated { .. } => "TreeSummarizerPropagated",
            Self::TreeSummarizerRebuildCompleted { .. } => "TreeSummarizerRebuildCompleted",
            Self::MemoryTreeBuildProgress { .. } => "MemoryTreeBuildProgress",
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
            Self::AgentPathsChanged => "AgentPathsChanged",
            Self::HealthChanged { .. } => "HealthChanged",
            Self::HealthRestarted { .. } => "HealthRestarted",
            Self::KeyringConsentRequired => "KeyringConsentRequired",
            Self::KeyringDecryptFailed { .. } => "KeyringDecryptFailed",
            Self::SessionExpired { .. } => "SessionExpired",
            Self::ApprovalRequested { .. } => "ApprovalRequested",
            Self::ApprovalDecided { .. } => "ApprovalDecided",
            Self::ApprovalGateOverrideIgnored { .. } => "ApprovalGateOverrideIgnored",
            Self::ApprovalGateDisabled { .. } => "ApprovalGateDisabled",
            Self::ArtifactReady { .. } => "ArtifactReady",
            Self::ArtifactFailed { .. } => "ArtifactFailed",
            Self::ArtifactPending { .. } => "ArtifactPending",
            Self::McpServerInstalled { .. } => "McpServerInstalled",
            Self::McpServerConnected { .. } => "McpServerConnected",
            Self::McpServerDisconnected { .. } => "McpServerDisconnected",
            Self::McpClientToolExecuted { .. } => "McpClientToolExecuted",
            Self::McpSetupSecretRequested { .. } => "McpSetupSecretRequested",
            Self::McpToolRejected { .. } => "McpToolRejected",
            Self::EmbeddingModelUnhealthy { .. } => "EmbeddingModelUnhealthy",
            Self::TaskSourceFetched { .. } => "TaskSourceFetched",
            Self::TaskSourceTaskIngested { .. } => "TaskSourceTaskIngested",
            Self::TaskSourceFetchFailed { .. } => "TaskSourceFetchFailed",
            Self::TaskPlanAwaitingApproval { .. } => "TaskPlanAwaitingApproval",
            Self::TaskRunReclaimed { .. } => "TaskRunReclaimed",
            Self::BackendMeetJoined { .. } => "BackendMeetJoined",
            Self::BackendMeetLeft { .. } => "BackendMeetLeft",
            Self::BackendMeetReply { .. } => "BackendMeetReply",
            Self::BackendMeetHarness { .. } => "BackendMeetHarness",
            Self::BackendMeetTranscript { .. } => "BackendMeetTranscript",
            Self::BackendMeetError { .. } => "BackendMeetError",
            Self::BackendMeetInCallRequest { .. } => "BackendMeetInCallRequest",
            Self::BackendMeetSpeak { .. } => "BackendMeetSpeak",
            Self::InCallApprovalRequested { .. } => "InCallApprovalRequested",
            Self::MeetAutoJoinPrompt { .. } => "MeetAutoJoinPrompt",
            Self::MeetingSessionCreated { .. } => "MeetingSessionCreated",
            Self::MeetingAutoJoinTriggered { .. } => "MeetingAutoJoinTriggered",
            Self::MeetingSummaryGenerated { .. } => "MeetingSummaryGenerated",
            Self::TinyPlaceStreamMessage { .. } => "TinyPlaceStreamMessage",
            Self::TinyPlaceStreamStatusChanged { .. } => "TinyPlaceStreamStatusChanged",
            Self::Voice(_) => "Voice",
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
            | Self::SubagentAwaitingUser { agent_id, .. }
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
            Self::RunQueueMessageQueued { thread_id, .. }
            | Self::RunQueueMessageDelivered { thread_id, .. }
            | Self::RunQueueFollowupDispatched { thread_id, .. }
            | Self::RunQueueInterrupted { thread_id, .. } => Some(thread_id.as_str()),
            Self::MonitorStatusChanged { thread_id, .. } | Self::MonitorLine { thread_id, .. } => {
                thread_id.as_deref()
            }
            _ => None,
        }
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
