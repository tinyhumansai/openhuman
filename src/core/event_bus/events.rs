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
    CronJobTriggered { job_id: String, job_type: String },
    /// A cron job completed execution.
    CronJobCompleted { job_id: String, success: bool },
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

            Self::SystemStartup { .. }
            | Self::SystemShutdown { .. }
            | Self::SystemRestartRequested { .. }
            | Self::HealthChanged { .. }
            | Self::HealthRestarted { .. } => "system",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_variants_have_correct_domain() {
        let cases: Vec<(DomainEvent, &str)> = vec![
            // Agent
            (
                DomainEvent::AgentTurnStarted {
                    session_id: "s".into(),
                    channel: "c".into(),
                },
                "agent",
            ),
            (
                DomainEvent::AgentTurnCompleted {
                    session_id: "s".into(),
                    text_chars: 0,
                    iterations: 0,
                },
                "agent",
            ),
            (
                DomainEvent::AgentError {
                    session_id: "s".into(),
                    message: "e".into(),
                    recoverable: false,
                },
                "agent",
            ),
            (
                DomainEvent::SubagentSpawned {
                    parent_session: "s".into(),
                    agent_id: "researcher".into(),
                    mode: "typed".into(),
                    task_id: "task-1".into(),
                    prompt_chars: 42,
                },
                "agent",
            ),
            (
                DomainEvent::SubagentCompleted {
                    parent_session: "s".into(),
                    task_id: "task-1".into(),
                    agent_id: "researcher".into(),
                    elapsed_ms: 123,
                    output_chars: 100,
                    iterations: 2,
                },
                "agent",
            ),
            (
                DomainEvent::SubagentFailed {
                    parent_session: "s".into(),
                    task_id: "task-1".into(),
                    agent_id: "researcher".into(),
                    error: "boom".into(),
                },
                "agent",
            ),
            // Memory
            (
                DomainEvent::MemoryStored {
                    key: "k".into(),
                    category: "c".into(),
                    namespace: "n".into(),
                },
                "memory",
            ),
            (
                DomainEvent::MemoryRecalled {
                    query: "q".into(),
                    hit_count: 0,
                },
                "memory",
            ),
            // Channel
            (
                DomainEvent::ChannelInboundMessage {
                    event_name: "telegram:message".into(),
                    channel: "telegram".into(),
                    message: "hi".into(),
                    raw_data: serde_json::Value::Null,
                },
                "channel",
            ),
            (
                DomainEvent::ChannelMessageReceived {
                    channel: "c".into(),
                    message_id: "m1".into(),
                    sender: "s".into(),
                    reply_target: "r".into(),
                    content: "hi".into(),
                    thread_ts: None,
                },
                "channel",
            ),
            (
                DomainEvent::ChannelMessageProcessed {
                    channel: "c".into(),
                    message_id: "m1".into(),
                    sender: "s".into(),
                    reply_target: "r".into(),
                    content: "hi".into(),
                    thread_ts: None,
                    response: "hello".into(),
                    elapsed_ms: 0,
                    success: true,
                },
                "channel",
            ),
            (
                DomainEvent::ChannelReactionReceived {
                    channel: "c".into(),
                    sender: "s".into(),
                    target_message_id: "m1".into(),
                    emoji: "👍".into(),
                },
                "channel",
            ),
            (
                DomainEvent::ChannelReactionSent {
                    channel: "c".into(),
                    target_message_id: "m1".into(),
                    emoji: "✅".into(),
                    success: true,
                },
                "channel",
            ),
            (
                DomainEvent::ChannelConnected {
                    channel: "c".into(),
                },
                "channel",
            ),
            (
                DomainEvent::ChannelDisconnected {
                    channel: "c".into(),
                    reason: "r".into(),
                },
                "channel",
            ),
            // Cron
            (
                DomainEvent::CronJobTriggered {
                    job_id: "j".into(),
                    job_type: "t".into(),
                },
                "cron",
            ),
            (
                DomainEvent::CronJobCompleted {
                    job_id: "j".into(),
                    success: true,
                },
                "cron",
            ),
            (
                DomainEvent::CronDeliveryRequested {
                    job_id: "j".into(),
                    channel: "c".into(),
                    target: "t".into(),
                    output: "o".into(),
                },
                "cron",
            ),
            (
                DomainEvent::ProactiveMessageRequested {
                    source: "cron:morning_briefing".into(),
                    message: "Good morning!".into(),
                    job_name: Some("morning_briefing".into()),
                },
                "cron",
            ),
            // Skill
            (
                DomainEvent::SkillLoaded {
                    skill_id: "s".into(),
                    runtime: "quickjs".into(),
                },
                "skill",
            ),
            (
                DomainEvent::SkillStopped {
                    skill_id: "s".into(),
                },
                "skill",
            ),
            (
                DomainEvent::SkillStartFailed {
                    skill_id: "s".into(),
                    error: "e".into(),
                },
                "skill",
            ),
            (
                DomainEvent::SkillExecuted {
                    skill_id: "s".into(),
                    tool_name: "t".into(),
                    arguments: serde_json::Value::Null,
                    result: None,
                    success: true,
                    elapsed_ms: 0,
                },
                "skill",
            ),
            // Tool
            (
                DomainEvent::ToolExecutionStarted {
                    tool_name: "t".into(),
                    session_id: "s".into(),
                },
                "tool",
            ),
            (
                DomainEvent::ToolExecutionCompleted {
                    tool_name: "t".into(),
                    session_id: "s".into(),
                    success: true,
                    elapsed_ms: 0,
                },
                "tool",
            ),
            // Webhook
            (
                DomainEvent::WebhookIncomingRequest {
                    request: crate::openhuman::webhooks::WebhookRequest {
                        correlation_id: "c".into(),
                        tunnel_id: "t".into(),
                        tunnel_uuid: "u".into(),
                        tunnel_name: "n".into(),
                        method: "GET".into(),
                        path: "/".into(),
                        headers: Default::default(),
                        query: Default::default(),
                        body: String::new(),
                    },
                    raw_data: serde_json::Value::Null,
                },
                "webhook",
            ),
            (
                DomainEvent::WebhookReceived {
                    tunnel_id: "t".into(),
                    skill_id: "s".into(),
                    method: "GET".into(),
                    path: "/".into(),
                    correlation_id: "c".into(),
                },
                "webhook",
            ),
            (
                DomainEvent::WebhookRegistered {
                    tunnel_id: "t".into(),
                    skill_id: "s".into(),
                    tunnel_name: None,
                },
                "webhook",
            ),
            (
                DomainEvent::WebhookUnregistered {
                    tunnel_id: "t".into(),
                    skill_id: "s".into(),
                },
                "webhook",
            ),
            (
                DomainEvent::WebhookProcessed {
                    tunnel_id: "t".into(),
                    skill_id: "s".into(),
                    method: "GET".into(),
                    path: "/".into(),
                    correlation_id: "c".into(),
                    status_code: 200,
                    elapsed_ms: 0,
                    error: None,
                },
                "webhook",
            ),
            // Composio
            (
                DomainEvent::ComposioTriggerReceived {
                    toolkit: "gmail".into(),
                    trigger: "GMAIL_NEW_GMAIL_MESSAGE".into(),
                    metadata_id: "trig-1".into(),
                    metadata_uuid: "uuid-1".into(),
                    payload: serde_json::Value::Null,
                },
                "composio",
            ),
            (
                DomainEvent::ComposioConnectionCreated {
                    toolkit: "gmail".into(),
                    connection_id: "conn-1".into(),
                    connect_url: "https://backend.composio.dev/connect/abc".into(),
                },
                "composio",
            ),
            (
                DomainEvent::ComposioActionExecuted {
                    tool: "GMAIL_SEND_EMAIL".into(),
                    success: true,
                    error: None,
                    cost_usd: 0.0,
                    elapsed_ms: 123,
                },
                "composio",
            ),
            // Triage
            (
                DomainEvent::TriggerEvaluated {
                    source: "composio".into(),
                    external_id: "uuid-1".into(),
                    display_label: "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE".into(),
                    decision: "drop".into(),
                    used_local: false,
                    latency_ms: 12,
                },
                "triage",
            ),
            (
                DomainEvent::TriggerEscalated {
                    source: "composio".into(),
                    external_id: "uuid-1".into(),
                    display_label: "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE".into(),
                    target_agent: "orchestrator".into(),
                },
                "triage",
            ),
            (
                DomainEvent::TriggerEscalationFailed {
                    source: "composio".into(),
                    external_id: "uuid-1".into(),
                    reason: "parser gave up after remote retry".into(),
                },
                "triage",
            ),
            // Tree Summarizer
            (
                DomainEvent::TreeSummarizerHourCompleted {
                    namespace: "n".into(),
                    node_id: "2024/03/15/14".into(),
                    token_count: 500,
                },
                "tree_summarizer",
            ),
            (
                DomainEvent::TreeSummarizerPropagated {
                    namespace: "n".into(),
                    node_id: "2024/03/15".into(),
                    level: "day".into(),
                    token_count: 1000,
                },
                "tree_summarizer",
            ),
            (
                DomainEvent::TreeSummarizerRebuildCompleted {
                    namespace: "n".into(),
                    total_nodes: 10,
                },
                "tree_summarizer",
            ),
            // System
            (
                DomainEvent::SystemStartup {
                    component: "c".into(),
                },
                "system",
            ),
            (
                DomainEvent::SystemShutdown {
                    component: "c".into(),
                },
                "system",
            ),
            (
                DomainEvent::SystemRestartRequested {
                    source: "rpc".into(),
                    reason: "test".into(),
                },
                "system",
            ),
            (
                DomainEvent::HealthChanged {
                    component: "c".into(),
                    healthy: true,
                    message: None,
                },
                "system",
            ),
            (
                DomainEvent::HealthRestarted {
                    component: "c".into(),
                },
                "system",
            ),
        ];

        for (event, expected_domain) in cases {
            assert_eq!(
                event.domain(),
                expected_domain,
                "Wrong domain for {:?}",
                std::mem::discriminant(&event)
            );
        }
    }
}
