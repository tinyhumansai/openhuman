//! The channel runtime's entry point: wiring subscribers, providers, the
//! system prompt, and the message dispatch loop.

use super::super::dispatch::run_message_dispatch_loop;
use super::module_providers::ModuleChannelRuntime;
use super::turn_parts::{build_channel_turn_parts, runtime_context, PromptToolDescs};
use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::security::SecurityPolicy;
use anyhow::Result;
use std::sync::{Arc, OnceLock, Weak};
use tokio_util::sync::CancellationToken;

pub async fn start_channels(config: Config) -> Result<()> {
    start_channels_with_session(config, super::super::session::channel_session()).await
}

pub(crate) async fn start_channels_with_session(
    config: Config,
    session: tokio_util::sync::CancellationToken,
) -> Result<()> {
    start_channels_inner(config, session).await
}

async fn start_channels_inner(mut config: Config, session: CancellationToken) -> Result<()> {
    if session.is_cancelled() {
        return Ok(());
    }
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
    if let Err(err) = crate::agent::harness::host_definition::AgentDefinitionRegistry::init_global(
        &config.workspace_dir,
    ) {
        tracing::warn!(
            "AgentDefinitionRegistry::init_global failed: {err} — \
             spawn_subagent will be unavailable until restart"
        );
    }
    // Note: WebhookRequestSubscriber and ChannelInboundSubscriber are registered
    // in bootstrap_core_runtime() (crates/openhuman-core/src/core/runtime/bootstrap.rs) to avoid double-registration
    // when both startup paths run in the same process.

    // Create the agent's action sandbox + default projects home and register the
    // projects dir as a ReadWrite trusted root. Shared with the always-run
    // `bootstrap_core_runtime` boot so a fresh install gets these dirs even with
    // no messaging integrations connected (#3353, RC-A).
    crate::config::ensure_agent_dirs(&mut config).await;
    // Install as the process-global live policy so runtime autonomy changes
    // (config.update_autonomy_settings) are reflected by `live_policy::current()`
    // and picked up by the next session.
    let security = crate::security::live_policy::install(
        Arc::new(SecurityPolicy::from_full_config(&config).with_privacy_mode(config.privacy.mode)),
        config.workspace_dir.clone(),
        config.action_dir.clone(),
    );
    // NOTE: the live tool-execution timeout seed is done in
    // `core::runtime::subscribers::register_domain_subscribers` (unconditional core boot), NOT
    // here — `start_channels` is skipped when no channel is configured or
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` is set, which would otherwise leave
    // channel-less / web-chat-only cores running the default timeout instead of the
    // user-configured `[agent].agent_timeout_secs` (#5027).
    // Model, tools and the identity-refreshing system prompt (`turn_parts`),
    // shared with the hosted-channel relay.
    let parts = build_channel_turn_parts(&config, &security, PromptToolDescs::Fixed)?;
    let model = parts.model.clone();
    let workspace = config.workspace_dir.clone();
    let skills = parts.skills.clone();

    // Install the triggered-workflow subscriber now that workflows are
    // discovered — otherwise any workflow declaring `triggers:` is silently
    // ignored. Idempotent + shares a process-global OnceLock with the
    // `bootstrap_core_runtime` site, so it registers exactly once regardless of
    // which startup path runs first (web-chat-only cores never reach here).
    crate::skills::bus::ensure_triggered_workflow_subscriber(&workspace);

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

    let configured = crate::channels::configured_providers(&config);
    let relay_configured = config
        .channels_config
        .relay
        .as_ref()
        .is_some_and(|relay| !relay.url.is_empty() && !relay.identities.is_empty());
    if configured.is_empty() && !relay_configured {
        println!("No channels configured. Set up channels in the web UI.");
        return Ok(());
    }

    // The module runtime captures one config identity and host callback object.
    // A weak context cell is filled once the turn context is ready, avoiding a
    // callback -> context -> channel -> lease -> callback reference cycle.
    let (dispatch_tx, rx) = tokio::sync::mpsc::channel(100);
    let context_slot: Arc<OnceLock<Weak<crate::channels::context::ChannelRuntimeContext>>> =
        Arc::new(OnceLock::new());
    let preparing = ModuleChannelRuntime::prepare(
        Arc::new(config.clone()),
        Arc::clone(&context_slot),
        dispatch_tx.clone(),
    );
    tokio::pin!(preparing);
    let mut module_runtime = tokio::select! {
        biased;
        _ = session.cancelled() => {
            // Do not drop preparation after it may have acquired a module
            // lease. Finish it and await the runtime's acknowledged close
            // before the service supervisor can admit another identity.
            let mut runtime = preparing.await?;
            runtime.close().await?;
            return Ok(());
        },
        prepared = &mut preparing => {
            Some(prepared?)
        }
    };
    let channels_by_name = module_runtime
        .as_ref()
        .expect("runtime prepared when providers are configured")
        .channels();
    let channels = channels_by_name.values().cloned().collect::<Vec<_>>();

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
            .chain(relay_configured.then_some("relay"))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!();
    println!("  Listening for messages... (Ctrl+C to stop)");
    println!();

    BUS.publish(DomainEvent::SystemStartup {
        component: "channels".into(),
    });

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
        crate::channels::host::ChannelTurnStateSubscriber::with_config(Arc::new(config.clone())),
    ));
    // In-chat approvals (sub-issue 2 of #3098) for every channel with the
    // `chat_approvals` capability: `Prompt`-class tool calls are gated for the
    // user instead of silently allowed. The dispatch loop pairs this by scoping
    // each such turn in an `ApprovalChatContext` and intercepting `yes`/`no`
    // replies for parked approvals.
    let _approval_surface_handle = bus.subscribe(Arc::new(
        crate::channels::host::ChannelApprovalSurfaceSubscriber::with_config(
            Arc::clone(&channels_by_name),
            Arc::new(config.clone()),
        ),
    ));
    tracing::debug!("[channels] registered turn-state and approval-surface subscribers");

    let listener_count = channels.len() + usize::from(relay_configured);
    let (message_timeout_secs, max_in_flight_messages) = module_runtime
        .as_ref()
        .expect("runtime prepared when providers are configured")
        .runtime_limits(config.channels_config.message_timeout_secs, listener_count)
        .await?;

    println!("  🚦 In-flight message limit: {max_in_flight_messages}");

    let runtime_ctx = Arc::new(runtime_context(
        &config,
        parts,
        Arc::clone(&channels_by_name),
        message_timeout_secs,
    ));
    context_slot
        .set(Arc::downgrade(&runtime_ctx))
        .map_err(|_| anyhow::anyhow!("channel callback context was already initialized"))?;
    // Let cron jobs created from a channel chat read that chat's history, send
    // into it and append their replies to it.
    crate::cron::channel_bridge::register_channel_bridge(
        Arc::clone(&runtime_ctx.conversation_histories),
        Arc::clone(&runtime_ctx.channels_by_name),
        crate::channels::context::MAX_CHANNEL_HISTORY,
    );

    let startup = module_runtime
        .as_mut()
        .expect("runtime prepared when providers are configured")
        .start(&config);
    tokio::select! {
        biased;
        _ = session.cancelled() => {},
        result = startup => result?,
    }

    tokio::select! {
        biased;
        _ = session.cancelled() => {},
        _ = run_message_dispatch_loop(rx, runtime_ctx, max_in_flight_messages) => {},
    }
    if let Some(runtime) = module_runtime.as_mut() {
        runtime.close().await?;
    }

    Ok(())
}
