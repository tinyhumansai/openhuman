//! Channel startup wiring.

use super::dispatch::{run_message_dispatch_loop, RuntimeChannelMessage};
use super::supervision::{compute_max_in_flight_messages, spawn_supervised_listener};
use crate::core::event_bus::{self, DomainEvent, TracingSubscriber, DEFAULT_CAPACITY};
use crate::openhuman::agent::harness::build_tool_instructions_filtered;
use crate::openhuman::agent::host_runtime;
use crate::openhuman::channels::context::{
    effective_channel_message_timeout_secs, ChannelRuntimeContext,
    DEFAULT_CHANNEL_INITIAL_BACKOFF_SECS, DEFAULT_CHANNEL_MAX_BACKOFF_SECS,
};
use crate::openhuman::channels::dingtalk::DingTalkChannel;
use crate::openhuman::channels::discord::DiscordChannel;
use crate::openhuman::channels::email_channel::EmailChannel;
use crate::openhuman::channels::imessage::IMessageChannel;
use crate::openhuman::channels::irc;
use crate::openhuman::channels::irc::IrcChannel;
use crate::openhuman::channels::lark::LarkChannel;
use crate::openhuman::channels::linq::LinqChannel;
use crate::openhuman::channels::mattermost::MattermostChannel;
use crate::openhuman::channels::qq::QQChannel;
use crate::openhuman::channels::signal::SignalChannel;
use crate::openhuman::channels::slack::SlackChannel;
use crate::openhuman::channels::telegram::TelegramChannel;
use crate::openhuman::channels::traits;
use crate::openhuman::channels::whatsapp::WhatsAppChannel;
#[cfg(feature = "whatsapp-web")]
use crate::openhuman::channels::whatsapp_web::WhatsAppWebChannel;
use crate::openhuman::channels::yuanbao::YuanbaoChannel;
use crate::openhuman::channels::Channel;
use crate::openhuman::config::Config;
use crate::openhuman::context::channels_prompt::build_system_prompt;
use crate::openhuman::inference::provider;
use crate::openhuman::memory::Memory;
use crate::openhuman::memory_store;
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools;
use anyhow::Result;
use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::mpsc;

/// How the channels runtime should construct its default chat provider.
///
/// Issue #3098 sub-issue 1: the runtime used to ignore the per-workload
/// `chat_provider` routing and unconditionally build a cloud chain, so
/// Telegram (and other channels) never honored a user's local-Ollama /
/// BYOK selection. `resolve_chat_workload` inspects the resolved chat
/// workload string and chooses between preserving the legacy
/// `create_intelligent_routing_provider` chain (Cloud) and dispatching
/// to the unified workload factory (Workload).
pub(super) enum ChatWorkloadResolution {
    /// Preserve the existing cloud chain (`ReliableProvider` +
    /// `IntelligentRoutingProvider`) and `config.default_model`.
    Cloud,
    /// Build the channel provider via `create_chat_provider("chat", config)`.
    Workload {
        provider_string: String,
        slug: String,
    },
}

pub(super) struct RelayInboundMessageHandler {
    tx: mpsc::Sender<RuntimeChannelMessage>,
}

