//! Agent chat turns: the full tool-using turn and the simple no-tools variant.

use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use crate::inference::provider as providers;
use crate::rpc::RpcOutcome;

use super::turn_guards::{
    effective_agent_chat_origin, enforce_user_prompt_or_reject, grant_turn_cwd,
    normalize_model_override, resolve_turn_cwd,
};

/// Executes a single chat turn with an AI agent.
///
/// This function initializes an agent from the provided configuration and
/// processes the input message.
///
/// # Arguments
///
/// * `config` - The configuration used to build the agent. May be updated with model/temp overrides.
/// * `message` - The user message to process.
/// * `model_override` - Optional model name to use for this call.
/// * `temperature` - Optional sampling temperature override.
/// * `cwd` - Optional per-turn working directory. When present and non-empty the
///   agent's filesystem / shell tools are rooted there for this turn only: a
///   relative path resolves inside it and an absolute path under it is
///   permitted. Absent or empty behaves exactly as before (the configured
///   `action_dir`). The override is applied to a *clone* of the config, so it
///   never leaks into concurrent turns the way a process-global would.
///
/// # Errors
///
/// Returns an error when the prompt is rejected by the injection guard, when
/// `cwd` names something that is not an accessible directory, or when building
/// or running the agent fails.
///
/// # Progress
///
/// If the caller scoped a
/// [`ProgressSink`](crate::agent::progress_sink::ProgressSink) around
/// the awaited future (see [`crate::agent_progress`]), it is attached to the
/// agent built here, so an in-process embedder observes the turn's tool calls
/// and deltas live instead of only its final string.
///
/// # Per-call inference route
///
/// `route` names an endpoint and bearer for this call alone. It is applied to
/// `config` in memory before the agent is built — including before the `cwd`
/// clone below, so a turn that is both rooted and routed gets both — and is
/// never persisted. See
/// [`ephemeral_route`](crate::config::schema::ephemeral_route).
pub async fn agent_chat(
    config: &mut Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
    cwd: Option<String>,
    route: Option<crate::config::schema::EphemeralRoute>,
) -> Result<RpcOutcome<String>, String> {
    agent_chat_for(
        config,
        AgentChatTarget::Orchestrator,
        message,
        model_override,
        temperature,
        thread_id,
        cwd,
        route,
    )
    .await
}

/// Which session [`agent_chat_for`] builds the turn on.
#[derive(Debug, Clone, Copy)]
pub enum AgentChatTarget<'a> {
    /// The orchestrator — [`OpenHumanSessionHost::from_config`], today's `agent_chat`.
    Orchestrator,
    /// Resolve `id` the way every other id-keyed entry point does: the
    /// process registry first, then `config.agent_registry.entries`.
    AgentId(&'a str),
    /// A definition the caller already holds; nothing is resolved by id. The
    /// entry point for a library host running its own per-agent specs — see
    /// [`OpenHumanSessionHost::from_config_with_definition`].
    Definition {
        definition: &'a crate::agent::harness::definition::AgentDefinition,
    },
}

fn build_turn_agent(
    config: &Config,
    target: &AgentChatTarget<'_>,
) -> Result<OpenHumanSessionHost, String> {
    match target {
        AgentChatTarget::Orchestrator => OpenHumanSessionHost::from_config(config),
        AgentChatTarget::AgentId(id) => {
            log::debug!("[inference] agent_chat building agent_id={id}");
            OpenHumanSessionHost::from_config_for_agent(config, id)
        }
        AgentChatTarget::Definition { definition } => {
            OpenHumanSessionHost::from_config_with_definition(config, definition)
        }
    }
    .map_err(|e| e.to_string())
}

