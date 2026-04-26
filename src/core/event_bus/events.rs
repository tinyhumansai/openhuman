//! Domain events for cross-module communication.
//!
//! Events carry full payloads so subscribers have everything they need without
//! secondary lookups. The broadcast channel clones each event per subscriber,
//! which is fine — richness beats round-trips.

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

    // ── Memory ──────────────────────────────────────────────────────────
    /// A memory entry was stored.
    MemoryStored {
        key: String,
        category: String,
        namespace: String,
    },
    /// A memory recall query completed.
    MemoryRecalled { query: String, hit_count: usize },

    // ── Channels ────────────────────────────────────────────────────────
    /// An inbound channel message from the transport layer, ready for processing.
    ChannelInboundMessage {
        event_name: String,
        channel: String,
        message: String,
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

    // ── System lifecycle ────────────────────────────────────────────────
    /// A system component started up.
    SystemStartup { component: String },
    /// A system component is shutting down.
    SystemShutdown { component: String },
    /// A restart of the current core process was requested.
    SystemRestartRequested { source: String, reason: String },
    /// A component's health status changed.
    HealthChanged {
        component: String,
        healthy: bool,
        message: Option<String>,
    },
    /// A component restart was observed.
    HealthRestarted { component: String },
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
            | Self::SubagentFailed { .. } => "agent",

            Self::MemoryStored { .. } | Self::MemoryRecalled { .. } => "memory",

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
            | Self::ComposioActionExecuted { .. } => "composio",

            Self::TriggerEvaluated { .. }
            | Self::TriggerEscalated { .. }
            | Self::TriggerEscalationFailed { .. } => "triage",

            Self::TreeSummarizerHourCompleted { .. }
            | Self::TreeSummarizerPropagated { .. }
            | Self::TreeSummarizerRebuildCompleted { .. } => "tree_summarizer",

            Self::NotificationIngested { .. } | Self::NotificationTriaged { .. } => "notification",

            Self::SystemStartup { .. }
            | Self::SystemShutdown { .. }
            | Self::SystemRestartRequested { .. }
            | Self::HealthChanged { .. }
            | Self::HealthRestarted { .. } => "system",
        }
    }
}

#[cfg(test)]
#[path = "events_tests.rs"]
mod tests;
