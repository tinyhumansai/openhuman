use chrono::Utc;

use crate::openhuman::agent::Agent;
use crate::openhuman::config::Config;
use crate::openhuman::inference::local as local_ai;
use crate::openhuman::inference::provider as providers;
use crate::openhuman::inference::{
    LocalAiAssetsStatus, LocalAiDownloadsProgress, LocalAiEmbeddingResult, LocalAiSpeechResult,
    LocalAiStatus, LocalAiTtsResult,
};
use crate::openhuman::security::prompt_injection::{
    enforce_prompt_input, PromptEnforcementAction, PromptEnforcementContext,
};
use crate::rpc::RpcOutcome;

fn prompt_guard_user_message(action: PromptEnforcementAction) -> &'static str {
    match action {
        PromptEnforcementAction::Allow => "Message accepted.",
        PromptEnforcementAction::Blocked => {
            "Prompt blocked by security policy. Please rephrase without instruction overrides or exfiltration requests."
        }
        PromptEnforcementAction::ReviewBlocked => {
            "Prompt flagged for security review and was not processed. Please rephrase clearly."
        }
    }
}

/// Normalize a `model_override` string into the `Option<String>` form the
/// downstream config-resolution path expects.
///
/// `None` → `None`. `Some(non-empty-after-trim)` → `Some(trimmed)`. Anything
/// else (`Some("")`, `Some("   ")`, `Some("\t\n")`) collapses to `None` so
/// the existing default-model fallback applies instead of overwriting
/// `config.default_model` with a blank string that the OpenHuman backend
/// would reject with `400 model is required` (Sentry TAURI-RUST-RS).
///
/// Extracted to keep `agent_chat` and `agent_chat_simple` in lockstep —
/// future tweaks (additional log lines, tightening the trim rules) live in
/// exactly one place.
fn normalize_model_override(opt: Option<String>) -> Option<String> {
    opt.and_then(|m| {
        let t = m.trim();
        if t.is_empty() {
            None
        } else {
            Some(t.to_string())
        }
    })
}

fn enforce_user_prompt_or_reject(prompt: &str, source: &'static str) -> Result<(), String> {
    let decision = enforce_prompt_input(
        prompt,
        PromptEnforcementContext {
            source,
            request_id: None,
            user_id: None,
            session_id: Some("local_ai"),
        },
    );
    match decision.action {
        PromptEnforcementAction::Allow => Ok(()),
        PromptEnforcementAction::Blocked | PromptEnforcementAction::ReviewBlocked => {
            Err(prompt_guard_user_message(decision.action).to_string())
        }
    }
}

/// Resolve the per-turn `cwd` parameter into the directory the turn's tools
/// should be rooted at.
///
/// `None` / empty / whitespace-only collapses to `None`, which keeps the turn on
/// the configured `action_dir` exactly as before. A present value must name an
/// existing directory: rooting an agent at a path that does not exist would give
/// it a cwd every shell and file call fails against, so this rejects loudly
/// instead. The path is canonicalized so symlinked and `..`-containing inputs
/// compare equal to the paths the security policy derives from it.
fn resolve_turn_cwd(cwd: Option<String>) -> Result<Option<std::path::PathBuf>, String> {
    let Some(raw) = cwd else { return Ok(None) };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let path = std::path::Path::new(trimmed);
    let canonical = std::fs::canonicalize(path)
        .map_err(|e| format!("cwd '{trimmed}' is not accessible: {e}"))?;
    if !canonical.is_dir() {
        return Err(format!("cwd '{trimmed}' is not a directory"));
    }
    Ok(Some(canonical))
}

/// Grant `root` as a `ReadWrite` trusted root on a per-turn config clone.
///
/// Setting `action_dir` alone only moves where a *relative* path resolves; the
/// allow/deny decision reads `workspace_dir` + `trusted_roots`, so without this
/// grant an absolute path in `root` is refused by `workspace_only` and a
/// relative one is refused as "escapes workspace". Idempotent: an entry the
/// user already configured for the same path is left untouched so a `Read`-only
/// grant is not silently widened by the presence of a `cwd`.
///
/// Only ever called on a *clone* of the config, on the `cwd`-present branch —
/// the no-`cwd` path is untouched, and nothing process-global is mutated.
fn grant_turn_cwd(config: &mut Config, root: &std::path::Path) {
    let path = root.to_string_lossy().to_string();
    if config.autonomy.trusted_roots.iter().any(|r| r.path == path) {
        return;
    }
    config
        .autonomy
        .trusted_roots
        .push(crate::openhuman::security::TrustedRoot {
            path,
            access: crate::openhuman::security::TrustedAccess::ReadWrite,
        });
}

