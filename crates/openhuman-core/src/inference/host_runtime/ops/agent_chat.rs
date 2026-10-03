//! Agent chat turns: the full tool-using turn and the simple no-tools variant.

use crate::agent::OpenHumanSessionHost;
use crate::config::Config;
use crate::core::Outcome;
use crate::inference::provider as providers;

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
) -> Result<Outcome<String>, String> {
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

/// The log line every successful agent chat turn carries.
const AGENT_CHAT_COMPLETED_LOG: &str = "agent chat completed";

/// What one agent chat turn produced: its reply and whether it stopped at the
/// tool-iteration cap rather than finishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentChatReply {
    /// The reply text. When `hit_cap` is set this is the turn's checkpoint.
    pub text: String,
    /// The turn paused at its iteration cap (see
    /// [`OpenHumanSessionHost::last_turn_hit_cap`]).
    pub hit_cap: bool,
}

impl AgentChatReply {
    /// The historical `Outcome<String>` shape, for library callers.
    pub fn into_outcome(self) -> Outcome<String> {
        Outcome::single_log(self.text, AGENT_CHAT_COMPLETED_LOG)
    }

    /// The `inference.agent_chat` wire shape: the historical
    /// `{"result": <text>, "logs": [...]}` envelope plus `hit_cap`, and, when
    /// the turn was capped, `checkpoint` (the same text, named for what it is).
    ///
    /// Additive on purpose: existing clients keep reading the reply from
    /// `result` (directly or through `unwrap_rpc`), and a headless caller can
    /// now tell a capped turn from a finished one and continue it (#6958).
    pub fn into_rpc_json(self) -> Result<serde_json::Value, String> {
        let hit_cap = self.hit_cap;
        let checkpoint = hit_cap.then(|| self.text.clone());
        let mut value = self.into_outcome().into_cli_compatible_json()?;
        let serde_json::Value::Object(map) = &mut value else {
            return Err("agent chat envelope is not a JSON object".to_string());
        };
        map.insert("hit_cap".to_string(), serde_json::Value::Bool(hit_cap));
        if let Some(checkpoint) = checkpoint {
            map.insert(
                "checkpoint".to_string(),
                serde_json::Value::String(checkpoint),
            );
        }
        Ok(value)
    }
}

/// Which session [`agent_chat_for`] builds the turn on.
#[derive(Clone, Copy)]
pub enum AgentChatTarget<'a> {
    /// The orchestrator — [`OpenHumanSessionHost::from_config`], today's `agent_chat`.
    Orchestrator,
    /// Resolve `id` the way every other id-keyed entry point does: the
    /// process registry first, then `config.agent_registry.entries`.
    AgentId(&'a str),
    /// A definition the caller already holds; nothing is resolved by id. The
    /// entry point for a library host running its own per-agent specs — see
    /// [`OpenHumanSessionHost::from_config_with_definition`].
    ///
    /// `host` carries the caller's own `dyn Tool` objects. It rides the target
    /// rather than `agent_chat_for`'s argument list because only this target
    /// can honour it: the other two resolve a definition the caller does not
    /// hold, so there is no agent for a host belt to belong to.
    Definition {
        definition: &'a crate::agent::harness::definition::AgentDefinition,
        host: Option<&'a crate::agent::HostTools>,
        /// History to seed this turn with, as `(role, content)` rows, instead
        /// of whatever the session would otherwise resume.
        ///
        /// Rides the target for the same reason `host` does, and it is the
        /// per-turn half of the same idea: a host whose history lives in its
        /// own log -- a journal, a board, an episode -- is the only thing that
        /// can say what this turn should have seen. Seeding is how that view
        /// reaches the session with roles intact; passing it as prose in the
        /// message would flatten the host's own prior turns into quoted text.
        ///
        /// `None` leaves resume untouched, which is every existing caller.
        seed: Option<&'a [(String, String)]>,
        /// Where to report what the turn spent.
        ///
        /// An out-parameter because this is the only target that can fill it:
        /// the turn runs in-process here, so the session that counted the
        /// tokens is still in hand when it ends. The other two answer over
        /// `AGENT_CHAT`, whose reply is a string.
        ///
        /// Written whether the turn succeeded or failed -- a turn that ended
        /// badly still spent what it spent, and a host that meters only
        /// successes bills nothing for the ones that cost most.
        usage: Option<&'a std::sync::Mutex<Option<crate::agent::tinyagents::host::LastTurnUsage>>>,
    },
}

