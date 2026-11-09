//! Starting a live voice session for a chat thread.
//!
//! The orchestrator's session host supplies the tool surface (the same tools,
//! visibility and policy a typed turn on that agent gets); the live tool
//! harness runs every call through approvals and policy; `tinyagents-live`
//! connects the provider and executes the calls. Tool calls run inside a
//! `voice` external-channel origin and an approval chat context for the
//! thread, so approval prompts reach the user's open conversation.

use crate::core::runtime::DispatchCtx;
use std::sync::Arc;

use tinyagents_harness::context::RunConfig;
use tinyagents_live::{LiveAgent, LiveAgentOptions, LiveAgentSession, TaskScope};

use super::error::LiveVoiceError;
use super::providers;
use crate::agent::session_host::OpenHumanSessionHost;
use crate::agent::tinyagents::host::OpenHumanRunContext;
use crate::agent::tinyagents::live_harness::{assemble_live_tool_harness, LIVE_MAX_TOOL_CALLS};
use crate::config::schema::voice_live::is_live_provider;
use crate::config::Config;
use crate::core::runtime::turn_origin::{with_origin, AgentTurnOrigin};
use crate::security::approval::{ApprovalChatContext, ApprovalTurnContext};
use crate::threads::store::{ConversationMessage, CreateConversationThread};

/// The agent a live session borrows its tools from.
pub(crate) const LIVE_AGENT_ID: &str = "orchestrator";
/// How many recent thread messages are summarised into the prompt.
pub(crate) const CONTEXT_MESSAGES: usize = 12;
/// Characters kept per message in that summary.
pub(crate) const CONTEXT_CHARS: usize = 600;

const BASE_PROMPT: &str = include_str!("prompt.md");

/// What the client asked for.
#[derive(Debug, Clone)]
pub(crate) struct LiveStartRequest {
    pub(crate) provider: Option<String>,
    pub(crate) thread_id: Option<String>,
    pub(crate) client_id: Option<String>,
    pub(crate) input_sample_rate: u32,
}

/// A running session plus what the transport needs to know about it.
pub(crate) struct StartedLiveSession {
    pub(crate) session: LiveAgentSession,
    pub(crate) provider: String,
    pub(crate) thread_id: String,
}

/// The provider a request resolves to.
pub(crate) fn resolve_provider(
    config: &Config,
    requested: Option<&str>,
) -> Result<String, LiveVoiceError> {
    let provider = requested
        .filter(|p| !p.trim().is_empty())
        .unwrap_or(config.voice_live.default_provider.as_str())
        .to_string();
    if is_live_provider(&provider) {
        Ok(provider)
    } else {
        Err(LiveVoiceError::invalid(format!(
            "unknown live provider `{provider}`"
        )))
    }
}

/// The system prompt: the voice guidance plus the thread's recent messages,
/// so a spoken follow-up knows what was just typed.
pub(crate) fn system_prompt(recent: &[ConversationMessage]) -> String {
    let mut prompt = BASE_PROMPT.trim().to_string();
    prompt.push_str(&format!(
        "\n\nThe user's local date and time now: {}.",
        chrono::Local::now().format("%A, %B %-d, %Y, %-I:%M %p")
    ));
    let start = recent.len().saturating_sub(CONTEXT_MESSAGES);
    let lines: Vec<String> = recent[start..]
        .iter()
        .filter(|m| !m.content.trim().is_empty())
        .map(|m| {
            let who = if m.sender == "user" { "User" } else { "You" };
            let text: String = m.content.trim().chars().take(CONTEXT_CHARS).collect();
            format!("{who}: {text}")
        })
        .collect();
    if !lines.is_empty() {
        prompt.push_str("\n\nThe conversation so far (most recent last):\n");
        prompt.push_str(&lines.join("\n"));
    }
    prompt
}