/// The origin label [`agent_chat`] scopes around its turn.
///
/// An ambient origin — scoped by an in-process embedder around its
/// `invoke("openhuman.inference_agent_chat", …)` — is the caller's own,
/// deliberate trust statement about the turn and is kept. Absent one,
/// [`AgentTurnOrigin::DirectChat`] applies: this RPC is reached by trusted
/// clients (the desktop Settings agent-chat panel, an operator running the RPC
/// by hand), and leaving it unlabelled would fail the approval gate closed on
/// every external-effect tool.
///
/// `DirectChat` rather than the historical `Cli`, and the difference is not
/// cosmetic. The approval gate treats the two identically, so trust is
/// unchanged — but `message` here is something a *person* typed, and `Cli`
/// answers "who wrote this" with a trust answer. Its own documentation covers
/// sub-agent and internal invocations, so
/// [`is_user_authored`](crate::openhuman::agent::turn_origin::AgentTurnOrigin::is_user_authored)
/// reads `false` for it and the conversation autosave would silently drop a
/// real user message.
fn effective_agent_chat_origin() -> crate::openhuman::agent::turn_origin::AgentTurnOrigin {
    crate::openhuman::agent::turn_origin::current()
        .unwrap_or(crate::openhuman::agent::turn_origin::AgentTurnOrigin::DirectChat)
}

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
/// [`ProgressSink`](crate::openhuman::agent::progress_sink::ProgressSink) around
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
/// [`ephemeral_route`](crate::openhuman::config::schema::ephemeral_route).
pub async fn agent_chat(
    config: &mut Config,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    thread_id: Option<String>,
    cwd: Option<String>,
    route: Option<crate::openhuman::config::schema::EphemeralRoute>,
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
        crate::openhuman::config::schema::ephemeral_route::apply(config, route);
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
            let mut agent = Agent::from_config(&scoped).map_err(|e| e.to_string())?;
            // Also thread it as the turn's workspace descriptor so acting tools
            // that read `ToolExecutionContext::workspace` (shell) resolve their
            // default cwd here, and so spawned sub-agents inherit the same root.
            agent.set_workspace_descriptor(Some(
                tinyagents_harness::workspace::WorkspaceDescriptor::new(root.clone()),
            ));
            agent
        }
        None => Agent::from_config(config).map_err(|e| e.to_string())?,
    };
    // Live progress for in-process embedders. `Agent::from_config` never
    // attaches a sink itself, so there is nothing to clobber here; callers that
    // set one explicitly (web chat, platform socket, flows, skills) hold their
    // own `Agent` and never reach this path — where both could apply, the
    // explicitly-set sink wins because it is applied to the agent it owns.
    if let Some(tx) = crate::openhuman::agent::progress_sink::current_progress_sink() {
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
    let run = crate::openhuman::agent::turn_origin::with_origin(
        effective_agent_chat_origin(),
        agent.run_single(message),
    );
    let response = match thread_id.as_deref() {
        Some(id) if !id.trim().is_empty() => {
            log::debug!("[inference] agent_chat routing with thread_id={id}");
            crate::openhuman::agent::tinyagents::thread_context::with_thread_id(id, run).await
        }
        _ => {
            log::debug!("[inference] agent_chat routing without thread_id");
            run.await
        }
    }
    .map_err(|e| e.to_string())?;
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
        .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.to_string());

    let (model, resolved_model) = providers::create_chat_model_with_model_id(
        "chat",
        &effective,
        effective.default_temperature,
    )
    .map_err(|e| e.to_string())?;
    tracing::debug!(
        requested_model = %default_model,
        resolved_model = %resolved_model,
        temperature = effective.default_temperature,
        "[inference] agent_chat_simple invoking crate-native chat model"
    );
    let run = model.invoke(
        &(),
        tinyinference::model::ModelRequest::new(vec![
            tinyinference::message::Message::user(message.to_string()),
        ])
        .with_model(default_model.clone())
        .with_temperature(effective.default_temperature),
    );
    let response = match thread_id.as_deref() {
        Some(id) if !id.trim().is_empty() => {
            log::debug!("[inference] agent_chat_simple routing with thread_id={id}");
            crate::openhuman::agent::tinyagents::thread_context::with_thread_id(id, run).await
        }
        _ => {
            log::debug!("[inference] agent_chat_simple routing without thread_id");
            run.await
        }
    }
    .map_err(|e| e.to_string())?
    .text();

    Ok(RpcOutcome::single_log(
        response,
        "agent simple chat completed",
    ))
}

