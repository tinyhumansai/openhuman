//! Channel startup wiring.

use super::dispatch::run_message_dispatch_loop;
use super::supervision::{compute_max_in_flight_messages, spawn_supervised_listener};
use crate::openhuman::agent::harness::build_tool_instructions;
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
#[cfg(feature = "channel-matrix")]
use crate::openhuman::channels::matrix::MatrixChannel;
use crate::openhuman::channels::mattermost::MattermostChannel;
use crate::openhuman::channels::prompt::build_system_prompt;
use crate::openhuman::channels::qq::QQChannel;
use crate::openhuman::channels::signal::SignalChannel;
use crate::openhuman::channels::slack::SlackChannel;
use crate::openhuman::channels::telegram::TelegramChannel;
use crate::openhuman::channels::traits;
use crate::openhuman::channels::whatsapp::WhatsAppChannel;
#[cfg(feature = "whatsapp-web")]
use crate::openhuman::channels::whatsapp_web::WhatsAppWebChannel;
use crate::openhuman::channels::Channel;
use crate::openhuman::config::Config;
use crate::openhuman::event_bus::{self, DomainEvent, TracingSubscriber, DEFAULT_CAPACITY};
use crate::openhuman::memory::{self, Memory};
use crate::openhuman::providers::{self, Provider};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools;
use anyhow::Result;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub async fn start_channels(config: Config) -> Result<()> {
    // Initialize the global event bus singleton and register the tracing
    // subscriber for debug logging of all domain events.
    let bus = event_bus::init_global(DEFAULT_CAPACITY);
    let _tracing_handle = bus.subscribe(Arc::new(TracingSubscriber));
    crate::openhuman::health::bus::register_health_subscriber();
    crate::openhuman::skills::bus::register_skill_cleanup_subscriber();
    crate::openhuman::memory::conversations::register_conversation_persistence_subscriber(
        config.workspace_dir.clone(),
    );
    crate::openhuman::composio::register_composio_trigger_subscriber();
    tracing::debug!("[event_bus] global singleton initialized in start_channels");

    // Initialise the sub-agent definition registry from this workspace.
    // Idempotent — `bootstrap_skill_runtime` may also call it.
    if let Err(err) = crate::openhuman::agent::harness::AgentDefinitionRegistry::init_global(
        &config.workspace_dir,
    ) {
        tracing::warn!(
            "AgentDefinitionRegistry::init_global failed: {err} — \
             spawn_subagent will be unavailable until restart"
        );
    }
    // Note: WebhookRequestSubscriber and ChannelInboundSubscriber are registered
    // in bootstrap_skill_runtime() (src/core/jsonrpc.rs) to avoid double-registration
    // when both startup paths run in the same process.

    let provider_runtime_options = providers::ProviderRuntimeOptions {
        auth_profile_override: None,
        openhuman_dir: config.config_path.parent().map(std::path::PathBuf::from),
        secrets_encrypt: config.secrets.encrypt,
        reasoning_enabled: config.runtime.reasoning_enabled,
    };
    let provider: Arc<dyn Provider> = Arc::from(providers::create_resilient_provider_with_options(
        config.api_key.as_deref(),
        config.api_url.as_deref(),
        &config.reliability,
        &provider_runtime_options,
    )?);

    // Warm up the provider connection pool (TLS handshake, DNS, HTTP/2 setup)
    // so the first real message doesn't hit a cold-start timeout.
    if let Err(e) = provider.warmup().await {
        tracing::warn!("Provider warmup failed (non-fatal): {e}");
    }

    let runtime: Arc<dyn host_runtime::RuntimeAdapter> =
        Arc::from(host_runtime::create_runtime(&config.runtime)?);
    let security = Arc::new(SecurityPolicy::from_config(
        &config.autonomy,
        &config.workspace_dir,
    ));
    let model = config
        .default_model
        .clone()
        .unwrap_or_else(|| crate::openhuman::config::DEFAULT_MODEL.into());
    let temperature = config.default_temperature;
    let mem: Arc<dyn Memory> = Arc::from(memory::create_memory_with_storage(
        &config.memory,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
        config.api_key.as_deref(),
    )?);
    let (composio_key, composio_entity_id) = if config.composio.enabled {
        (
            config.composio.api_key.as_deref(),
            Some(config.composio.entity_id.as_str()),
        )
    } else {
        (None, None)
    };
    // Build system prompt from workspace identity files + skills
    let workspace = config.workspace_dir.clone();
    let tools_registry = Arc::new(tools::all_tools_with_runtime(
        Arc::new(config.clone()),
        &security,
        runtime,
        Arc::clone(&mem),
        composio_key,
        composio_entity_id,
        &config.browser,
        &config.http_request,
        &workspace,
        &config.agents,
        config.api_key.as_deref(),
        &config,
    ));

    let skills = crate::openhuman::skills::load_skills(&workspace);

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
    if config.composio.enabled {
        tool_descs.push((
            "composio",
            "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). Use action='list' to discover, 'execute' to run (optionally with connected_account_id), 'connect' to OAuth.",
        ));
    }
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
    let mut system_prompt = build_system_prompt(
        &workspace,
        &model,
        &tool_descs,
        &skills,
        bootstrap_max_chars,
    );
    system_prompt.push_str(&build_tool_instructions(tools_registry.as_ref()));

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
        channels.push(Arc::new(
            TelegramChannel::new(
                tg.bot_token.clone(),
                tg.allowed_users.clone(),
                tg.mention_only,
            )
            .with_streaming(tg.stream_mode, tg.draft_update_interval_ms),
        ));
    } else {
        tracing::info!(
            "[channels] telegram not configured (no channels_config.telegram in saved config)"
        );
    }

    if let Some(ref dc) = config.channels_config.discord {
        channels.push(Arc::new(DiscordChannel::new(
            dc.bot_token.clone(),
            dc.guild_id.clone(),
            dc.channel_id.clone(),
            dc.allowed_users.clone(),
            dc.listen_to_bots,
            dc.mention_only,
        )));
    }

    if let Some(ref sl) = config.channels_config.slack {
        channels.push(Arc::new(SlackChannel::new(
            sl.bot_token.clone(),
            sl.channel_id.clone(),
            sl.allowed_users.clone(),
        )));
    }

    if let Some(ref mm) = config.channels_config.mattermost {
        channels.push(Arc::new(MattermostChannel::new(
            mm.url.clone(),
            mm.bot_token.clone(),
            mm.channel_id.clone(),
            mm.allowed_users.clone(),
            mm.thread_replies.unwrap_or(true),
            mm.mention_only.unwrap_or(false),
        )));
    }

    if let Some(ref im) = config.channels_config.imessage {
        channels.push(Arc::new(IMessageChannel::new(im.allowed_contacts.clone())));
    }

    #[cfg(feature = "channel-matrix")]
    if let Some(ref mx) = config.channels_config.matrix {
        channels.push(Arc::new(MatrixChannel::new_with_session_hint(
            mx.homeserver.clone(),
            mx.access_token.clone(),
            mx.room_id.clone(),
            mx.allowed_users.clone(),
            mx.user_id.clone(),
            mx.device_id.clone(),
        )));
    }

    #[cfg(not(feature = "channel-matrix"))]
    if config.channels_config.matrix.is_some() {
        tracing::warn!(
            "Matrix channel is configured but this build was compiled without `channel-matrix`; skipping Matrix runtime startup."
        );
    }

    if let Some(ref sig) = config.channels_config.signal {
        channels.push(Arc::new(SignalChannel::new(
            sig.http_url.clone(),
            sig.account.clone(),
            sig.group_id.clone(),
            sig.allowed_from.clone(),
            sig.ignore_attachments,
            sig.ignore_stories,
        )));
    }

    if let Some(ref wa) = config.channels_config.whatsapp {
        // Runtime negotiation: detect backend type from config
        match wa.backend_type() {
            "cloud" => {
                // Cloud API mode: requires phone_number_id, access_token, verify_token
                if wa.is_cloud_config() {
                    channels.push(Arc::new(WhatsAppChannel::new(
                        wa.access_token.clone().unwrap_or_default(),
                        wa.phone_number_id.clone().unwrap_or_default(),
                        wa.verify_token.clone().unwrap_or_default(),
                        wa.allowed_numbers.clone(),
                    )));
                } else {
                    tracing::warn!("WhatsApp Cloud API configured but missing required fields (phone_number_id, access_token, verify_token)");
                }
            }
            "web" => {
                // Web mode: requires session_path
                #[cfg(feature = "whatsapp-web")]
                if wa.is_web_config() {
                    channels.push(Arc::new(WhatsAppWebChannel::new(
                        wa.session_path.clone().unwrap_or_default(),
                        wa.pair_phone.clone(),
                        wa.pair_code.clone(),
                        wa.allowed_numbers.clone(),
                    )));
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
        channels.push(Arc::new(EmailChannel::new(email_cfg.clone())));
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
        channels.push(Arc::new(DingTalkChannel::new(
            dt.client_id.clone(),
            dt.client_secret.clone(),
            dt.allowed_users.clone(),
        )));
    }

    if let Some(ref qq) = config.channels_config.qq {
        channels.push(Arc::new(QQChannel::new(
            qq.app_id.clone(),
            qq.app_secret.clone(),
            qq.allowed_users.clone(),
        )));
    }

    if channels.is_empty() {
        println!("No channels configured. Set up channels in the web UI.");
        return Ok(());
    }

    println!("🦀 OpenHuman Channel Server");
    println!("  🤖 Model:    {model}");
    let effective_backend = memory::effective_memory_backend_name(
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

    // Single message bus — all channels send messages here
    let (tx, rx) = tokio::sync::mpsc::channel::<traits::ChannelMessage>(100);

    // Spawn a listener for each channel
    let mut handles = Vec::new();
    for ch in &channels {
        handles.push(spawn_supervised_listener(
            ch.clone(),
            tx.clone(),
            initial_backoff_secs,
            max_backoff_secs,
        ));
    }
    drop(tx); // Drop our copy so rx closes when all channels stop

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
    // Register the tree summarizer event subscriber for observability logging.
    let _tree_summarizer_handle = bus.subscribe(Arc::new(
        crate::openhuman::tree_summarizer::bus::TreeSummarizerEventSubscriber::new(),
    ));

    let max_in_flight_messages = compute_max_in_flight_messages(channels.len());

    println!("  🚦 In-flight message limit: {max_in_flight_messages}");

    let provider_name = providers::INFERENCE_BACKEND_ID.to_string();
    let mut provider_cache_seed: HashMap<String, Arc<dyn Provider>> = HashMap::new();
    provider_cache_seed.insert(provider_name.clone(), Arc::clone(&provider));
    let message_timeout_secs =
        effective_channel_message_timeout_secs(config.channels_config.message_timeout_secs);

    let runtime_ctx = Arc::new(ChannelRuntimeContext {
        channels_by_name,
        provider: Arc::clone(&provider),
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
        provider_cache: Arc::new(Mutex::new(provider_cache_seed)),
        route_overrides: Arc::new(Mutex::new(HashMap::new())),
        api_key: config.api_key.clone(),
        api_url: config.api_url.clone(),
        reliability: Arc::new(config.reliability.clone()),
        provider_runtime_options,
        workspace_dir: Arc::new(config.workspace_dir.clone()),
        message_timeout_secs,
        multimodal: config.multimodal.clone(),
    });

    run_message_dispatch_loop(rx, runtime_ctx, max_in_flight_messages).await;

    // Wait for all channel tasks
    for h in handles {
        let _ = h.await;
    }

    Ok(())
}
