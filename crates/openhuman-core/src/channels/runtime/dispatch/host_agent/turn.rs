//! Running one channel turn as a host agent.

use tokio::sync::mpsc;

use super::posture;
use crate::agent::bus::AgentTurnResponse;
use crate::agent::host_agents::HostAgent;
use crate::agent::progress::AgentProgress;
use crate::agent::turn_origin::AgentTurnOrigin;
use tinyagents_session::transcript::TranscriptMessage;

/// One channel message, as a host agent's turn needs it.
pub(crate) struct HostChannelTurn {
    /// Durable thread supplied explicitly by a hosted relay.
    pub(crate) relay_thread: Option<String>,
    /// The channel the message came in on (`"telegram"`).
    pub(crate) channel: String,
    /// The chat's history key: the session the host belt is told about and
    /// the event session id.
    pub(crate) history_key: String,
    /// This chat's prior turns, `(role, content)`, oldest first. The channel
    /// owns them, as it does for the orchestrator, so the session is seeded
    /// from them rather than from a durable transcript.
    pub(crate) prior_turns: Vec<(String, String)>,
    /// The message the agent answers.
    pub(crate) message: String,
    /// Who asked: the channel's `ExternalChannel` origin.
    pub(crate) origin: AgentTurnOrigin,
    /// Live progress for the channel's draft updates.
    pub(crate) progress: Option<mpsc::Sender<AgentProgress>>,
    /// Where this turn's parked approvals surface; default for a channel
    /// without an approval surface.
    pub(crate) approval: crate::security::approval::ApprovalTurnContext,
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
        history_key,
        prior_turns,
        message,
        origin,
        progress,
        approval,
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
    let model = config.default_model.clone().unwrap_or_default();
    tracing::info!(
        channel = %channel,
        agent_id = %agent_id,
        prior_turns = prior_turns.len(),
        "[channels::dispatch::host_agent] running channel turn as host agent"
    );

    host.context = host
        .context
        .derive_channel_turn(relay_thread.clone(), Some(origin.clone()));
    let mut session = host.session_host(
        &config,
        Some(relay_thread.as_deref().unwrap_or(&history_key)),
    )?;
    session.set_thread_id(relay_thread.as_deref());
    // The channel names the session, which also selects the channel's
    // permission ceiling in the session's tool policy.
    session.set_event_context(history_key, channel);
    // The chat's history is the channel's, so the session holds exactly that
    // and never resumes a transcript of its own.
    session.clear_history();
    session.seed_resume_from_messages(prior_turns, &message)?;
    session.set_next_turn_overrides(crate::agent::TurnOverrides {
        suppress_transcript_autoload: true,
        ..Default::default()
    });
    session.set_on_progress(progress);

    let text = host
        .scope(session.run_single_with_approval(&message, Some(origin), approval))
        .await?;
    Ok(AgentTurnResponse::with_resolved_route(
        text,
        format!("agent:{agent_id}"),
        model,
    ))
}

/// The prior turns of a channel `history` (`[system, ..prior, user]`) as seed
/// rows: the channel's own system prompt and the new message are dropped, the
/// agent brings its own prompt and the message is the turn.
pub(crate) fn seed_rows(history: &[TranscriptMessage]) -> Vec<(String, String)> {
    let end = history.len().saturating_sub(1);
    history
        .get(1..end)
        .unwrap_or_default()
        .iter()
        .map(|row| (row.role.clone(), row.content.clone()))
        .collect()
}