// Hand-written because a host belt is a closure, and a closure is not `Debug`.
// Reporting whether one is present is what a log line here is ever for; the
// belt it would return is not known until the turn builds it.
impl std::fmt::Debug for AgentChatTarget<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Orchestrator => f.write_str("Orchestrator"),
            Self::AgentId(id) => f.debug_tuple("AgentId").field(id).finish(),
            Self::Definition {
                definition,
                host,
                seed,
                usage,
            } => f
                .debug_struct("Definition")
                .field("definition", &definition.id)
                .field("host_tools", &host.is_some())
                .field("seed_rows", &seed.map_or(0, <[(String, String)]>::len))
                .field("meters", &usage.is_some())
                .finish(),
        }
    }
}

fn build_turn_agent(
    config: &Config,
    target: &AgentChatTarget<'_>,
    session_id: Option<&str>,
) -> Result<OpenHumanSessionHost, String> {
    match target {
        AgentChatTarget::Orchestrator => OpenHumanSessionHost::from_config(config),
        AgentChatTarget::AgentId(id) => {
            log::debug!("[inference] agent_chat building agent_id={id}");
            OpenHumanSessionHost::from_config_for_agent(config, id)
        }
        AgentChatTarget::Definition {
            definition, host, ..
        } => match host {
            Some(host) => OpenHumanSessionHost::from_config_with_host_tools(
                config, definition, host, session_id,
            ),
            None => OpenHumanSessionHost::from_config_with_definition(config, definition),
        },
    }
    .map_err(|e| e.to_string())
}

/// [`agent_chat`] on an explicit [`AgentChatTarget`].
///
/// Two differences from the historical `agent_chat` beyond the target:
///
/// * A non-empty `thread_id` binds the session's durable identity, so the turn
///   resumes **that conversation's** transcript exactly
///   (`ResumeMode::Session`). A fresh thread simply has nothing to resume; it
///   can no longer fall back to the agent's newest transcript from some other
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
) -> Result<Outcome<String>, String> {
    agent_chat_reply_for(
        config,
        target,
        message,
        model_override,
        temperature,
        thread_id,
        cwd,
        route,
    )
    .await
    .map(AgentChatReply::into_outcome)
}