impl RelayInboundMessageHandler {
    pub(super) fn new(tx: mpsc::Sender<RuntimeChannelMessage>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl tinychannels::relay::RelayInboundHandler for RelayInboundMessageHandler {
    async fn handle(
        &self,
        event: tinychannels::relay::AuthenticatedRelayInboundEvent,
    ) -> Result<(), tinychannels::relay::RelayTransportError> {
        let envelope: tinychannels::ChannelInboundEnvelope = serde_json::from_value(event.event)
            .map_err(|error| {
                tinychannels::relay::RelayTransportError::Handler(format!(
                    "invalid inbound envelope: {error}"
                ))
            })?;
        let msg = tinychannels::legacy_message_from_inbound_envelope(&envelope, 0);
        self.tx
            .send(RuntimeChannelMessage::with_inbound_envelope(msg, envelope))
            .await
            .map_err(|_| tinychannels::relay::RelayTransportError::Closed)
    }
}

struct RelayRuntimeHandle {
    _transport: Arc<tinychannels::relay::RelayTransport>,
    _reconnect: tinychannels::relay::RelayReconnectHandle,
}

async fn start_relay_runtime(
    relay: &tinychannels::config::RelayRuntimeConfig,
    tx: mpsc::Sender<RuntimeChannelMessage>,
) -> Result<RelayRuntimeHandle> {
    anyhow::ensure!(
        relay.is_listener_configured(),
        "relay runtime requires non-empty url and at least one identity"
    );

    let websocket_config = tinychannels::relay::WebSocketRelayConfig::from(relay);
    let io = tinychannels::relay::connect_websocket_relay_io(&websocket_config).await?;
    let transport = Arc::new(tinychannels::relay::RelayTransport::new(
        relay.relay_identities(),
        Arc::new(io),
        relay.timeouts,
    ));
    transport
        .set_inbound_handler(Arc::new(RelayInboundMessageHandler::new(tx)))
        .await;
    transport.connect().await?;
    let descriptor = transport.handshake().await?;
    tracing::info!(
        label = %descriptor.label,
        max_message_length = descriptor.max_message_length,
        "[channels][relay] connected relay runtime"
    );
    crate::openhuman::channels::relay_runtime::register_relay_transport(transport.clone());

    let dialer = Arc::new(tinychannels::relay::WebSocketRelayDialer::new(
        websocket_config,
    ));
    let reconnect = transport.spawn_reconnect_supervisor(dialer, relay.reconnect);
    Ok(RelayRuntimeHandle {
        _transport: transport,
        _reconnect: reconnect,
    })
}

pub(super) fn resolve_chat_workload(config: &Config) -> ChatWorkloadResolution {
    let resolved = provider::provider_for_role("chat", config);
    let trimmed = resolved.trim();
    if trimmed.is_empty() || trimmed == "cloud" || trimmed == provider::INFERENCE_BACKEND_ID {
        return ChatWorkloadResolution::Cloud;
    }
    let slug = trimmed
        .split_once(':')
        .map(|(s, _)| s.to_string())
        .unwrap_or_else(|| trimmed.to_string());
    ChatWorkloadResolution::Workload {
        provider_string: trimmed.to_string(),
        slug,
    }
}

pub async fn start_channels(mut config: Config) -> Result<()> {
    // Initialize the global event bus singleton and register the tracing
    // subscriber for debug logging of all domain events.
    let bus = event_bus::init_global(DEFAULT_CAPACITY);
    let _tracing_handle = bus.subscribe(Arc::new(TracingSubscriber));
    crate::openhuman::health::bus::register_health_subscriber();
    crate::openhuman::memory_conversations::register_conversation_persistence_subscriber(
        config.workspace_dir.clone(),
    );
    crate::openhuman::memory::sync::register_sync_stage_bridge(&config);
    crate::openhuman::composio::register_composio_trigger_subscriber();
    crate::openhuman::agent_meetings::calendar::register_meet_calendar_subscriber();
    crate::openhuman::agent_meetings::bus::register_meeting_event_subscriber();
    // Surface parked ApprovalGate requests as chat messages so the user can
    // answer yes/no in the thread (chat-native approval, issue #1339).
    crate::openhuman::web_chat::register_approval_surface_subscriber();
    // Surface generated-artifact lifecycle events (ArtifactReady /
    // ArtifactFailed) as `artifact_ready` / `artifact_failed` web-channel
    // events so the frontend ArtifactCard can render in chat (#2779).
    crate::openhuman::web_chat::register_artifact_surface_subscriber();
    // Surface external-egress disclosure events (ExternalTransferPending) as
    // `external_transfer_pending` web-channel events so the frontend can show a
    // per-action "what leaves, to where, why" card (privacy epic S2, #4436).
    crate::openhuman::web_chat::register_egress_surface_subscriber();
    // Spawn the per-toolkit provider periodic sync scheduler. This is
    // a thin tokio task that ticks every minute and dispatches into
    // any provider whose `sync_interval_secs` has elapsed for an
    // active Composio connection. Safe to call here even though
    // `bootstrap_core_runtime` may also start it — `start_periodic_sync`
    // is intentionally cheap and the loop body no-ops when there are
    // no connections.
    crate::openhuman::composio::start_periodic_sync();
    // Task-sources: subscribe to Composio connection-created events for
    // one-shot fetches, and spawn the periodic poll that pulls work from
    // configured external sources onto the agent's todo board.
    crate::openhuman::task_sources::bus::register_task_sources_subscriber();
    crate::openhuman::task_sources::start_periodic_poll();
    // Board poller: dispatch the highest-urgency `todo` card on the
    // task-sources board (catch-all for cards without a proactive trigger).
    crate::openhuman::agent::task_dispatcher::start_board_poller();
    // Native request handlers. Re-registering is safe (latest wins) so
    // this is idempotent even if `bootstrap_core_runtime` also runs.
    // Must happen before `run_message_dispatch_loop` begins, because
    // channel dispatch calls `request_native_global("agent.run_turn", …)`
    // for every inbound message.
    crate::openhuman::agent::bus::register_agent_handlers();
    // The Phase 2/3/4 self-improvement subscribers (email-signature producer,
    // rebuild trigger, ProfileMdRenderer) are registered in
    // core::jsonrpc::register_domain_subscribers instead. start_channels is
    // skipped when no channel is configured, so wiring them here silently
    // dropped user-profile inference for channel-less users (#5003).

    tracing::debug!("[event_bus] global singleton initialized in start_channels");

    // Initialise the sub-agent definition registry from this workspace.
    // Idempotent — `bootstrap_core_runtime` may also call it.
    if let Err(err) = crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(
        &config.workspace_dir,
    ) {
        tracing::warn!(
            "AgentDefinitionRegistry::init_global failed: {err} — \
             spawn_subagent will be unavailable until restart"
        );
    }
    // Note: WebhookRequestSubscriber and ChannelInboundSubscriber are registered
    // in bootstrap_core_runtime() (src/core/jsonrpc.rs) to avoid double-registration
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
    crate::openhuman::config::ensure_agent_dirs(&mut config).await;
    // Install as the process-global live policy so runtime autonomy changes
    // (config.update_autonomy_settings) are reflected by `live_policy::current()`
    // and picked up by the next session.
    let security = crate::openhuman::security::live_policy::install(
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
    // `core::jsonrpc::register_domain_subscribers` (unconditional core boot), NOT
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
    let audit = crate::openhuman::security::get_or_create_workspace_audit_logger(
        crate::openhuman::config::AuditConfig::default(),
        config.workspace_dir.clone(),
    )?;
    let temperature = config.default_temperature;
    let local_embedding = config.workload_local_model("embeddings");
    let embedding_api_key =
        crate::openhuman::embeddings::resolve_api_key(&config, &config.memory.embedding_provider);
    // Build the memory store. A misconfigured/removed embedding provider (e.g. a
    // stale `embedding_provider = "fastembed"` that the factory no longer knows)
    // makes the embedder build fail — but that must NOT take every messaging
    // channel offline (issue #3712). Fall back to keyword-only memory
    // (`embedding_provider = "none"` → NoopEmbedding) so the channel listeners
    // still start; semantic memory degrades gracefully instead of the whole
    // runtime aborting.
    let mem: Arc<dyn Memory> = match memory_store::create_memory_with_local_ai(
        &config.memory,
        local_embedding.as_deref(),
        &embedding_api_key,
        &[],
        Some(&config.storage.provider.config),
        &config.workspace_dir,
    ) {
        Ok(mem) => Arc::from(mem),
        Err(e) => {
            tracing::error!(
                error = %format!("{e:#}"),
                provider = %config.memory.embedding_provider,
                "[channels] memory embedder build failed — falling back to keyword-only \
                 memory so channels still start"
            );
            let mut fallback_memory = config.memory.clone();
            fallback_memory.embedding_provider = "none".to_string();
            Arc::from(memory_store::create_memory_with_local_ai(
                &fallback_memory,
                local_embedding.as_deref(),
                &embedding_api_key,
                &[],
                Some(&config.storage.provider.config),
                &config.workspace_dir,
            )?)
        }
    };
    // Build system prompt from workspace identity files + skills
    let workspace = config.workspace_dir.clone();
    let tools_registry = Arc::new(tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        audit,
        Arc::clone(&mem),
        &config.browser,
        &config.http_request,
        &config.action_dir,
        &config.agents,
        &config,
        None,
        None,
    ));

    let skills = crate::openhuman::skills::load_workflow_metadata(&workspace);

    // Install the triggered-workflow subscriber now that workflows are
    // discovered — otherwise any workflow declaring `triggers:` is silently
    // ignored. Idempotent + shares a process-global OnceLock with the
    // `bootstrap_core_runtime` site, so it registers exactly once regardless of
    // which startup path runs first (web-chat-only cores never reach here).
    crate::openhuman::skills::bus::ensure_triggered_workflow_subscriber(&workspace);

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
    // agent prompt — those tools are only available to the integrations_agent
    // subagent via category_filter = "skill".
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
    // `channel_name = None` on startup: the channel runtime wires up
    // multiple providers in parallel, so there's no single platform to
    // name here. The capability block falls back to a platform-agnostic
    // "messaging bot" phrasing. Per-channel renderers that want a
    // named capabilities section can call `build_system_prompt` with
    // `Some(name)` directly.
    let mut system_prompt = build_system_prompt(
        &workspace,
        &model,
        &tool_descs,
        &skills,
        bootstrap_max_chars,
        None,
    );
    // Filter out Workflow-category tools (e.g. Composio, Apify) from the
    // main agent prompt — those are only available to the integrations_agent
    // subagent via category_filter = "skill".
    let non_skill_tools: Vec<&Box<dyn crate::openhuman::tools::Tool>> = tools_registry
        .iter()
        .filter(|t| t.category() != crate::openhuman::tools::traits::ToolCategory::Workflow)
        .collect();
    let non_skill_refs: Vec<&dyn crate::openhuman::tools::Tool> =
        non_skill_tools.iter().map(|t| t.as_ref()).collect();
    system_prompt.push_str(&build_tool_instructions_filtered(&non_skill_refs));
    // Tell the model its current filesystem access boundaries so it self-limits
    // (advisory only — the SecurityPolicy enforces these regardless).
    system_prompt.push_str(&format_access_context(&security));

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
    let channel_host =
        crate::openhuman::channels::host::build_channel_host(Arc::new(config.clone()));

    // Collect active channels
    let mut channels: Vec<Arc<dyn Channel>> = Vec::new();

    if let Some(ref tg) = config.channels_config.telegram {
        tracing::info!(
            channel = "telegram",
            allowed_users_count = tg.allowed_users.len(),
            mention_only = tg.mention_only,
            stream_mode = ?tg.stream_mode,
            draft_update_interval_ms = tg.draft_update_interval_ms,
            "[channels] telegram enabled in core config (bot token not logged)"
        );
        let mut telegram = TelegramChannel::new(
            tg.bot_token.clone(),
            tg.allowed_users.clone(),
            tg.mention_only,
        )
        .with_streaming(
            tg.stream_mode,
            tg.draft_update_interval_ms,
            tg.silent_streaming,
        )
        .with_chat_id(tg.chat_id.clone())
        .with_http_client(crate::openhuman::config::build_runtime_proxy_client(
            "channel.telegram",
        ));
        // Inject host capabilities: voice STT, persisted allowlist, reaction
        // event fan-out. Each is optional — telegram degrades gracefully.
        if let Some(transcriber) = channel_host.transcriber() {
            telegram = telegram.with_transcriber(transcriber);
        }
        if let Some(allowlist) = channel_host.allowlist() {
            telegram = telegram.with_allowlist(allowlist);
        }
        if let Some(events) = channel_host.events() {
            telegram = telegram.with_events(events);
        }
        channels.push(Arc::new(telegram));
    } else {
        tracing::info!(
            "[channels] telegram not configured (no channels_config.telegram in saved config)"
        );
    }

    if let Some(ref dc) = config.channels_config.discord {
        channels.push(Arc::new(DiscordChannel::with_http_client(
            dc.bot_token.clone(),
            dc.guild_id.clone(),
            dc.channel_id.clone(),
            dc.allowed_users.clone(),
            dc.listen_to_bots,
            dc.mention_only,
            crate::openhuman::config::build_runtime_proxy_client("channel.discord"),
        )));
    }

    if let Some(ref sl) = config.channels_config.slack {
        channels.push(Arc::new(SlackChannel::with_http_client(
            sl.bot_token.clone(),
            sl.channel_id.clone(),
            sl.allowed_users.clone(),
            crate::openhuman::config::build_runtime_proxy_client("channel.slack"),
        )));
        // Memory-tree ingestion is handled by the Composio-backed
        // `SlackProvider`, which runs inside `composio::periodic` and
        // fires per-connection on its own 15-minute cadence. No spawn
        // required here.
    }

    if let Some(ref mm) = config.channels_config.mattermost {
        channels.push(Arc::new(MattermostChannel::with_http_client(
            mm.url.clone(),
            mm.bot_token.clone(),
            mm.channel_id.clone(),
            mm.allowed_users.clone(),
            mm.thread_replies.unwrap_or(true),
            mm.mention_only.unwrap_or(false),
            crate::openhuman::config::build_runtime_proxy_client("channel.mattermost"),
        )));
    }

    if let Some(ref im) = config.channels_config.imessage {
        channels.push(Arc::new(IMessageChannel::new(im.allowed_contacts.clone())));
    }

    if config.channels_config.matrix.is_some() {
        tracing::warn!(
            "Matrix channel is configured but Matrix support was removed from this build; skipping Matrix runtime startup."
        );
    }

    if let Some(ref sig) = config.channels_config.signal {
        channels.push(Arc::new(SignalChannel::with_http_client(
            sig.http_url.clone(),
            sig.account.clone(),
            sig.group_id.clone(),
            sig.allowed_from.clone(),
            sig.ignore_attachments,
            sig.ignore_stories,
            crate::openhuman::config::apply_runtime_proxy_to_builder(
                reqwest::Client::builder().connect_timeout(std::time::Duration::from_secs(10)),
                "channel.signal",
            )
            .build()
            .expect("Signal HTTP client should build"),
        )));
    }

    if let Some(ref wa) = config.channels_config.whatsapp {
        // Runtime negotiation: detect backend type from config
        match wa.backend_type() {
            "cloud" => {
                // Cloud API mode: requires phone_number_id, access_token, verify_token
                if wa.is_cloud_config() {
                    channels.push(Arc::new(WhatsAppChannel::with_http_client(
                        wa.access_token.clone().unwrap_or_default(),
                        wa.phone_number_id.clone().unwrap_or_default(),
                        wa.verify_token.clone().unwrap_or_default(),
                        wa.allowed_numbers.clone(),
                        crate::openhuman::config::build_runtime_proxy_client("channel.whatsapp"),
                    )));
                } else {
                    tracing::warn!("WhatsApp Cloud API configured but missing required fields (phone_number_id, access_token, verify_token)");
                }
            }
            "web" => {
                // Web mode: requires session_path
                #[cfg(feature = "whatsapp-web")]
                if wa.is_web_config() {
                    let mut wa_channel = WhatsAppWebChannel::new(
                        wa.session_path.clone().unwrap_or_default(),
                        wa.pair_phone.clone(),
                        wa.pair_code.clone(),
                        wa.allowed_numbers.clone(),
                    );
                    if let Some(lifecycle) = channel_host.lifecycle() {
                        wa_channel = wa_channel.with_lifecycle(lifecycle);
                    }
                    channels.push(Arc::new(wa_channel));
                } else {
                    tracing::warn!("WhatsApp Web configured but session_path not set");
                }
                #[cfg(not(feature = "whatsapp-web"))]
                {
                    tracing::warn!("WhatsApp Web backend requires 'whatsapp-web' feature. Enable with: cargo build --features whatsapp-web");
                }
            }
            _ => {
                tracing::warn!("WhatsApp config invalid: neither phone_number_id (Cloud API) nor session_path (Web) is set");
            }
        }
    }

    if let Some(ref lq) = config.channels_config.linq {
        channels.push(Arc::new(LinqChannel::new(
            lq.api_token.clone(),
            lq.from_phone.clone(),
            lq.allowed_senders.clone(),
        )));
    }

    if let Some(ref email_cfg) = config.channels_config.email {
        let hydrated = resolve_email_password(email_cfg.clone(), &config);
        channels.push(Arc::new(EmailChannel::new(hydrated)));
    }

    if let Some(ref irc) = config.channels_config.irc {
        channels.push(Arc::new(IrcChannel::new(irc::IrcChannelConfig {
            server: irc.server.clone(),
            port: irc.port,
            nickname: irc.nickname.clone(),
            username: irc.username.clone(),
            channels: irc.channels.clone(),
            allowed_users: irc.allowed_users.clone(),
            server_password: irc.server_password.clone(),
            nickserv_password: irc.nickserv_password.clone(),
            sasl_password: irc.sasl_password.clone(),
            verify_tls: irc.verify_tls.unwrap_or(true),
        })));
    }

    if let Some(ref lk) = config.channels_config.lark {
        channels.push(Arc::new(LarkChannel::from_config(lk)));
    }

    if let Some(ref dt) = config.channels_config.dingtalk {
        channels.push(Arc::new(DingTalkChannel::with_http_client(
            dt.client_id.clone(),
            dt.client_secret.clone(),
            dt.allowed_users.clone(),
            crate::openhuman::config::build_runtime_proxy_client("channel.dingtalk"),
        )));
    }

    if let Some(ref qq) = config.channels_config.qq {
        channels.push(Arc::new(QQChannel::with_http_client(
            qq.app_id.clone(),
            qq.app_secret.clone(),
            qq.allowed_users.clone(),
            crate::openhuman::config::build_runtime_proxy_client("channel.qq"),
        )));
    }

    if let Some(ref yb) = config.channels_config.yuanbao {
        let yb_cfg = resolve_yuanbao_app_secret(yb.clone(), &config);
        match YuanbaoChannel::new(yb_cfg) {
            Ok(ch) => channels.push(Arc::new(ch)),
            Err(e) => tracing::warn!("[channels] yuanbao config invalid: {e}"),
        }
    }

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
    let effective_backend = memory_store::effective_memory_backend_name(
        &config.memory.backend,
        Some(&config.storage.provider.config),
    );
    println!(
        "  🧠 Memory:   {} (auto-save: {})",
        effective_backend,
        if config.memory.auto_save { "on" } else { "off" }
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

    event_bus::publish_global(DomainEvent::SystemStartup {
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
    let provider_bridge = tokio::spawn(async move {
        while let Some(msg) = provider_rx.recv().await {
            if provider_dispatch_tx
                .send(RuntimeChannelMessage::from(msg))
                .await
                .is_err()
            {
                break;
            }
        }
    });

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
        handles.push(spawn_supervised_listener(
            ch.clone(),
            provider_tx.clone(),
            initial_backoff_secs,
            max_backoff_secs,
        ));
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
        crate::openhuman::cron::bus::CronDeliverySubscriber::new(Arc::clone(&channels_by_name)),
    ));
    // NOTE: the flows `FlowTriggerSubscriber` is registered in
    // `jsonrpc.rs::register_domain_subscribers` (unconditional core boot), NOT
    // here — `start_channels` is skipped when no channel is configured or
    // `OPENHUMAN_DISABLE_CHANNEL_LISTENERS` is set, which would otherwise leave
    // schedule/app-event workflows undispatched (issue B2 review).
    // Register the proactive message subscriber so morning briefings,
    // welcome messages, and other proactive agent output gets routed to
    // the user's active channel (+ always to web).
    let proactive_sub = crate::openhuman::channels::proactive::ProactiveMessageSubscriber::new(
        Arc::clone(&channels_by_name),
        config.channels_config.active_channel.clone(),
    );
    // Expose its active-channel handle so the `channels_set_default` RPC can
    // switch the default channel at runtime without a restart (issue #3712).
    crate::openhuman::channels::proactive::register_active_channel_handle(
        proactive_sub.active_channel_handle(),
    );
    let _proactive_handle = bus.subscribe(Arc::new(proactive_sub));
    let _telegram_remote_handle = if channels_by_name.contains_key("telegram") {
        let handle = bus.subscribe(Arc::new(
            crate::openhuman::channels::providers::telegram::TelegramRemoteSubscriber::new(
                config.workspace_dir.clone(),
            ),
        ));
        tracing::debug!("[telegram-remote] registered TelegramRemoteSubscriber");
        Some(handle)
    } else {
        None
    };
    // Sub-issue 2 of #3098: when Telegram is enabled, register the
    // approval-surface subscriber so `Prompt`-class tool calls actually
    // get gated for the user instead of silently allowed (the legacy
    // behavior when `ApprovalChatContext` is unset). The dispatch loop
    // pairs this by scoping each Telegram turn in an `ApprovalChatContext`
    // and intercepting `yes`/`no` replies for parked approvals.
    let _telegram_approval_surface_handle = if channels_by_name.contains_key("telegram") {
        let handle = bus.subscribe(Arc::new(
            crate::openhuman::channels::providers::telegram::TelegramApprovalSurfaceSubscriber::new(
                Arc::clone(&channels_by_name),
            ),
        ));
        tracing::debug!("[telegram-approval] registered TelegramApprovalSurfaceSubscriber");
        Some(handle)
    } else {
        None
    };
    // Register the tree summarizer event subscriber for observability logging.
    let _tree_summarizer_handle = bus.subscribe(Arc::new(
        crate::openhuman::memory_tree::tree_runtime::bus::TreeSummarizerEventSubscriber::new(),
    ));

    let listener_count = channels.len() + relay_config.as_ref().map(|_| 1).unwrap_or_default();
    let max_in_flight_messages = compute_max_in_flight_messages(listener_count);

    println!("  🚦 In-flight message limit: {max_in_flight_messages}");

    let message_timeout_secs =
        effective_channel_message_timeout_secs(config.channels_config.message_timeout_secs);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name,
        provider: None,
        default_provider: Arc::new(provider_name),
        memory: Arc::clone(&mem),
        tools_registry: Arc::clone(&tools_registry),
        system_prompt: Arc::new(system_prompt),
        model: Arc::new(model.clone()),
        temperature,
        auto_save_memory: config.memory.auto_save,
        max_tool_iterations: config.agent.max_tool_iterations,
        min_relevance_score: config.memory.min_relevance_score,
        conversation_histories: Arc::new(Mutex::new(HashMap::new())),
        provider_cache: Arc::new(Mutex::new(HashMap::new())),
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

/// Render the agent's current filesystem-access boundaries as a system-prompt
/// section. Advisory only: the `SecurityPolicy` enforces these regardless of
/// what the model believes, but stating them keeps the model from wasting turns
/// attempting actions the runtime will deny.
fn format_access_context(security: &SecurityPolicy) -> String {
    use crate::openhuman::security::{AutonomyLevel, TrustedAccess};

    let mode = match security.autonomy {
        AutonomyLevel::ReadOnly => "read-only (observe only; no writes or shell commands)",
        AutonomyLevel::Supervised => "supervised (acts; risky operations require approval)",
        AutonomyLevel::Full => "full (autonomous within policy bounds)",
    };
    let mut s =
        String::from("\n\n## Host access (enforced by the runtime — you cannot exceed this)\n");
    s.push_str(&format!("- Access mode: {mode}\n"));
    s.push_str(&format!(
        "- Workspace: {} ({})\n",
        security.workspace_dir.display(),
        if security.workspace_only {
            "file access confined to the workspace"
        } else {
            "workspace_only is OFF"
        }
    ));
    if security.trusted_roots.is_empty() {
        s.push_str("- Trusted roots outside the workspace: none granted\n");
    } else {
        s.push_str("- Trusted roots outside the workspace:\n");
        for root in &security.trusted_roots {
            let access = match root.access {
                TrustedAccess::Read => "read-only",
                TrustedAccess::ReadWrite => "read+write",
            };
            s.push_str(&format!("    - {} ({access})\n", root.path));
        }
    }
    s.push_str(&format!(
        "- OS package installation: {}\n",
        if security.allow_tool_install {
            "allowed via install_tool"
        } else {
            "disabled"
        }
    ));
    s.push_str(
        "Credential stores (~/.ssh, ~/.gnupg, ~/.aws) are always blocked. \
         Use detect_tools to check what's installed before assuming a tool exists.\n",
    );
    s
}

/// Best-effort fill of `yb_cfg.app_secret` from the encrypted credentials
/// store when TOML doesn't already carry one.
///
/// `app_secret` is intentionally not persisted in `config.toml` (see the
/// `yuanbao` branch in `controllers/ops.rs`). Existing TOML values still
/// win so manually-installed deployments don't break. Returns the
/// (possibly-modified) config; logging is the only side effect on failure.
///
/// The stored secret is **only** copied when the stored profile's
/// `app_key` matches `yb_cfg.app_key`. Without that guard, editing
/// `app_key` in `config.toml` would silently pair a fresh key with a
/// stale secret on next startup, and the channel would fail auth until
/// the user reconnected or cleared credentials manually.
fn resolve_yuanbao_app_secret(
    mut yb_cfg: crate::openhuman::channels::providers::yuanbao::YuanbaoConfig,
    config: &Config,
) -> crate::openhuman::channels::providers::yuanbao::YuanbaoConfig {
    if !yb_cfg.app_secret.is_empty() {
        return yb_cfg;
    }
    let auth = crate::openhuman::credentials::AuthService::from_config(config);
    match auth.get_profile("channel:yuanbao:api_key", None) {
        Ok(Some(profile)) => {
            let stored_app_key = profile.metadata.get("app_key").map(String::as_str);
            if stored_app_key != Some(yb_cfg.app_key.as_str()) {
                tracing::warn!(
                    "[channels] yuanbao stored credentials are for a different app_key (toml={:?}, store={:?}); reconnect the channel to refresh the secret",
                    yb_cfg.app_key,
                    stored_app_key,
                );
            } else if let Some(secret) = profile.metadata.get("app_secret") {
                yb_cfg.app_secret = secret.clone();
            }
        }
        Ok(None) => {
            tracing::warn!(
                "[channels] yuanbao credentials missing — connect the channel again from the UI"
            );
        }
        Err(e) => {
            tracing::warn!("[channels] failed to load yuanbao credentials: {e}");
        }
    }
    yb_cfg
}

/// Best-effort fill of `email_cfg.password` from the encrypted credentials store
/// when TOML doesn't already carry one.
///
/// The IMAP/SMTP `password` is intentionally not persisted in `config.toml` (see
/// `persist_email_config` in `controllers/ops/connect.rs`); it lives only in the
/// credentials store under `channel:email:api_key`. Existing TOML values still
/// win so manually-installed deployments keep working. The stored secret is only
/// copied when the stored profile's `username` matches, so editing `username` in
/// `config.toml` can't silently pair a fresh account with a stale password.
fn resolve_email_password(
    mut email_cfg: crate::openhuman::channels::email_channel::EmailConfig,
    config: &Config,
) -> crate::openhuman::channels::email_channel::EmailConfig {
    if !email_cfg.password.is_empty() {
        return email_cfg;
    }
    let auth = crate::openhuman::credentials::AuthService::from_config(config);
    match auth.get_profile("channel:email:api_key", None) {
        Ok(Some(profile)) => {
            let stored_username = profile.metadata.get("username").map(String::as_str);
            if stored_username != Some(email_cfg.username.as_str()) {
                tracing::warn!(
                    "[channels] email stored credentials are for a different username (toml={:?}, store={:?}); reconnect the channel to refresh the password",
                    email_cfg.username,
                    stored_username,
                );
            } else if let Some(password) = profile.metadata.get("password") {
                email_cfg.password = password.clone();
            }
        }
        Ok(None) => {
            tracing::warn!(
                "[channels] email credentials missing — connect the channel again from the UI"
            );
        }
        Err(e) => {
            tracing::warn!("[channels] failed to load email credentials: {e}");
        }
    }
    email_cfg
}

#[cfg(any(test, debug_assertions))]
pub mod test_support {
    use super::*;

    pub fn resolve_yuanbao_app_secret_for_test(
        yb_cfg: crate::openhuman::channels::providers::yuanbao::YuanbaoConfig,
        config: &Config,
    ) -> crate::openhuman::channels::providers::yuanbao::YuanbaoConfig {
        resolve_yuanbao_app_secret(yb_cfg, config)
    }
}

#[cfg(test)]
#[path = "startup_tests.rs"]
mod tests;

#[cfg(test)]
mod yuanbao_secret_tests {
    use super::*;
    use crate::openhuman::channels::providers::yuanbao::YuanbaoConfig;
    use crate::openhuman::credentials::AuthService;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn isolated_config() -> (tempfile::TempDir, Config) {
        let tmp = tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = tmp.path().join("workspace");
        config.config_path = tmp.path().join("config.toml");
        std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
        (tmp, config)
    }

    #[test]
    fn loads_app_secret_from_credentials_when_toml_empty() {
        let (_tmp, config) = isolated_config();
        // Pre-write the credentials the same way `connect_channel` does:
        // metadata under the `channel:yuanbao:api_key` provider key.
        let auth = AuthService::from_config(&config);
        let mut metadata = HashMap::new();
        metadata.insert("app_key".to_string(), "ak".to_string());
        metadata.insert("app_secret".to_string(), "from-credentials".to_string());
        auth.store_provider_token("channel:yuanbao:api_key", "default", "", metadata, true)
            .expect("store credentials");

        let yb = YuanbaoConfig {
            app_key: "ak".into(),
            app_secret: String::new(),
            ..Default::default()
        };
        let resolved = resolve_yuanbao_app_secret(yb, &config);
        assert_eq!(resolved.app_secret, "from-credentials");
    }

    #[test]
    fn preserves_existing_toml_secret_without_consulting_store() {
        // No credentials in the store at all — resolver must still leave
        // the TOML-supplied secret untouched.
        let (_tmp, config) = isolated_config();
        let yb = YuanbaoConfig {
            app_key: "ak".into(),
            app_secret: "from-toml".into(),
            ..Default::default()
        };
        let resolved = resolve_yuanbao_app_secret(yb, &config);
        assert_eq!(resolved.app_secret, "from-toml");
    }

    #[test]
    fn returns_empty_secret_when_neither_toml_nor_credentials_have_one() {
        let (_tmp, config) = isolated_config();
        let yb = YuanbaoConfig {
            app_key: "ak".into(),
            app_secret: String::new(),
            ..Default::default()
        };
        let resolved = resolve_yuanbao_app_secret(yb, &config);
        // Surfaces empty so the downstream `YuanbaoChannel::new` validate()
        // step can fail clearly, instead of attempting auth with a stale value.
        assert_eq!(resolved.app_secret, "");
    }

    #[test]
    fn skips_hydration_when_stored_profile_has_different_app_key() {
        // Reproduces the stale-secret hazard: user changed `app_key` in
        // `config.toml` (e.g. swapped to a different bot) but the
        // credentials store still has the old key's profile. The resolver
        // must NOT graft the old secret onto the new key.
        let (_tmp, config) = isolated_config();
        let auth = AuthService::from_config(&config);
        let mut metadata = HashMap::new();
        metadata.insert("app_key".to_string(), "OLD-KEY".to_string());
        metadata.insert(
            "app_secret".to_string(),
            "old-key-secret-do-not-use".to_string(),
        );
        auth.store_provider_token("channel:yuanbao:api_key", "default", "", metadata, true)
            .expect("store credentials");

        let yb = YuanbaoConfig {
            app_key: "NEW-KEY".into(),
            app_secret: String::new(),
            ..Default::default()
        };
        let resolved = resolve_yuanbao_app_secret(yb, &config);
        assert_eq!(
            resolved.app_secret, "",
            "stale profile keyed to OLD-KEY must not hydrate NEW-KEY's secret",
        );
    }
}

#[cfg(test)]
mod email_secret_tests {
    use super::*;
    use crate::openhuman::channels::email_channel::EmailConfig;
    use crate::openhuman::credentials::AuthService;
    use std::collections::HashMap;
    use tempfile::tempdir;

    fn isolated_config() -> (tempfile::TempDir, Config) {
        let tmp = tempdir().expect("tempdir");
        let mut config = Config::default();
        config.workspace_dir = tmp.path().join("workspace");
        config.config_path = tmp.path().join("config.toml");
        std::fs::create_dir_all(&config.workspace_dir).expect("workspace dir");
        (tmp, config)
    }

    fn store_email_creds(config: &Config, username: &str, password: &str) {
        let auth = AuthService::from_config(config);
        let mut metadata = HashMap::new();
        metadata.insert("username".to_string(), username.to_string());
        metadata.insert("password".to_string(), password.to_string());
        auth.store_provider_token("channel:email:api_key", "default", "", metadata, true)
            .expect("store credentials");
    }

    #[test]
    fn loads_password_from_credentials_when_toml_empty() {
        let (_tmp, config) = isolated_config();
        store_email_creds(&config, "me@example.com", "from-credentials");

        let cfg = EmailConfig {
            username: "me@example.com".into(),
            password: String::new(),
            ..EmailConfig::default()
        };
        let resolved = resolve_email_password(cfg, &config);
        assert_eq!(resolved.password, "from-credentials");
    }

    #[test]
    fn preserves_existing_toml_password_without_consulting_store() {
        let (_tmp, config) = isolated_config();
        let cfg = EmailConfig {
            username: "me@example.com".into(),
            password: "from-toml".into(),
            ..EmailConfig::default()
        };
        let resolved = resolve_email_password(cfg, &config);
        assert_eq!(resolved.password, "from-toml");
    }

    #[test]
    fn skips_hydration_when_stored_profile_has_different_username() {
        // User changed `username` in config.toml; the stored profile is for the
        // old account. The resolver must not graft the old password onto it.
        let (_tmp, config) = isolated_config();
        store_email_creds(&config, "old@example.com", "old-password-do-not-use");

        let cfg = EmailConfig {
            username: "new@example.com".into(),
            password: String::new(),
            ..EmailConfig::default()
        };
        let resolved = resolve_email_password(cfg, &config);
        assert_eq!(
            resolved.password, "",
            "stale profile for old username must not hydrate the new account",
        );
    }
}
