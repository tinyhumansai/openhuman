//! Running one channel turn as a host agent.

use tinyagents_harness::run_queue::RunQueue;
use tokio::sync::mpsc;

use super::posture;
use crate::agent::bus::AgentTurnResponse;
use crate::agent::host_agents::HostAgent;
use crate::progress::AgentProgress;
use crate::security::turn_origin::AgentTurnOrigin;

/// One channel message, as a host agent's turn needs it.
pub(crate) struct HostChannelTurn {
    /// Existing trusted relay correlation used by scoped tools.
    pub(crate) relay_thread: Option<String>,
    /// The channel the message came in on (`"telegram"`).
    pub(crate) channel: String,
    /// TinyChannels' canonical conversation identity, tenant-scoped by the
    /// host before it reaches this runner.
    pub(crate) session_key: String,
    /// Stable event/approval identity retained by the existing channel
    /// surface. It is attribution, not the transcript key.
    pub(crate) event_session_id: String,
    /// The message the agent answers.
    pub(crate) message: String,
    /// Scoped typed media input for this exact queued channel request.
    pub(crate) message_input: Option<tinyinference_llm::message::Message>,
    /// Exact staged native refs issued by the host for this user turn.
    pub(crate) staged_parts: Vec<tinychannels_bus::ContentPart>,
    pub(crate) staged_lease: Option<crate::threads::attachments::NativeStagedLease>,
    pub(crate) staged_message_id: Option<String>,
    /// Who asked: the channel's `ExternalChannel` origin.
    pub(crate) origin: AgentTurnOrigin,
    /// Live progress projected through the compiled channel module.
    pub(crate) progress: Option<mpsc::Sender<AgentProgress>>,
    pub(crate) run_queue: Option<std::sync::Arc<RunQueue<crate::agent::queued_turn::QueuedTurn>>>,
    /// Where this turn's parked approvals surface; default for a channel
    /// without an approval surface.
    pub(crate) approval: crate::security::approval::ApprovalTurnContext,
    pub(crate) cancellation: tinyagents_harness::CancellationToken,
    pub(crate) turn_deadline: Option<std::time::Duration>,
    /// Legacy SaaS relay history to adopt into the durable session once.
    pub(crate) imported_history: Vec<tinyagents_session::transcript::TranscriptMessage>,
    pub(crate) legacy_history: Option<crate::channels::providers::relay::LegacyHistoryImport>,
}

/// Run `turn` as `host`: its definition and host tools, inside its context,
/// under `turn.origin` and the posture that origin calls for.
pub(crate) async fn run_host_agent_turn(
    mut host: HostAgent,
    turn: HostChannelTurn,
) -> anyhow::Result<AgentTurnResponse> {
    let HostChannelTurn {
        relay_thread,
        channel,
        session_key,
        event_session_id,
        message,
        message_input,
        staged_parts,
        staged_lease,
        staged_message_id,
        origin,
        progress,
        run_queue,
        approval,
        cancellation,
        turn_deadline,
        imported_history,
        legacy_history,
    } = turn;
    let mut config = host.config.clone();
    let profile_ceiling = host
        .context
        .host_overrides()
        .and_then(|host| host.profile_capabilities())
        .map(|caps| caps.channel_permission(&channel));
    if let Some(ceiling) =
        profile_ceiling.filter(|level| *level > tinytools::PermissionLevel::ReadOnly)
    {
        posture::cap_channel(&mut config, &channel, ceiling);
    } else if let Some(ceiling) = posture::tool_ceiling(&origin) {
        posture::cap_channel(&mut config, &channel, ceiling);
        host.host_tools = host
            .host_tools
            .take()
            .map(|tools| posture::guard_host_tools(tools, ceiling));
    }
    let agent_id = host.definition.id.clone();
    tracing::info!(
        channel = %channel,
        agent_id = %agent_id,
        "[channels::dispatch::host_agent] running channel turn as host agent"
    );
    host.config = config;
    host.context = host
        .context
        .derive_channel_turn(relay_thread, Some(origin.clone()));
    let model = host.config.default_model.clone().unwrap_or_default();
    let text = host
        .run_external_channel_turn(
            &session_key,
            &event_session_id,
            &channel,
            &message,
            message_input,
            origin,
            approval,
            progress,
            run_queue,
            cancellation,
            turn_deadline,
            &imported_history,
            legacy_history,
            staged_parts,
            staged_lease,
            staged_message_id,
        )
        .await?;
    Ok(AgentTurnResponse::with_resolved_route(
        text,
        format!("agent:{agent_id}"),
        model,
    ))
}
