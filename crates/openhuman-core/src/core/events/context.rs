//! Context accessors shared by event consumers.

use super::DomainEvent;

impl DomainEvent {
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
            Self::ChannelMessageReceived { channel, .. }
            | Self::ChannelConnected { channel, .. }
            | Self::ChannelDisconnected { channel, .. } => Some(channel.as_str()),
            Self::ToolExecutionStarted { tool_name, .. }
            | Self::ToolExecutionCompleted { tool_name, .. } => Some(tool_name.as_str()),
            Self::WorkspacePrepared { policy_id, .. }
            | Self::WorkspaceCleanup { policy_id, .. } => Some(policy_id.as_str()),
            Self::WorkspaceViolation { path } => Some(path.as_str()),
            Self::RunQueueMessageQueued { thread_id, .. }
            | Self::RunQueueFollowupDispatched { thread_id, .. }
            | Self::RunQueueInterrupted { thread_id, .. }
            | Self::RunQueueMessageDelivered { thread_id, .. }
            | Self::RunQueueSteerRequeued { thread_id, .. } => Some(thread_id.as_str()),
            Self::MonitorStatusChanged { thread_id, .. } | Self::MonitorLine { thread_id, .. } => {
                thread_id.as_deref()
            }
            // The Event Log's "agent" column is the only per-row context the
            // stream carries, so an MCP row names its server there: the
            // install id for the RPC-driven lifecycle, the registry name
            // (`ac.inference.sh/mcp`) for the supervisor's verdicts, which is
            // how a user knows the server and how the log lines name it.
            Self::McpServerInstalled { server_id, .. }
            | Self::McpServerConnected { server_id, .. }
            | Self::McpServerDisconnected { server_id, .. } => Some(server_id.as_str()),
            Self::McpServerProbeTimedOut { qualified_name, .. }
            | Self::McpServerTransportDropped { qualified_name, .. }
            | Self::McpServerReconnected { qualified_name, .. }
            | Self::McpServerReconnectFailed { qualified_name, .. }
            | Self::McpServerParked { qualified_name, .. } => Some(qualified_name.as_str()),
            _ => None,
        }
    }

    /// The workspace this event belongs to, for the variants bound to one.
    ///
    /// One process serves more than one workspace over its life, so a
    /// process-wide consumer — the Event Log, the `core_notification`
    /// broadcast — has to be able to ask which workspace a given event came
    /// from. Three families already carry the answer and each used to be read
    /// by whoever happened to need it: `desktop::notifications` matched the
    /// MCP arms in a private helper of its own, and silently did not gate the
    /// channel and artifact families that carry the same field. This is the
    /// one place that question is answered, so a consumer cannot cover a
    /// subset by accident and a new workspace-bound variant has one arm to
    /// add rather than several to find (#5966).
    ///
    /// `None` means "not workspace-bound", which is the common case and is
    /// **not** the same as "unknown workspace": a consumer filtering by
    /// workspace should let these through rather than drop them.
    ///
    /// The value is an absolute path under the user's home directory. It is
    /// for in-process comparison only — anything that reaches a client or an
    /// export sends
    /// [`workspace_handle`](crate::config::workspace_handle)
    /// instead.
    #[must_use]
    pub fn workspace_dir(&self) -> Option<&std::path::Path> {
        match self {
            // Channel — the workspace the inbound message was received under.
            Self::ChannelMessageReceived { workspace_dir, .. }
            | Self::ChannelMessageProcessed { workspace_dir, .. } => Some(workspace_dir.as_path()),

            // Artifact — carried as a `String` because the producer takes it
            // from a config field it already has as text; same meaning.
            //
            // An empty string reads back as "bound to the empty path", which
            // matches no workspace and would therefore hide the event from
            // every scoped consumer — a worse failure than not knowing. Today
            // the producers pass a real `Path`, so this is a guard against a
            // future one that does not, not a case that occurs.
            Self::ArtifactReady { workspace_dir, .. }
            | Self::ArtifactFailed { workspace_dir, .. }
            | Self::ArtifactPending { workspace_dir, .. } => {
                if workspace_dir.is_empty() {
                    None
                } else {
                    Some(std::path::Path::new(workspace_dir))
                }
            }

            // MCP reconnect supervisor (#5931).
            Self::McpServerProbeTimedOut { workspace_dir, .. }
            | Self::McpServerTransportDropped { workspace_dir, .. }
            | Self::McpServerReconnected { workspace_dir, .. }
            | Self::McpServerReconnectFailed { workspace_dir, .. }
            | Self::McpServerParked { workspace_dir, .. } => Some(workspace_dir.as_path()),

            // The switch itself is *about* a workspace rather than bound to
            // one: it is what tells a consumer the active workspace changed,
            // so filtering it out by the very rule it announces would hide
            // the announcement from everyone who is not already up to date.
            Self::ActiveWorkspaceChanged { .. } => None,

            _ => None,
        }
    }
}