/// [`agent_chat_for`], keeping whether the turn stopped at its iteration cap.
/// The `inference.agent_chat` controller answers from this.
#[allow(clippy::too_many_arguments)]
pub async fn agent_chat_reply_for(
    config: &mut Config,
    target: AgentChatTarget<'_>,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
    cwd: Option<String>,
    route: Option<crate::config::schema::EphemeralRoute>,
) -> Result<AgentChatReply, String> {
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
    // The conversation this turn runs in, normalised once: the factory below is
    // told the same id `set_thread_id` will bind, so a belt keyed on the
    // session cannot disagree with the session it is built for.
    let turn_session_id = thread_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
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
            let mut agent = build_turn_agent(&scoped, &target, turn_session_id)?;
            // Also thread it as the turn's workspace descriptor so acting tools
            // that read `ToolExecutionContext::workspace` (shell) resolve their
            // default cwd here, and so spawned sub-agents inherit the same root.
            agent.set_workspace_descriptor(Some(tinytools::WorkspaceDescriptor::new(root.clone())));
            agent
        }
        None => build_turn_agent(config, &target, turn_session_id)?,
    };
    // Thread-correct resume. `OpenHumanSessionHost::turn` would otherwise auto-load the
    // newest transcript for the agent *name*, which is another thread's
    // history whenever the same agent serves several threads (every library
    // host does exactly that). Seed from this thread's transcript when it has
    // one; when it has none, keep the turn from falling back to that autoload.
    if let Some(id) = turn_session_id {
        // Binding the thread also binds the session's durable identity, and
        // the turn resumes by that identity. Nothing here has to seed history
        // by hand, suppress an autoload, or reason about which transcript is
        // newest: one conversation resolves to one transcript, scoped to this
        // agent so a caller-supplied thread id shared by several runtime
        // agents cannot splice one agent's history into another's turn.
        agent.set_thread_id(Some(id));
        log::debug!("[inference] agent_chat bound session for thread_id={id}");
    }
    // A seeded turn replaces resume rather than adding to it.
    //
    // The three calls are one operation and the order is load-bearing:
    // `clear_history` drops the composed session so `seed_resume_from_messages`
    // -- which returns `Ok(())` and seeds NOTHING on a live one, so getting
    // this order wrong runs the turn blind rather than failing -- can take
    // effect, and the override
    // stops the runtime reloading from its own durable transcript the history
    // that was just replaced. A caller seeding from its own log means that log
    // to be the whole of what this turn has seen; leaving either of the other
    // two off would quietly reunite it with a second source.
    if let AgentChatTarget::Definition {
        seed: Some(seed), ..
    } = target
    {
        agent.clear_history();
        agent
            .seed_resume_from_messages(seed.to_vec(), message)
            .map_err(|e| e.to_string())?;
        agent.set_next_turn_overrides(crate::agent::session_host::TurnOverrides {
            suppress_transcript_autoload: true,
            ..Default::default()
        });
        log::debug!(
            "[inference] agent_chat seeded {} row(s); transcript autoload suppressed",
            seed.len()
        );
    }
    // Live progress for in-process embedders. `OpenHumanSessionHost::from_config` never
    // attaches a sink itself, so there is nothing to clobber here; callers that
    // set one explicitly (web chat, platform socket, flows, skills) hold their
    // own `Agent` and never reach this path — where both could apply, the
    // explicitly-set sink wins because it is applied to the agent it owns.
    if let Some(tx) = crate::agent::progress_sink::current_progress_sink() {
        agent.set_on_progress(Some(tx));
    } else {
        // The turn runner streams model calls only when a progress sink is
        // attached (`streaming = on_progress.is_some()`). With none, a bare RPC
        // turn fell back to unary `invoke`, so time-to-first-token equalled the
        // full completion latency. Attach a drain-only sink so the call streams;
        // the task ends when the agent drops its sender.
        let (tx, mut rx) = tokio::sync::mpsc::channel::<crate::agent::progress::AgentProgress>(256);
        tokio::spawn(async move { while rx.recv().await.is_some() {} });
        agent.set_on_progress(Some(tx));
        log::debug!(
            "[inference] agent_chat no progress sink; attached drain sink to stream model calls"
        );
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
    let outcome = run.await;
    // Before the `?`. A turn that failed still spent what it spent, and the
    // session that counted it is about to go out of scope with the error.
    if let AgentChatTarget::Definition {
        usage: Some(sink), ..
    } = target
    {
        *sink
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = agent.last_turn_usage();
    }
    let text = outcome.map_err(|e| e.to_string())?;
    let hit_cap = agent.last_turn_hit_cap();
    if hit_cap {
        log::info!("[inference] agent_chat turn paused at its iteration cap; reporting hit_cap");
    }
    Ok(AgentChatReply { text, hit_cap })
}

/// A simplified chat interface that does not update the base configuration.
pub async fn agent_chat_simple(
    config: &Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
) -> Result<Outcome<String>, String> {
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

    Ok(Outcome::single_log(response, "agent simple chat completed"))
}

#[cfg(test)]
#[path = "agent_chat_tests.rs"]
mod tests;
