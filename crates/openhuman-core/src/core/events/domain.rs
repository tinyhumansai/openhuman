//! Domain routing for DomainEvent.

use super::DomainEvent;

impl DomainEvent {
    /// Returns the domain name for routing and filtering.
    pub fn domain(&self) -> &'static str {
        match self {
            Self::AgentTurnStarted { .. }
            | Self::AgentTurnCompleted { .. }
            | Self::ConversationTurnCommitted { .. }
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
            | Self::RunQueueFollowupDispatched { .. }
            | Self::RunQueueInterrupted { .. }
            | Self::RunQueueMessageDelivered { .. }
            | Self::RunQueueSteerRequeued { .. } => "agent",

            Self::MonitorStatusChanged { .. } | Self::MonitorLine { .. } => "monitor",

            Self::EmbeddingModelUnhealthy { .. }
            | Self::MemoryStored { .. }
            | Self::MemoryRecalled { .. } => "memory",

            Self::ChannelInboundMessage { .. }
            | Self::ChannelMessageReceived { .. }
            | Self::ChannelMessageProcessed { .. }
            | Self::ChannelReactionReceived { .. }
            | Self::ChannelReactionSent { .. }
            | Self::ChannelConnected { .. }
            | Self::ChannelDisconnected { .. } => "channel",

            Self::CronJobTriggered { .. }
            | Self::CronJobCompleted { .. }
            | Self::ProactiveMessageRequested { .. }
            | Self::CronSystemJobDue { .. }
            | Self::FlowRunProgress { .. }
            | Self::FlowRunStarted { .. }
            | Self::FlowRunFinished { .. }
            | Self::FlowChanged { .. } => "cron",

            Self::WorkflowLoaded { .. }
            | Self::WorkflowStopped { .. }
            | Self::WorkflowStartFailed { .. }
            | Self::WorkflowExecuted { .. }
            | Self::WorkflowsChanged { .. } => "workflow",

            Self::ToolExecutionStarted { .. } | Self::ToolExecutionCompleted { .. } => "tool",

            Self::WorkspacePrepared { .. }
            | Self::WorkspaceViolation { .. }
            | Self::WorkspaceCleanup { .. } => "workspace",

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

            Self::NotificationIngested { .. } | Self::NotificationTriaged { .. } => "notification",

            Self::InboundEventDropped { .. }
            | Self::InboundDeferred { .. }
            | Self::InboundDeadLettered { .. }
            | Self::InboundTurnCompleted { .. }
            | Self::InboundTurnFailed { .. } => "inbound",

            Self::DevicePaired { .. }
            | Self::DeviceRevoked { .. }
            | Self::DevicePeerOnline { .. }
            | Self::DevicePeerOffline { .. }
            | Self::DeviceTunnelFrame { .. } => "device",

            Self::SystemStartup { .. }
            | Self::SystemShutdown { .. }
            | Self::SystemRestartRequested { .. }
            | Self::SystemShutdownRequested { .. }
            | Self::AutonomyConfigChanged
            | Self::AgentPathsChanged
            | Self::ActiveWorkspaceChanged { .. }
            | Self::HealthChanged { .. }
            | Self::HealthRestarted { .. } => "system",

            Self::KeyringConsentRequired
            | Self::KeyringAccessApprovalRequested(..)
            | Self::KeyringDecryptFailed { .. } => "keyring",

            Self::SessionExpired { .. }
            | Self::CredentialChanged { .. }
            | Self::ProviderApiKeyRejected { .. } => "auth",

            Self::TaskSourceFetched { .. }
            | Self::TaskSourceTaskIngested { .. }
            | Self::TaskSourceFetchFailed { .. } => "task_sources",

            Self::ThreadGoalUpdated { .. }
            | Self::ThreadGoalCleared { .. }
            | Self::ThreadTodosChanged { .. }
            | Self::ThreadRunModeChanged { .. } => "agent",

            Self::ApprovalRequested { .. }
            | Self::ApprovalDecided { .. }
            | Self::ApprovalGateOverrideIgnored { .. }
            | Self::ApprovalGateDisabled { .. }
            | Self::FlowApprovalRequested { .. } => "approval",

            Self::PlanReviewRequested { .. } | Self::PlanReviewDecided { .. } => "plan_review",

            Self::ExternalTransferPending { .. } => "egress",

            Self::ArtifactReady { .. }
            | Self::ArtifactFailed { .. }
            | Self::ArtifactPending { .. } => "artifact",

            Self::McpServerInstalled { .. }
            | Self::McpServerConnected { .. }
            | Self::McpServerDisconnected { .. }
            | Self::McpClientToolExecuted { .. }
            | Self::McpToolRejected { .. }
            | Self::McpServerProbeTimedOut { .. }
            | Self::McpServerTransportDropped { .. }
            | Self::McpServerReconnected { .. }
            | Self::McpServerReconnectFailed { .. }
            | Self::McpServerParked { .. } => "mcp_client",
        }
    }
}
