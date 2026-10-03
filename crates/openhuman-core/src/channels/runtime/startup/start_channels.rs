//! The channel runtime's entry point: wiring subscribers, providers, the
//! system prompt, and the message dispatch loop.

use super::super::dispatch::{run_message_dispatch_loop, RuntimeChannelMessage};
use super::super::supervision::spawn_supervised_listener;
use super::chat_workload::{resolve_chat_workload, ChatWorkloadResolution};
use super::credentials::{hydrate_channel_credentials, RuntimeProxyClients};
use super::prompt::format_access_context;
use super::relay::start_relay_runtime;
use crate::agent::host_runtime;
use crate::channels::context::{
    effective_channel_message_timeout_secs, ChannelRuntimeContext,
    DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS, DEFAULT_CHANNEL_MAX_BACKOFF_SECS,
};
use crate::channels::system_prompt::{ChannelPromptInputs, ChannelSystemPrompt};
use crate::channels::traits;
use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::inference::provider;
use crate::security::SecurityPolicy;
use crate::tools;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tinychannels::runtime::compute_max_in_flight_messages;
use tokio_util::task::AbortOnDropHandle;

pub async fn start_channels(config: Config) -> Result<()> {
    start_channels_with_session(config, super::super::session::channel_session()).await
}

pub(crate) async fn start_channels_with_session(
    config: Config,
    session: tokio_util::sync::CancellationToken,
) -> Result<()> {
    super::super::session::run_in_session(session, start_channels_inner(config)).await
}