/// Returns the current operational status of the local AI stack.
pub async fn local_ai_status(config: &Config) -> Result<RpcOutcome<LocalAiStatus>, String> {
    let service = local_ai::global(config);
    let status = service.status();
    if matches!(status.state.as_str(), "idle" | "degraded") {
        let service_clone = service.clone();
        let config_clone = config.clone();
        tokio::spawn(async move {
            service_clone.bootstrap(&config_clone).await;
        });
    }
    // `LocalAiService` is a process-wide singleton whose cached `provider`
    // field was set at first init from whichever config it saw. After an
    // `inference_update_local_settings` call that swaps providers
    // (e.g. ollama → lm_studio) the cached value is stale, so we overlay
    // the current config's provider on the status snapshot before returning.
    let mut snapshot = service.status();
    snapshot.provider = local_ai::provider::provider_from_config(config)
        .as_str()
        .to_string();
    Ok(RpcOutcome::single_log(snapshot, "local ai status fetched"))
}

/// Generates a summary of the provided text using local AI models.
pub async fn local_ai_summarize(
    config: &Config,
    text: &str,
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(text.trim(), "local_ai.ops.local_ai_summarize")?;

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let summary = service
        .summarize_interactive(config, text, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        summary,
        "local ai summarize completed",
    ))
}

/// Executes a raw prompt directly against the local AI model.
pub async fn local_ai_prompt(
    config: &Config,
    prompt: &str,
    max_tokens: Option<u32>,
    no_think: Option<bool>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(prompt.trim(), "local_ai.ops.local_ai_prompt")?;

    let service = local_ai::global(config);
    let status = service.status();
    if !matches!(status.state.as_str(), "ready") {
        service.bootstrap(config).await;
    }
    let output = service
        .prompt_interactive(config, prompt.trim(), max_tokens, no_think.unwrap_or(true))
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(output, "local ai prompt completed"))
}

/// Executes a multimodal (vision) prompt with associated images.
pub async fn local_ai_vision_prompt(
    config: &Config,
    prompt: &str,
    image_refs: &[String],
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    enforce_user_prompt_or_reject(prompt.trim(), "local_ai.ops.local_ai_vision_prompt")?;

    let service = local_ai::global(config);
    let output = service
        .vision_prompt(config, prompt.trim(), image_refs, max_tokens)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai vision prompt completed",
    ))
}

/// Generates semantic embeddings for the provided input strings.
pub async fn local_ai_embed(
    config: &Config,
    inputs: &[String],
) -> Result<RpcOutcome<LocalAiEmbeddingResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .embed(config, inputs)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai embedding completed",
    ))
}

/// Transcribes the audio file at the specified path.
pub async fn local_ai_transcribe(
    config: &Config,
    audio_path: &str,
) -> Result<RpcOutcome<LocalAiSpeechResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .transcribe(config, audio_path.trim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

/// Transcribes raw audio bytes by first saving them to a temporary file.
pub async fn local_ai_transcribe_bytes(
    config: &Config,
    audio_bytes: &[u8],
    extension: Option<String>,
) -> Result<RpcOutcome<LocalAiSpeechResult>, String> {
    let service = local_ai::global(config);

    let ext = extension
        .unwrap_or_else(|| "webm".to_string())
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();
    if ext.is_empty() || !ext.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("Invalid audio extension".to_string());
    }

    let voice_dir = std::env::temp_dir().join("openhuman_voice_input");
    tokio::fs::create_dir_all(&voice_dir)
        .await
        .map_err(|e| format!("Failed to create voice input directory: {e}"))?;

    let filename = format!(
        "voice-{}-{}.{}",
        Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4(),
        ext
    );
    let file_path = voice_dir.join(filename);
    tokio::fs::write(&file_path, audio_bytes)
        .await
        .map_err(|e| format!("Failed to write audio file: {e}"))?;

    let output = service
        .transcribe(config, file_path.to_string_lossy().as_ref())
        .await;
    let _ = tokio::fs::remove_file(&file_path).await;

    let output = output.map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai transcription completed",
    ))
}