/// Makes sure the session's thread exists, creating a voice thread when the
/// client is not on one.
async fn ensure_thread(
    ctx: &DispatchCtx,
    config: &Config,
    thread_id: Option<String>,
) -> Result<String, LiveVoiceError> {
    let id = thread_id
        .filter(|t| !t.trim().is_empty())
        .unwrap_or_else(|| format!("voice-{}", uuid::Uuid::new_v4()));
    crate::threads::store::blocking::ensure_thread(
        ctx.clone(),
        config.workspace_dir.clone(),
        CreateConversationThread {
            id: id.clone(),
            title: "Voice conversation".to_string(),
            created_at: chrono::Utc::now().to_rfc3339(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
            working_dir: None,
        },
    )
    .await
    .map_err(LiveVoiceError::internal)?;
    Ok(id)
}

/// Starts a live session.
pub(crate) async fn start(
    ctx: &DispatchCtx,
    config: Arc<Config>,
    request: LiveStartRequest,
    session_id: &str,
) -> Result<StartedLiveSession, LiveVoiceError> {
    let provider = resolve_provider(&config, request.provider.as_deref())?;
    let thread_id = ensure_thread(ctx, &config, request.thread_id).await?;
    tracing::debug!(
        session_id,
        provider = %provider,
        thread_id = %thread_id,
        "[voice-live] starting session"
    );

    let mut host = OpenHumanSessionHost::from_config_for_agent(ctx, &config, LIVE_AGENT_ID)
        .await
        .map_err(|e| LiveVoiceError::internal(format!("agent build failed: {e}")))?;
    host.set_event_context(format!("voice_live_{session_id}"), "voice");
    host.set_thread_id(Some(&thread_id));
    let harness = assemble_live_tool_harness(host.live_tool_surface());

    let recent = crate::threads::store::blocking::get_messages(
        ctx.clone(),
        config.workspace_dir.clone(),
        thread_id.clone(),
    )
    .await
    .unwrap_or_else(|error| {
        tracing::warn!(
            session_id,
            "[voice-live] could not load thread context: {error}"
        );
        Vec::new()
    });
    let mut live = providers::live_config(
        &config,
        &provider,
        &system_prompt(&recent),
        request.input_sample_rate,
    );
    // Hosted Gemini fixes its tools when the ticket is minted, so they must be
    // in the configuration before `prepare`.
    live.tools = tinyagents_live::tool_declarations(&harness, false);
    tracing::debug!(
        session_id,
        direct = live.tools.len(),
        deferred = harness.tools().deferred_schemas().len(),
        tools = ?live.tools.iter().map(|t| t.name.as_str()).collect::<Vec<_>>(),
        "[voice-live] tools declared to the live model"
    );
    let prepared = providers::prepare(&config, &provider, live).await?;

    let origin = AgentTurnOrigin::ExternalChannel {
        sender_name: None,
        channel: "voice".to_string(),
        sender: None,
        reply_target: session_id.to_string(),
        message_id: format!("voice-live-{session_id}"),
        history_key: None,
        channel_session: None,
    };
    let approval = ApprovalChatContext {
        thread_id: thread_id.clone(),
        client_id: request
            .client_id
            .unwrap_or_else(|| format!("voice-live-{session_id}")),
        request_id: None,
    };
    let mut context = OpenHumanRunContext::new();
    context.origin = Some(origin.clone());
    // The chat the session's approvals surface in rides the run context, which
    // reaches the gate and every tool the live agent runs.
    context.approval = ApprovalTurnContext::chat(approval);
    context.thread_id = Some(thread_id.clone());
    context.workspace = host.live_workspace_descriptor();
    let run_config =
        RunConfig::new(format!("voice-live-{session_id}")).with_max_tool_calls(LIVE_MAX_TOOL_CALLS);
    let ctx = context.into_tinyagents(run_config);

    // The approval gate still reads the turn origin from a task-local (an
    // unlabelled call is treated as `Unknown` and denied), so the tool worker
    // -- a spawned task -- must be scoped explicitly. Baselined in
    // `agent-runtime-boundary-baseline.json` with the other entry points until
    // the gate reads the origin from the run context.
    let scope: TaskScope = Arc::new(move |task| {
        let origin = origin.clone();
        Box::pin(with_origin(origin, task))
    });
    let agent = LiveAgent::new(Arc::new(harness), Arc::new(())).with_task_scope(scope);
    let session = agent
        .start(
            prepared.provider.as_ref(),
            prepared.config,
            ctx,
            LiveAgentOptions::default(),
        )
        .await?;
    Ok(StartedLiveSession {
        session,
        provider,
        thread_id,
    })
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