async fn start_channels_inner(mut config: Config) -> Result<()> {
    // Initialize the global event bus singleton and register the tracing
    // subscriber for debug logging of all domain events.
    crate::core::bus::init().await.expect("bus init");
    let bus = crate::core::bus::BUS.get().expect("bus initialised");
    let _tracing_handle = bus.subscribe(Arc::new(crate::core::bus::TracingSubscriber));
    crate::platform::health::bus::register_health_subscriber();
    crate::threads::store::register_conversation_persistence_subscriber(
        config.workspace_dir.clone(),
    );
    crate::integrations::composio::register_composio_trigger_subscriber();
    // Surface parked ApprovalGate requests as chat messages so the user can
    // answer yes/no in the thread (chat-native approval, issue #1339).
    crate::web_chat::register_approval_surface_subscriber();
    // Surface generated-artifact lifecycle events (ArtifactReady /
    // ArtifactFailed) as `artifact_ready` / `artifact_failed` web-channel
    // events so the frontend ArtifactCard can render in chat (#2779).
    crate::web_chat::register_artifact_surface_subscriber();
    // Surface external-egress disclosure events (ExternalTransferPending) as
    // `external_transfer_pending` web-channel events so the frontend can show a
    // per-action "what leaves, to where, why" card (privacy epic S2, #4436).
    crate::web_chat::register_egress_surface_subscriber();
    // Surface thread-goal / thread-todo / run-queue lifecycle events
    // (ThreadGoalUpdated/Cleared, ThreadTodosChanged, RunQueue*) as
    // `thread_goal_updated`/`thread_goal_cleared`/`thread_todos_changed`/
    // `queue_item_queued`/`queue_item_delivered` web-channel events so the
    // desktop goal chip, todo drawer, and message-queue UI stay live (C3).
    crate::web_chat::register_agent_surface_subscriber();
    // Surface memory store/recall activity (MemoryStored/MemoryRecalled) as
    // `memory_activity` web-channel events, routed to the turn's own
    // thread/client only (C5) — never carries memory content or the raw
    // recall query, only a short clipped preview.
    crate::web_chat::register_memory_activity_surface_subscriber();
    // Task-sources: subscribe to Composio connection-created events for
    // one-shot fetches, and spawn the periodic poll that pulls work from
    // configured external sources onto the agent's todo board.
    crate::integrations::task_sources::bus::register_task_sources_subscriber();
    crate::integrations::task_sources::start_periodic_poll();
    // Native request handlers. Re-registering is safe (latest wins) so
    // this is idempotent even if `bootstrap_core_runtime` also runs.
    // Must happen before `run_message_dispatch_loop` begins, because
    // channel dispatch calls `BUS.native().request("agent.run_turn", …)`
    // for every inbound message.
    crate::agent::bus::register_agent_handlers();
    // The Phase 2/3/4 self-improvement subscribers (email-signature producer,
    // rebuild trigger, ProfileMdRenderer) are registered in
    // core::runtime::subscribers::register_domain_subscribers instead. start_channels is
    // skipped when no channel is configured, so wiring them here silently
    // dropped user-profile inference for channel-less users (#5003).

    tracing::debug!("[event_bus] global singleton initialized in start_channels");

    // Initialise the sub-agent definition registry from this workspace.
    // Idempotent — `bootstrap_core_runtime` may also call it.
    if let Err(err) =
        crate::agent::harness::AgentDefinitionRegistry::init_global(&config.workspace_dir)
    {
        tracing::warn!(
            "AgentDefinitionRegistry::init_global failed: {err} — \
             spawn_subagent will be unavailable until restart"
        );
    }
    // Note: WebhookRequestSubscriber and ChannelInboundSubscriber are registered
    // in bootstrap_core_runtime() (crates/openhuman-core/src/core/runtime/bootstrap.rs) to avoid double-registration
    // when both startup paths run in the same process.

    let provider_runtime_options = provider::ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };
    let (model, provider_name) = match resolve_chat_workload(&config) {
        ChatWorkloadResolution::Cloud => {
            let (_chat, model) = provider::create_chat_model_with_model_id(
                "chat",
                &config,
                config.default_temperature,
            )?;
            (model, provider::INFERENCE_BACKEND_ID.to_string())
        }
        ChatWorkloadResolution::Workload {
            provider_string,
            slug,
        } => {
            tracing::info!(
                chat_provider = %provider_string,
                slug = %slug,
                "[channels][startup] chat workload routed to per-workload provider — building dedicated channel provider"
            );
            let (_chat, model_id) = provider::create_chat_model_with_model_id(
                "chat",
                &config,
                config.default_temperature,
            )?;
            (model_id, slug)
        }
    };

    let runtime: Arc<dyn host_runtime::RuntimeAdapter> = Arc::from(host_runtime::create_runtime(
        &config.runtime,
        config.shell.hide_window,
    )?);
    // Create the agent's action sandbox + default projects home and register the
    // projects dir as a ReadWrite trusted root. Shared with the always-run
    // `bootstrap_core_runtime` boot so a fresh install gets these dirs even with
    // no messaging integrations connected (#3353, RC-A).
    crate::config::ensure_agent_dirs(&mut config).await;
    // Install as the process-global live policy so runtime autonomy changes
    // (config.update_autonomy_settings) are reflected by `live_policy::current()`
    // and picked up by the next session.
    let security = crate::security::live_policy::install(
        Arc::new(
            SecurityPolicy::from_config(
                &config.autonomy,
                &config.workspace_dir,
                &config.action_dir,
            )
            .with_privacy_mode(config.privacy.mode),
        ),
        config.workspace_dir.clone(),
        config.action_dir.clone(),
    );
    // NOTE: the live tool-execution timeout seed is done in
    // `core::runtime::subscribers::register_domain_subscribers` (unconditional core boot), NOT
    // here — `start_channels` is skipped when no channel is configured or
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` is set, which would otherwise leave
    // channel-less / web-chat-only cores running the default timeout instead of the
    // user-configured `[agent].agent_timeout_secs` (#5027).
    // Phase 1 of #1401: audit logger is wired with defaults so emission paths
    // are exercised at runtime. A follow-up promotes `SecurityConfig` (and
    // therefore the `audit` knob) onto the runtime `Config` schema so users
    // can override `enabled`, `log_path`, and `max_size_mb` via TOML. The
    // logger is workspace-scoped and shared, so concurrent sessions append to
    // one `audit.log` without racing on rotation.
    let audit = crate::security::get_or_create_workspace_audit_logger(
        crate::config::AuditConfig::default(),
        config.workspace_dir.clone(),
    )?;
    let temperature = config.default_temperature;
    // Build system prompt from workspace identity files + skills
    let workspace = config.workspace_dir.clone();
    let tools_registry = Arc::new(tools::ops::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        audit,
        // `all_tools_with_runtime` no longer takes a memory handle — the two
        // tools that needed one resolve the guarded driver per call.
        &config.browser,
        &config.http_request,
        &config.action_dir,
        &config.agents,
        &config,
        None,
    ));

    let skills = crate::skills::load_workflow_metadata(&workspace);

    // Install the triggered-workflow subscriber now that workflows are
    // discovered — otherwise any workflow declaring `triggers:` is silently
    // ignored. Idempotent + shares a process-global OnceLock with the
    // `bootstrap_core_runtime` site, so it registers exactly once regardless of
    // which startup path runs first (web-chat-only cores never reach here).
    crate::skills::bus::ensure_triggered_workflow_subscriber(&workspace);

    // Collect tool descriptions for the prompt
    let mut tool_descs: Vec<(&str, &str)> = vec![
        (
            "shell",
            "Execute terminal commands. Use when: running local checks, build/test commands, diagnostics. Don't use when: a safer dedicated tool exists, or command is destructive without approval.",
        ),
        (
            "file_read",
            "Read file contents. Use when: inspecting project files, configs, logs. Don't use when: a targeted search is enough.",
        ),
        (
            "file_write",
            "Write file contents. Use when: applying focused edits, scaffolding files, updating docs/code. Don't use when: side effects are unclear or file ownership is uncertain.",
        ),
        (
            "memory_store",
            "Save to memory. Use when: preserving durable preferences, decisions, key context. Don't use when: information is transient/noisy/sensitive without need.",
        ),
        (
            "memory_recall",
            "Search memory. Use when: retrieving prior decisions, user preferences, historical context. Don't use when: answer is already in current context.",
        ),
        (
            "memory_forget",
            "Delete a memory entry. Use when: memory is incorrect/stale or explicitly requested for removal. Don't use when: impact is uncertain.",
        ),
    ];

    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open approved HTTPS URLs in Brave Browser (allowlist-only, no scraping)",
        ));
    }
    // Composio tool descriptions are intentionally excluded from the main
    // agent prompt — integration actions are reached through tool search.
    tool_descs.push((
        "schedule",
        "Manage scheduled tasks (create/list/get/cancel/pause/resume). Supports recurring cron and one-shot delays.",
    ));
    tool_descs.push((
        "pushover",
        "Send a Pushover notification to your device. Requires PUSHOVER_TOKEN and PUSHOVER_USER_KEY in .env file.",
    ));
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a subtask to a specialized agent. Use when: a task benefits from a different model (e.g. fast summarization, deep reasoning, code generation). The sub-agent runs a single prompt and returns its response.",
        ));
    }

    let bootstrap_max_chars = if config.agent.compact_context {
        Some(6000)
    } else {
        None
    };
    // Filter out Workflow-category tools (e.g. Composio, Apify) from the
    // main agent prompt — integration actions are reached through tool search.
    let non_skill_tools: Vec<&Box<dyn tinytools::Tool>> = tools_registry
        .iter()
        .filter(|t| t.category() != tinytools::ToolCategory::Workflow)
        .collect();
    // Everything after the rendered prompt is fixed for the process: the
    // tool-instruction block, then the model's current filesystem access
    // boundaries so it self-limits (advisory only — the SecurityPolicy
    // enforces these regardless).
    let non_skill_specs: Vec<tinytools::ToolSpec> =
        non_skill_tools.iter().map(|tool| tool.spec()).collect();
    let mut prompt_suffix = tinytools_agent::dialect::XmlDialect::instructions(&non_skill_specs);
    prompt_suffix.push_str(&format_access_context(&security));
    // The prompt itself is rendered here for the current identity and
    // re-rendered whenever the active profile or an identity file changes
    // (#6027, #6028). `channel_name = None`: the runtime wires up multiple
    // providers in parallel, so the capability block keeps its
    // platform-agnostic "messaging bot" phrasing.
    let system_prompt = ChannelSystemPrompt::refreshing(ChannelPromptInputs {
        workspace_dir: workspace.clone(),
        model: model.clone(),
        tool_descs: tool_descs
            .iter()
            .map(|(name, desc)| ((*name).to_string(), (*desc).to_string()))
            .collect(),
        skills: skills.clone(),
        bootstrap_max_chars,
        suffix: prompt_suffix,
    });

    if !skills.is_empty() {
        println!(
            "  🧩 Skills:   {}",
            skills
                .iter()
                .map(|s| s.name.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        );
    }

    // Assemble the ChannelHost capability surface (shutdown, STT/TTS, reaction
    // gate, approvals, conversation store, event sink). Ported rich providers
    // reach host capabilities through this instead of calling core internals.
    let channel_host = crate::channels::host::build_channel_host(Arc::new(config.clone()));

    // Provider construction lives in `tinychannels::factory` so that this host
    // and the `tinychannels-module` cdylib build the same providers from the
    // same config. It used to be ~200 lines inline here; a second copy is how
    // the two drift, and only one of them would have been the one under test.
    //
    // Three things stay on this side because they are host policy, and the
    // factory's docs say so explicitly:
    //   - credential hydration (below) reads OpenHuman's keyring,
    //   - `RuntimeProxyClients` applies the configured HTTP proxy,
    //   - `channel_host` is the capability surface assembled just above.
    let channels = tinychannels::build_channels(
        &hydrate_channel_credentials(&config),
        &channel_host,
        &RuntimeProxyClients,
    );

    let relay_config = config
        .channels_config
        .relay
        .clone()
        .filter(tinychannels::config::RelayRuntimeConfig::is_listener_configured);

    if channels.is_empty() && relay_config.is_none() {
        println!("No channels configured. Set up channels in the web UI.");
        return Ok(());
    }

    println!("🦀 OpenHuman Channel Server");
    println!("  🤖 Model:    {model}");
    println!(
        "  🧠 Memory:   {}",
        if crate::memory::memory_is_on(&config) {
            config.memory.engine.as_str()
        } else {
            "off"
        }
    );
    println!(
        "  📡 Channels: {}",
        channels
            .iter()
            .map(|c| c.name())
            .chain(relay_config.as_ref().map(|_| "relay"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();
    println!("  Listening for messages... (Ctrl+C to stop)");
    println!();

    BUS.publish(DomainEvent::SystemStartup {
        component: "channels".into(),
    });

    let initial_backoff_secs = config
        .reliability
        .channel_initial_backoff_secs
        .max(DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS);
    let max_backoff_secs = config
        .reliability
        .channel_max_backoff_secs
        .max(DEFAULT_CHANNEL_MAX_BACKOFF_SECS);

    // Providers still publish legacy `ChannelMessage`s through the public
    // channel trait. The runtime dispatch queue wraps those messages so relay
    // inbound can carry its original TinyChannels envelope through processing.
    let (provider_tx, mut provider_rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(100);
    let (dispatch_tx, rx) = tokio::sync::mpsc::channel::<RuntimeChannelMessage>(100);
    let provider_dispatch_tx = dispatch_tx.clone();
    let provider_bridge = AbortOnDropHandle::new(tokio::spawn(async move {
        while let Some(msg) = provider_rx.recv().await {
            if provider_dispatch_tx
                .send(RuntimeChannelMessage::from(msg))
                .await
                .is_err()
            {
                break;
            }
        }
    }));

    let mut relay_handles = Vec::new();
    if let Some(ref relay) = relay_config {
        match start_relay_runtime(relay, dispatch_tx.clone()).await {
            Ok(handle) => relay_handles.push(handle),
            Err(error) => {
                tracing::warn!("[channels][relay] failed to start relay runtime: {error}")
            }
        }
    }

    // Spawn a listener for each channel
    let mut handles = Vec::new();
    for ch in &channels {
        handles.push(AbortOnDropHandle::new(spawn_supervised_listener(
            ch.clone(),
            provider_tx.clone(),
            initial_backoff_secs,
            max_backoff_secs,
        )));
    }
    drop(provider_tx); // Drop our copy so provider_rx closes when all channels stop.
    drop(dispatch_tx); // Drop startup's copy; relay/bridge clones keep dispatch alive.

    let channels_by_name = Arc::new(
        channels
            .iter()
            .map(|ch| (ch.name().to_string(), Arc::clone(ch)))
            .collect::<HashMap<_, _>>(),
    );
    // Register the cron delivery subscriber so cron jobs can deliver output
    // to channels via events instead of directly constructing channel instances.
    let _cron_delivery_handle = bus.subscribe(Arc::new(
        crate::cron::bus::CronDeliverySubscriber::new(Arc::clone(&channels_by_name)),
    ));
    // NOTE: the flows `FlowTriggerSubscriber` is registered in
    // `runtime/subscribers.rs::register_domain_subscribers` (unconditional core boot), NOT
    // here — `start_channels` is skipped when no channel is configured or
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` is set, which would otherwise leave
    // schedule/app-event workflows undispatched (issue B2 review).
    // Register the proactive message subscriber so morning briefings,
    // welcome messages, and other proactive agent output gets routed to
    // the user's active channel (+ always to web).
    let proactive_sub = crate::channels::proactive::ProactiveMessageSubscriber::new(
        Arc::clone(&channels_by_name),
        config.channels_config.active_channel.clone(),
    );
    // Expose its active-channel handle so the `channels_set_default` RPC can
    // switch the default channel at runtime without a restart (issue #3712).
    crate::channels::proactive::register_active_channel_handle(
        proactive_sub.active_channel_handle(),
    );
    let _proactive_handle = bus.subscribe(Arc::new(proactive_sub));
    // Remote-control turn state (`/status` shows "in progress") for every
    // channel with the `remote_control` capability.
    let _turn_state_handle = bus.subscribe(Arc::new(
        crate::channels::host::ChannelTurnStateSubscriber::new(config.workspace_dir.clone()),
    ));
    // In-chat approvals (sub-issue 2 of #3098) for every channel with the
    // `chat_approvals` capability: `Prompt`-class tool calls are gated for the
    // user instead of silently allowed. The dispatch loop pairs this by scoping
    // each such turn in an `ApprovalChatContext` and intercepting `yes`/`no`
    // replies for parked approvals.
    let _approval_surface_handle = bus.subscribe(Arc::new(
        crate::channels::host::ChannelApprovalSurfaceSubscriber::new(Arc::clone(&channels_by_name)),
    ));
    tracing::debug!("[channels] registered turn-state and approval-surface subscribers");

    let listener_count = channels.len() + relay_config.as_ref().map(|_| 1).unwrap_or_default();
    let max_in_flight_messages = compute_max_in_flight_messages(listener_count);

    println!("  🚦 In-flight message limit: {max_in_flight_messages}");

    let message_timeout_secs =
        effective_channel_message_timeout_secs(config.channels_config.message_timeout_secs);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name,
        turn_model_source: None,
        default_provider: Arc::new(provider_name),
        tools_registry: Arc::clone(&tools_registry),
        system_prompt,
        model: Arc::new(model.clone()),
        temperature,
        max_tool_iterations: config.agent.max_tool_iterations,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        turn_model_source_cache: Arc::new(Mutex::new(HashMap::new())),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_url: config.api_url.clone(),
        inference_url: config.inference_url.clone(),
        reliability: Arc::new(config.reliability.clone()),
        provider_runtime_options,
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        message_timeout_secs,
        multimodal: config.multimodal.clone(),
        multimodal_files: config.multimodal_files.clone(),
        // Crate-native turn models for the channel turn (Phase 3 P3-B).
        config: Some(std::sync::Arc::new(config.clone())),
    });

    run_message_dispatch_loop(rx, runtime_ctx, max_in_flight_messages).await;

    // Wait for all channel tasks
    for h in handles {
        let _ = h.await;
    }
    let _ = provider_bridge.await;

    Ok(())
}