/// Performs text-to-speech synthesis and optionally saves the result to a file.
pub async fn local_ai_tts(
    config: &Config,
    text: &str,
    output_path: Option<&str>,
) -> Result<RpcOutcome<LocalAiTtsResult>, String> {
    let service = local_ai::global(config);
    let output = service
        .tts(config, text.trim(), output_path)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(output, "local ai tts completed"))
}

/// Returns the status of all local AI assets (models and support files).
pub async fn local_ai_assets_status(
    config: &Config,
) -> Result<RpcOutcome<LocalAiAssetsStatus>, String> {
    let service = local_ai::global(config);
    let output = service
        .assets_status(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai assets status fetched",
    ))
}

/// Returns progress for any ongoing asset downloads.
pub async fn local_ai_downloads_progress(
    config: &Config,
) -> Result<RpcOutcome<LocalAiDownloadsProgress>, String> {
    let service = local_ai::global(config);
    let output = service
        .downloads_progress(config)
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai downloads progress fetched",
    ))
}

/// Triggers the download of a specific AI asset based on capability name.
pub async fn local_ai_download_asset(
    config: &Config,
    capability: &str,
) -> Result<RpcOutcome<LocalAiAssetsStatus>, String> {
    let service = local_ai::global(config);
    let output = service
        .download_asset(config, capability.trim())
        .await
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(
        output,
        "local ai asset download triggered",
    ))
}

/// A single message in a local AI chat conversation.
#[derive(Debug, serde::Deserialize)]
pub struct LocalAiChatMessage {
    /// The role of the message sender (e.g., "user", "assistant").
    pub role: String,
    /// The text content of the message.
    pub content: String,
}

/// Executes a multi-turn chat conversation using the local model.
pub async fn local_ai_chat(
    config: &Config,
    messages: Vec<LocalAiChatMessage>,
    max_tokens: Option<u32>,
) -> Result<RpcOutcome<String>, String> {
    tracing::debug!(
        message_count = messages.len(),
        "[local_ai:chat] local_ai_chat op: validating"
    );

    if messages.is_empty() {
        return Err("messages must not be empty".to_string());
    }

    let mut ollama_messages: Vec<crate::openhuman::inference::local::ollama::OllamaChatMessage> =
        Vec::with_capacity(messages.len());

    for msg in messages.into_iter() {
        let normalized_role = msg.role.trim().to_ascii_lowercase();
        match normalized_role.as_str() {
            "user" => {
                enforce_user_prompt_or_reject(msg.content.as_str(), "local_ai.ops.local_ai_chat")?;
            }
            "system" | "assistant" => {}
            _ => {
                return Err(format!(
                    "unsupported message role: '{}'; expected one of: user, system, assistant",
                    msg.role.trim()
                ));
            }
        }

        ollama_messages.push(
            crate::openhuman::inference::local::ollama::OllamaChatMessage {
                role: normalized_role,
                content: msg.content,
            },
        );
    }

    let service = local_ai::global(config);
    let reply = service
        .chat_with_history_interactive(config, ollama_messages, max_tokens)
        .await?;

    tracing::debug!(
        reply_len = reply.len(),
        "[local_ai:chat] local_ai_chat op: done"
    );
    Ok(RpcOutcome::single_log(reply, "local ai chat completed"))
}
/// Result of the reaction-decision prompt.
#[derive(Debug, serde::Serialize)]
pub struct ReactionDecision {
    /// Whether the model thinks a reaction is appropriate.
    pub should_react: bool,
    /// The emoji to use (only meaningful when `should_react` is true).
    pub emoji: Option<String>,
}