/// [`agent_chat`] on an explicit [`AgentChatTarget`].
///
/// Two differences from the historical `agent_chat` beyond the target:
///
/// * A non-empty `thread_id` resumes **that thread's** transcript
///   (`OpenHumanSessionHost::seed_resume_from_thread_transcript`). When the thread has no
///   transcript yet, auto-resume is suppressed for the turn so a fresh thread
///   never splices in the agent's newest transcript from some other thread —
///   `OpenHumanSessionHost::turn` resolves the latest transcript per agent *name*, not per
///   thread.
/// * The agent is built by `target`, so a library host can run one booted
///   core with many independently defined agents.
#[allow(clippy::too_many_arguments)]
pub async fn agent_chat_for(
    config: &mut Config,
    target: AgentChatTarget<'_>,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
    cwd: Option<String>,
    route: Option<crate::config::schema::EphemeralRoute>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(message, "local_ai.ops.agent_chat")?;

    // TAURI-RUST-RS: an upstream caller (frontend, JSON-RPC client) can pass
    // `model_override: Some("")`. See `normalize_model_override` for the
    // rationale — an empty / whitespace-only override collapses to `None`.
    if let Some(model) = normalize_model_override(model_override) {
        config.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        config.default_temperature = temp;
    }
    // After the model override, because the route pins its roles to
    // `"<slug>:<model>"` and the model it uses is the one this call resolved.
    // Before the `cwd` clone below, so a rooted turn is routed too.
    if let Some(route) = route {
        crate::config::schema::ephemeral_route::apply(config, route);
    }
    let turn_cwd = resolve_turn_cwd(cwd)?;
    let mut agent = match turn_cwd.as_ref() {
        // Per-turn root. Building from a config clone whose `action_dir` is the
        // requested directory is what makes this a *turn-scoped* override: the
        // session's `SecurityPolicy`, its tool registry and the builder's
        // `action_dir` are all derived from that one field, so they agree, and
        // nothing process-global is mutated (unlike `live_policy::set_action_dir`,
        // which would race concurrent turns).
        Some(root) => {
            let mut scoped = config.clone();
            scoped.action_dir = root.clone();
            // `action_dir` alone is inert for access control — grant the same
            // root so the turn's tools may actually read and write in it.
            grant_turn_cwd(&mut scoped, root);
            log::debug!(
                "[inference] agent_chat rooting turn tools at cwd={}",
                root.display()
            );
            let mut agent = build_turn_agent(&scoped, &target)?;
            // Also thread it as the turn's workspace descriptor so acting tools
            // that read `ToolExecutionContext::workspace` (shell) resolve their
            // default cwd here, and so spawned sub-agents inherit the same root.
            agent.set_workspace_descriptor(Some(tinytools::WorkspaceDescriptor::new(root.clone())));
            agent
        }
        None => build_turn_agent(config, &target)?,
    };
    // Thread-correct resume. `OpenHumanSessionHost::turn` would otherwise auto-load the
    // newest transcript for the agent *name*, which is another thread's
    // history whenever the same agent serves several threads (every library
    // host does exactly that). Seed from this thread's transcript when it has
    // one; when it has none, keep the turn from falling back to that autoload.
    if let Some(id) = thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
    {
        agent.set_thread_id(Some(id));
        // Scoped to this call's agent identity, when it has one: a library
        // host can hand the same caller-supplied thread_id to several
        // independently configured runtime agents, and unscoped matching
        // would resume whichever agent's transcript for that thread is
        // newest into this one's turn. `Orchestrator` has no such identity
        // and keeps the original unscoped lookup (#5351's cross-profile
        // resume depends on it).
        let resume_agent_id: Option<&str> = match &target {
            AgentChatTarget::Orchestrator => None,
            AgentChatTarget::AgentId(agent_id) => Some(agent_id),
            AgentChatTarget::Definition { definition, .. } => Some(definition.id.as_str()),
        };
        if agent.seed_resume_from_thread_transcript_scoped(id, resume_agent_id) {
            log::debug!("[inference] agent_chat resumed thread transcript thread_id={id}");
        } else {
            log::debug!("[inference] agent_chat fresh thread thread_id={id}; autoload suppressed");
            agent.set_next_turn_overrides(crate::agent::session_host::TurnOverrides {
                suppress_transcript_autoload: true,
                ..Default::default()
            });
        }
    }
    // Live progress for in-process embedders. `OpenHumanSessionHost::from_config` never
    // attaches a sink itself, so there is nothing to clobber here; callers that
    // set one explicitly (web chat, platform socket, flows, skills) hold their
    // own `Agent` and never reach this path — where both could apply, the
    // explicitly-set sink wins because it is applied to the agent it owns.
    if let Some(tx) = crate::agent::progress_sink::current_progress_sink() {
        agent.set_on_progress(Some(tx));
    }
    // Direct `agent_chat` RPC — invoked by trusted clients (desktop UI,
    // operator CLI). Label as CLI so the approval gate doesn't fail
    // closed on an unlabelled call site — *unless* the caller already scoped a
    // more specific origin around this dispatch, which an in-process embedder
    // can do (a workflow node labels its turn `TrustedAutomation::Workflow`).
    // Overwriting that with `Cli` would silently discard the caller's own
    // trust statement and hand every such turn the blanket CLI allowance
    // instead of the narrower one it asked for.
    let run = crate::agent::turn_origin::with_origin(
        effective_agent_chat_origin(),
        agent.run_single(message),
    );
    let response = run.await.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(response, "agent chat completed"))
}

/// A simplified chat interface that does not update the base configuration.
pub async fn agent_chat_simple(
    config: &Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(message, "local_ai.ops.agent_chat_simple")?;

    let mut effective = config.clone();
    // TAURI-RUST-RS: see `normalize_model_override` for the rationale.
    if let Some(model) = normalize_model_override(model_override) {
        effective.default_model = Some(model);
    }
    if let Some(temp) = temperature {
        effective.default_temperature = temp;
    }

    let default_model = effective
        .default_model
        .clone()
        .unwrap_or_else(|| crate::config::DEFAULT_MODEL.to_string());

    let (model, resolved_model): (
        std::sync::Arc<dyn tinyinference_llm::model::ChatModel<()>>,
        String,
    ) = if providers::factory::resolves_to_managed_backend("chat", &effective) {
        let (backend, resolved_model) =
            providers::factory::make_openhuman_backend_model_for_thread(
                "chat",
                &effective,
                &default_model,
                true,
                thread_id.as_deref(),
            )
            .map_err(|e| e.to_string())?;
        (backend, resolved_model)
    } else {
        providers::create_chat_model_with_model_id(
            "chat",
            &effective,
            effective.default_temperature,
        )
        .map_err(|e| e.to_string())?
    };
    tracing::debug!(
        requested_model = %default_model,
        resolved_model = %resolved_model,
        temperature = effective.default_temperature,
        "[inference] agent_chat_simple invoking crate-native chat model"
    );
    let run = model.invoke(
        &(),
        tinyinference_llm::model::ModelRequest::new(vec![
            tinyinference_llm::message::Message::user(message.to_string()),
        ])
        .with_model(default_model.clone())
        .with_temperature(effective.default_temperature),
    );
    let response = run.await.map_err(|e| e.to_string())?.text();

    Ok(RpcOutcome::single_log(
        response,
        "agent simple chat completed",
    ))
}
