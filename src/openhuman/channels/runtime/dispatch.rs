//! Channel runtime loop and message processing.

use crate::core::event_bus::{
    publish_global, request_native_global, DomainEvent, NativeRequestError,
};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, ToolScope,
};
use crate::openhuman::agent::progress::AgentProgress;
use crate::openhuman::channels::context::{
    build_memory_context, compact_sender_history, conversation_history_key,
    conversation_memory_key, is_context_window_overflow_error, ChannelRuntimeContext,
    CHANNEL_TYPING_REFRESH_INTERVAL_SECS, MAX_CHANNEL_HISTORY,
};
use crate::openhuman::channels::routes::{
    get_or_create_provider, get_route_selection, handle_runtime_command_if_needed,
};
use crate::openhuman::channels::traits;
use crate::openhuman::channels::{Channel, SendMessage};
use crate::openhuman::composio::fetch_connected_integrations;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::{self, ChatMessage};
use crate::openhuman::tools::{orchestrator_tools, Tool};
use crate::openhuman::util::truncate_with_ellipsis;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Maximum characters shown in the debug reply println. Large enough to not truncate
/// real responses while keeping terminal output readable.
const REPLY_LOG_TRUNCATE_CHARS: usize = 200;

/// Returns `true` if `s` contains any of the given substrings.
#[inline]
fn contains_any(s: &str, words: &[&str]) -> bool {
    words.iter().any(|w| s.contains(w))
}

/// Returns `true` if `s` starts with any of the given prefixes.
#[inline]
fn starts_with_any(s: &str, prefixes: &[&str]) -> bool {
    prefixes.iter().any(|p| s.starts_with(p))
}

/// Build the per-turn `[Channel context]` block prepended to the user
/// message for non-web inbound channels (e.g. Telegram, Discord, Slack).
///
/// Surfaces the active channel and reply target so the model knows
/// where it is talking and can route any tool side-effects (notably
/// `cron_add`) back to the same chat instead of defaulting to the
/// in-app web stream. See issue #928.
///
/// Returns an empty string for web/cli turns (the desktop UI is the
/// default delivery surface, no hint needed).
fn build_channel_context_block(msg: &traits::ChannelMessage) -> String {
    let channel = msg.channel.trim();
    if channel.is_empty()
        || channel.eq_ignore_ascii_case("web")
        || channel.eq_ignore_ascii_case("cli")
    {
        return String::new();
    }

    let reply_target = msg.reply_target.trim();
    if reply_target.is_empty() {
        return String::new();
    }

    format!(
        "[Channel context]\n\
         You are responding via the \"{channel}\" channel. Reply target: \"{reply_target}\".\n\
         For any cron/scheduled reminder you create with `cron_add`, set `delivery` to \
         `{{ \"mode\": \"announce\", \"channel\": \"{channel}\", \"to\": \"{reply_target}\" }}` \
         so the reminder is delivered back here instead of the in-app web stream. \
         Only fall back to the default proactive delivery if the user explicitly asks for \
         in-app/desktop notification.\n\n"
    )
}

/// Pick a contextual acknowledgment emoji for an inbound message.
///
/// Intent categories are checked in priority order. Within each category two
/// emoji options are defined; a cheap deterministic index (based on message
/// length + first char value) selects between them so that similar messages
/// don't always produce the identical reaction.
///
/// All emojis used here are in Telegram's standard (non-premium) reaction set.
fn select_acknowledgment_reaction(content: &str) -> &'static str {
    let l = content.to_lowercase();

    // Deterministic variant (0 or 1) — avoids true randomness while giving variety.
    let v = content
        .len()
        .wrapping_add(content.chars().next().map_or(0, |c| c as usize))
        & 1;

    let opts: &[&str] = if contains_any(&l, &["thank", "thx", "appreciate", "grateful", "cheers"]) {
        // Gratitude
        &["❤️", "🙏"]
    } else if contains_any(
        &l,
        &[
            "amazing",
            "awesome",
            "incredible",
            "love it",
            "congrat",
            "!!",
        ],
    ) {
        // Excitement / celebration
        &["🔥", "🎉"]
    } else if contains_any(
        &l,
        &[
            "price", "btc", "eth", "crypto", "trade", "pump", "dump", "market", "token", "wallet",
            "defi", "nft", "sol", "bnb",
        ],
    ) {
        // Crypto / finance
        &["💯", "⚡"]
    } else if contains_any(
        &l,
        &[
            "code",
            "function",
            "api",
            "deploy",
            "build",
            "debug",
            "script",
            "git",
            "rust",
            "python",
            "js",
            "typescript",
        ],
    ) {
        // Technical / dev
        &["👨‍💻", "🤓"]
    } else if starts_with_any(
        &l,
        &[
            "hi",
            "hello",
            "hey",
            "sup",
            "good morning",
            "good evening",
            "good afternoon",
        ],
    ) || l == "yo"
        || l.starts_with("yo ")
    {
        // Greeting
        &["🤗", "😁"]
    } else if l.contains('?')
        || starts_with_any(
            &l,
            &[
                "how",
                "what",
                "why",
                "when",
                "where",
                "who",
                "can you",
                "could you",
                "would you",
                "is ",
                "are ",
                "do you",
                "does",
            ],
        )
    {
        // Question / help request
        &["🤔", "✍️"]
    } else {
        // Default — "seen, on it"
        &["👀", "✍️"]
    };

    opts[v % opts.len()]
}

fn log_worker_join_result(result: Result<(), tokio::task::JoinError>) {
    if let Err(error) = result {
        tracing::error!("Channel message worker crashed: {error}");
    }
}

fn spawn_scoped_typing_task(
    channel: Arc<dyn Channel>,
    recipient: String,
    cancellation_token: CancellationToken,
) -> tokio::task::JoinHandle<()> {
    let stop_signal = cancellation_token;
    let refresh_interval = Duration::from_secs(CHANNEL_TYPING_REFRESH_INTERVAL_SECS);
    let handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = stop_signal.cancelled() => break,
                _ = tokio::time::sleep(refresh_interval) => {
                    if let Err(e) = channel.start_typing(&recipient).await {
                        tracing::debug!("Failed to start typing on {}: {e}", channel.name());
                    }
                }
            }
        }

        if let Err(e) = channel.stop_typing(&recipient).await {
            tracing::debug!("Failed to stop typing on {}: {e}", channel.name());
        }
    });

    handle
}

/// Per-turn scoping fields derived from the active agent definition.
///
/// Carries the three new fields that get spliced into [`AgentTurnRequest`]
/// in [`process_channel_message`]. Constructed by [`resolve_target_agent`]
/// after reading `config.onboarding_completed`, looking up the matching
/// definition in [`AgentDefinitionRegistry`], and synthesising any
/// per-turn delegation tools the agent needs.
struct AgentScoping {
    target_agent_id: Option<String>,
    visible_tool_names: Option<HashSet<String>>,
    extra_tools: Vec<Box<dyn Tool>>,
}

impl AgentScoping {
    /// Empty scoping — preserves the legacy "every tool in the global
    /// registry is visible" behaviour. Returned when the registry isn't
    /// initialised yet (early startup) or when the target agent
    /// definition isn't found, so the channel layer never crashes the
    /// runtime over a routing miss.
    fn unscoped() -> Self {
        Self {
            target_agent_id: None,
            visible_tool_names: None,
            extra_tools: Vec::new(),
        }
    }
}

/// Decide which agent should run for this channel turn and build the
/// matching tool-scoping payload.
///
/// All channel turns route directly to the `orchestrator` agent. The
/// welcome agent has been removed; the Joyride walkthrough in the
/// frontend handles onboarding UI instead.
///
/// On any failure path (missing registry, missing definition, missing
/// orchestrator delegation targets) the function logs and returns
/// [`AgentScoping::unscoped`], which lets the turn run with the legacy
/// unfiltered behaviour rather than failing the whole message.
async fn resolve_target_agent(channel: &str) -> AgentScoping {
    let config = match Config::load_or_init().await {
        Ok(c) => c,
        Err(err) => {
            tracing::warn!(
                channel = %channel,
                error = %err,
                "[dispatch::routing] failed to load config — falling back to unscoped turn"
            );
            return AgentScoping::unscoped();
        }
    };

    let target_id = "orchestrator";

    tracing::info!(
        channel = %channel,
        target_agent = target_id,
        ui_onboarding_completed = config.onboarding_completed,
        "[dispatch::routing] selected target agent"
    );

    let registry = match AgentDefinitionRegistry::global() {
        Some(reg) => reg,
        None => {
            tracing::warn!(
                channel = %channel,
                target_agent = target_id,
                "[dispatch::routing] AgentDefinitionRegistry not initialised — falling back to unscoped turn"
            );
            return AgentScoping::unscoped();
        }
    };

    let definition = match registry.get(target_id) {
        Some(def) => def,
        None => {
            tracing::warn!(
                channel = %channel,
                target_agent = target_id,
                "[dispatch::routing] target agent not in registry — falling back to unscoped turn"
            );
            return AgentScoping::unscoped();
        }
    };

    // Synthesise per-turn delegation tools when the target agent has a
    // `subagents = [...]` field. Today only the orchestrator does, but
    // the helper is agent-agnostic so future agents that delegate
    // (e.g. a custom workspace-override planner that subdivides work)
    // pick this up for free.
    //
    // Wrap the Composio fetch in a 3-second timeout so a slow/unresponsive
    // Composio API can never block turn dispatch indefinitely.
    const COMPOSIO_FETCH_TIMEOUT_SECS: u64 = 3;
    let extra_tools = if !definition.subagents.is_empty() {
        let connected = match tokio::time::timeout(
            Duration::from_secs(COMPOSIO_FETCH_TIMEOUT_SECS),
            fetch_connected_integrations(&config),
        )
        .await
        {
            Ok(list) => list,
            Err(_) => {
                tracing::warn!(
                    channel = %channel,
                    target_agent = target_id,
                    "[dispatch::routing] Composio fetch timed out after {}s — proceeding without connected integrations",
                    COMPOSIO_FETCH_TIMEOUT_SECS
                );
                Vec::new()
            }
        };
        tracing::debug!(
            channel = %channel,
            target_agent = target_id,
            connected_integration_count = connected.len(),
            "[dispatch::routing] fetched connected integrations for delegation expansion"
        );
        orchestrator_tools::collect_orchestrator_tools(definition, registry, &connected)
    } else {
        Vec::new()
    };

    let visible_tool_names = build_visible_tool_set(definition, &extra_tools);

    tracing::debug!(
        channel = %channel,
        target_agent = target_id,
        named_tool_count = match &definition.tools {
            ToolScope::Named(names) => names.len(),
            ToolScope::Wildcard => 0,
        },
        extra_tool_count = extra_tools.len(),
        visible_tool_count = visible_tool_names.as_ref().map(|s| s.len()).unwrap_or(0),
        "[dispatch::routing] assembled tool scoping for turn"
    );

    AgentScoping {
        target_agent_id: Some(target_id.to_string()),
        visible_tool_names,
        extra_tools,
    }
}

/// Build the visible-tool whitelist for an agent.
///
/// The set is the union of:
/// * every tool name in the agent's `[tools] named = [...]` list
///   (when the scope is [`ToolScope::Named`]); and
/// * every name produced by the per-turn synthesised delegation tools
///   in `extra_tools` (e.g. `research`, `plan`,
///   `delegate_to_integrations_agent`).
///
/// When the agent's tool scope is [`ToolScope::Wildcard`] **and** there
/// are no `extra_tools`, returns `None` to preserve the legacy
/// "everything visible" semantics — a `Wildcard` agent that delegates
/// nothing should still see the full registry. When `Wildcard` is
/// combined with non-empty extras (an unusual but legal combination),
/// the legacy unfiltered behaviour also wins because the wildcard
/// implicitly covers anything in the registry plus the extras.
fn build_visible_tool_set(
    definition: &AgentDefinition,
    extra_tools: &[Box<dyn Tool>],
) -> Option<HashSet<String>> {
    match &definition.tools {
        ToolScope::Wildcard => None,
        ToolScope::Named(names) => {
            let mut set: HashSet<String> = names.iter().cloned().collect();
            for tool in extra_tools {
                set.insert(tool.name().to_string());
            }
            Some(set)
        }
    }
}

#[cfg(test)]
mod scoping_tests {
    //! Pure-function unit tests for the agent-scoping helpers added by
    //! the #525/#526 fix. These exercise the synchronous logic without
    //! touching the real `Config::load_or_init` disk read or the global
    //! `AgentDefinitionRegistry`, so they can run in any environment.
    //!
    //! End-to-end exercise of the dispatch path is covered by the
    //! existing `runtime_dispatch::dispatch_routes_through_agent_run_turn_
    //! bus_handler` integration test, which still passes after the new
    //! fields landed (the resolver gracefully falls back to
    //! `AgentScoping::unscoped()` when no orchestrator is registered in
    //! the test environment).

    use super::*;
    use crate::openhuman::agent::harness::definition::{
        DefinitionSource, ModelSpec, PromptSource, SandboxMode,
    };
    use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
    use async_trait::async_trait;

    /// Minimal owned tool stub — just enough for `build_visible_tool_set`
    /// to read its `name()`.
    struct StubTool {
        name: &'static str,
    }

    #[async_trait]
    impl Tool for StubTool {
        fn name(&self) -> &str {
            self.name
        }
        fn description(&self) -> &str {
            "stub"
        }
        fn parameters_schema(&self) -> serde_json::Value {
            serde_json::json!({"type": "object"})
        }
        fn category(&self) -> ToolCategory {
            ToolCategory::System
        }
        fn permission_level(&self) -> PermissionLevel {
            PermissionLevel::None
        }
        async fn execute(&self, _args: serde_json::Value) -> anyhow::Result<ToolResult> {
            Ok(ToolResult::success("ok"))
        }
    }

    fn def_with_scope(scope: ToolScope) -> AgentDefinition {
        AgentDefinition {
            id: "test_agent".into(),
            when_to_use: "test".into(),
            display_name: None,
            system_prompt: PromptSource::Inline(String::new()),
            omit_identity: true,
            omit_memory_context: true,
            omit_safety_preamble: true,
            omit_skills_catalog: true,
            omit_profile: true,
            omit_memory_md: true,
            model: ModelSpec::Inherit,
            temperature: 0.4,
            tools: scope,
            disallowed_tools: vec![],
            skill_filter: None,
            extra_tools: vec![],
            max_iterations: 8,
            max_result_chars: None,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            subagents: vec![],
            delegate_name: None,
            agent_tier: crate::openhuman::agent::harness::definition::AgentTier::Worker,
            source: DefinitionSource::Builtin,
        }
    }

    /// `ToolScope::Wildcard` must yield `None` — the prompt builder
    /// treats `None` as "no filter, every tool visible", which is the
    /// correct behaviour for agents like `integrations_agent` that want the
    /// full skill-category catalogue. Even when extras are present, a
    /// wildcard agent should not start filtering.
    #[test]
    fn wildcard_scope_yields_none_filter() {
        let def = def_with_scope(ToolScope::Wildcard);
        let extras: Vec<Box<dyn Tool>> = vec![Box::new(StubTool { name: "research" })];
        assert!(build_visible_tool_set(&def, &extras).is_none());
        assert!(build_visible_tool_set(&def, &[]).is_none());
    }

    /// `ToolScope::Named` with no extras returns exactly the named set.
    /// For agents with a narrow tool scope (e.g. 2 tools in TOML,
    /// no delegation, no extras) → 2 entries in the visibility whitelist.
    #[test]
    fn named_scope_without_extras_returns_named_only() {
        let def = def_with_scope(ToolScope::Named(vec![
            "memory_recall".into(),
            "ask_user_clarification".into(),
        ]));
        let set = build_visible_tool_set(&def, &[]).expect("named scope yields Some");
        assert_eq!(set.len(), 2);
        assert!(set.contains("memory_recall"));
        assert!(set.contains("ask_user_clarification"));
    }

    /// `ToolScope::Named` with extras returns the union of the TOML
    /// named list and the extras' names. This is the orchestrator's
    /// path: direct tools from the TOML + the synthesised delegation
    /// tools (`research`, `plan`, `delegate_to_integrations_agent`)
    /// → all of them visible to the orchestrator's LLM. The stub
    /// names in this test are arbitrary; they exercise the union
    /// logic, not the real synthesiser.
    #[test]
    fn named_scope_with_extras_returns_union() {
        let def = def_with_scope(ToolScope::Named(vec![
            "query_memory".into(),
            "ask_user_clarification".into(),
            "spawn_subagent".into(),
        ]));
        let extras: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool { name: "research" }),
            Box::new(StubTool {
                name: "delegate_gmail",
            }),
            Box::new(StubTool {
                name: "delegate_github",
            }),
        ];
        let set = build_visible_tool_set(&def, &extras).expect("named scope yields Some");
        assert_eq!(set.len(), 6);
        assert!(set.contains("query_memory"));
        assert!(set.contains("ask_user_clarification"));
        assert!(set.contains("spawn_subagent"));
        assert!(set.contains("research"));
        assert!(set.contains("delegate_gmail"));
        assert!(set.contains("delegate_github"));
    }

    /// Empty `Named` list with extras still yields `Some` containing
    /// just the extras — useful for hypothetical agents that only
    /// reach the world via delegation, with no direct tools.
    #[test]
    fn empty_named_with_extras_returns_extras_only() {
        let def = def_with_scope(ToolScope::Named(vec![]));
        let extras: Vec<Box<dyn Tool>> = vec![Box::new(StubTool {
            name: "delegate_only",
        })];
        let set = build_visible_tool_set(&def, &extras).expect("named scope yields Some");
        assert_eq!(set.len(), 1);
        assert!(set.contains("delegate_only"));
    }

    /// Empty `Named` list with no extras yields an empty `Some(set)` —
    /// effectively "no tools visible". The prompt loop's `is_visible`
    /// helper treats `Some(empty)` differently from `None`: the former
    /// means "filter active, nothing matches" so the LLM gets an empty
    /// tool list, while the latter means "no filter at all".
    #[test]
    fn empty_named_with_no_extras_returns_empty_set() {
        let def = def_with_scope(ToolScope::Named(vec![]));
        let set = build_visible_tool_set(&def, &[]).expect("named scope yields Some");
        assert!(set.is_empty());
    }

    /// Duplicate names across named + extras are de-duplicated by the
    /// HashSet — no double-counting if a workspace override happens to
    /// list a delegation tool name in the direct `named` list too.
    #[test]
    fn duplicate_names_across_named_and_extras_are_deduplicated() {
        let def = def_with_scope(ToolScope::Named(vec![
            "research".into(),
            "query_memory".into(),
        ]));
        let extras: Vec<Box<dyn Tool>> = vec![
            Box::new(StubTool { name: "research" }), // collides with named
            Box::new(StubTool { name: "plan" }),
        ];
        let set = build_visible_tool_set(&def, &extras).expect("named scope yields Some");
        assert_eq!(set.len(), 3);
        assert!(set.contains("research"));
        assert!(set.contains("query_memory"));
        assert!(set.contains("plan"));
    }

    /// `AgentScoping::unscoped` is the safe-fallback constructor used
    /// when the registry is uninitialised or the target agent isn't
    /// found. All three fields must default to "no scoping applied"
    /// so the channel turn runs with the legacy unfiltered behaviour.
    #[test]
    fn agent_scoping_unscoped_has_no_filter_or_extras() {
        let scoping = AgentScoping::unscoped();
        assert!(scoping.target_agent_id.is_none());
        assert!(scoping.visible_tool_names.is_none());
        assert!(scoping.extra_tools.is_empty());
    }
}

pub(crate) async fn process_channel_message(
    ctx: Arc<ChannelRuntimeContext>,
    msg: traits::ChannelMessage,
) {
    println!(
        "  💬 [{}] from {}: {}",
        msg.channel,
        msg.sender,
        truncate_with_ellipsis(&msg.content, 80)
    );

    publish_global(DomainEvent::ChannelMessageReceived {
        channel: msg.channel.clone(),
        message_id: msg.id.clone(),
        sender: msg.sender.clone(),
        reply_target: msg.reply_target.clone(),
        content: msg.content.clone(),
        thread_ts: msg.thread_ts.clone(),
        workspace_dir: ctx.workspace_dir.as_ref().clone(),
    });

    let target_channel = ctx.channels_by_name.get(&msg.channel).cloned();
    if handle_runtime_command_if_needed(ctx.as_ref(), &msg, target_channel.as_ref()).await {
        return;
    }

    // Fire typing indicator as early as possible — before any async I/O — so the
    // user sees feedback immediately regardless of how fast the LLM responds.
    if let Some(channel) = target_channel.as_ref() {
        if let Err(e) = channel.start_typing(&msg.reply_target).await {
            tracing::debug!(
                "[dispatch] Early typing start failed on {}: {e}",
                channel.name()
            );
        }
    }

    // Send a smart acknowledgment reaction immediately so the user knows the message
    // was received and understood. The LLM may override this later by including its
    // own [REACTION:...] marker, which Telegram replaces atomically.
    if let Some(channel) = target_channel.as_ref() {
        if channel.supports_reactions() && msg.thread_ts.is_some() {
            let ack_emoji = select_acknowledgment_reaction(&msg.content);
            tracing::debug!(
                channel = msg.channel,
                emoji = ack_emoji,
                "[dispatch] Sending acknowledgment reaction"
            );
            let react_content = format!("[REACTION:{ack_emoji}]");
            let channel_for_react = Arc::clone(channel);
            let react_msg =
                SendMessage::new(react_content, &msg.reply_target).in_thread(msg.thread_ts.clone());
            tokio::spawn(async move {
                if let Err(e) = channel_for_react.send(&react_msg).await {
                    tracing::debug!("[dispatch] Acknowledgment reaction failed: {e}");
                }
            });
        }
    }

    let history_key = conversation_history_key(&msg);
    let route = get_route_selection(ctx.as_ref(), &history_key);
    let active_provider = match get_or_create_provider(ctx.as_ref(), &route.provider).await {
        Ok(provider) => provider,
        Err(err) => {
            crate::core::observability::report_error(
                &err,
                "channels",
                "provider_init",
                &[
                    ("channel", msg.channel.as_str()),
                    ("provider", route.provider.as_str()),
                ],
            );
            let safe_err = provider::sanitize_api_error(&err.to_string());
            let message = format!(
                "⚠️ Failed to initialize provider `{}`. Please run `/models` to choose another provider.\nDetails: {safe_err}",
                route.provider
            );
            if let Some(channel) = target_channel.as_ref() {
                let _ = channel
                    .send(
                        &SendMessage::new(message, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone()),
                    )
                    .await;
            }
            return;
        }
    };

    let memory_context =
        build_memory_context(ctx.memory.as_ref(), &msg.content, ctx.min_relevance_score).await;

    if ctx.auto_save_memory {
        let autosave_key = conversation_memory_key(&msg);
        let _ = ctx
            .memory
            .store(
                "",
                &autosave_key,
                &msg.content,
                crate::openhuman::memory::MemoryCategory::Conversation,
                None,
            )
            .await;
    }

    let channel_context = build_channel_context_block(&msg);
    let enriched_message = match (memory_context.is_empty(), channel_context.is_empty()) {
        (true, true) => msg.content.clone(),
        (false, true) => format!("{memory_context}{}", msg.content),
        (true, false) => format!("{channel_context}{}", msg.content),
        (false, false) => format!("{memory_context}{channel_context}{}", msg.content),
    };

    println!("  ⏳ Processing message...");
    let started_at = Instant::now();

    // Build history from per-sender conversation cache
    let mut prior_turns = ctx
        .conversation_histories
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&history_key)
        .cloned()
        .unwrap_or_default();

    let mut history = vec![ChatMessage::system(ctx.system_prompt.as_str())];
    history.append(&mut prior_turns);
    history.push(ChatMessage::user(&enriched_message));

    // Determine if this channel supports streaming draft updates
    let use_streaming = target_channel
        .as_ref()
        .is_some_and(|ch| ch.supports_draft_updates());

    // Set up streaming channel if supported
    let (progress_tx, progress_rx) = if use_streaming {
        let (tx, rx) = tokio::sync::mpsc::channel::<AgentProgress>(64);
        (Some(tx), Some(rx))
    } else {
        (None, None)
    };

    // Send initial draft message if streaming
    let draft_message_id = if use_streaming {
        if let Some(channel) = target_channel.as_ref() {
            match channel
                .send_draft(
                    &SendMessage::new("...", &msg.reply_target).in_thread(msg.thread_ts.clone()),
                )
                .await
            {
                Ok(id) => id,
                Err(e) => {
                    tracing::debug!("Failed to send draft on {}: {e}", channel.name());
                    None
                }
            }
        } else {
            None
        }
    } else {
        None
    };

    // Spawn a task to forward streaming progress to draft updates
    let draft_updater = if let (Some(mut rx), Some(draft_id_ref), Some(channel_ref)) = (
        progress_rx,
        draft_message_id.as_deref(),
        target_channel.as_ref(),
    ) {
        let channel = Arc::clone(channel_ref);
        let reply_target = msg.reply_target.clone();
        let draft_id = draft_id_ref.to_string();
        Some(tokio::spawn(async move {
            let mut accumulated = String::new();
            let mut last_thinking_update = None;
            const THINKING_UPDATE_INTERVAL_MS: u128 = 2000;

            while let Some(progress) = rx.recv().await {
                match progress {
                    AgentProgress::TextDelta { delta, .. } => {
                        accumulated.push_str(&delta);
                        if let Err(e) = channel
                            .update_draft(&reply_target, &draft_id, &accumulated)
                            .await
                        {
                            tracing::debug!("Draft update failed: {e}");
                        }
                    }
                    AgentProgress::ThinkingDelta { .. } => {
                        // Suppress thinking text to Telegram; only show a placeholder if we haven't
                        // started receiving the final answer yet.
                        if accumulated.is_empty() {
                            let now = std::time::Instant::now();
                            let should_update = match last_thinking_update {
                                None => true,
                                Some(last) => {
                                    now.duration_since(last).as_millis()
                                        > THINKING_UPDATE_INTERVAL_MS
                                }
                            };

                            if should_update {
                                if let Err(e) = channel
                                    .update_draft(&reply_target, &draft_id, "Thinking...")
                                    .await
                                {
                                    tracing::debug!("Thinking update failed: {e}");
                                }
                                last_thinking_update = Some(now);
                            }
                        }
                    }
                    AgentProgress::ToolCallStarted { tool_name, .. } => {
                        if accumulated.is_empty() {
                            let _ = channel
                                .update_draft(
                                    &reply_target,
                                    &draft_id,
                                    &format!("Working ({})...", tool_name),
                                )
                                .await;
                        }
                    }
                    _ => {}
                }
            }
        }))
    } else {
        None
    };

    let typing_cancellation = target_channel.as_ref().map(|_| CancellationToken::new());
    // Typing was already started early (before memory/provider setup). Here we only
    // spawn the background refresh task that keeps the indicator alive during long turns.
    let typing_task = match (target_channel.as_ref(), typing_cancellation.as_ref()) {
        (Some(channel), Some(token)) => Some(spawn_scoped_typing_task(
            Arc::clone(channel),
            msg.reply_target.clone(),
            token.clone(),
        )),
        _ => None,
    };

    // Dispatch the agentic turn through the native event bus instead of
    // calling `run_tool_call_loop` directly. The agent domain registers
    // an `agent.run_turn` handler at startup (see
    // `crate::openhuman::agent::bus::register_agent_handlers`); this keeps
    // the channel layer free of direct harness imports and makes the
    // agent side mockable in unit tests via a handler override.
    //
    // The agent handler owns the history vector — we `mem::take` the
    // local one to avoid an unnecessary clone; `history` is not read
    // again below.
    // Pick the active agent for this turn (always orchestrator) and
    // synthesise its delegation tool surface. Fresh disk read of
    // `Config::onboarding_completed` happens inside `resolve_target_agent`.
    let scoping = resolve_target_agent(&msg.channel).await;

    // A channel's explicitly-registered `tools_registry` tools are always visible
    // to the model. The resolved agent's visible-tool scope is meant to filter the
    // ambient/builtin tool surface, not to hide tools the channel deliberately
    // handed in for this turn. Without this, a channel that provides a tool
    // outside the resolved agent's `Named` scope (e.g. a test mock, or a custom
    // channel-specific tool) would be filtered out and surfaced to the model as
    // "unknown tool". When the scope is `Wildcard` (`None`), no filter applies.
    let visible_tool_names = scoping.visible_tool_names.map(|mut set| {
        for tool in ctx.tools_registry.iter() {
            set.insert(tool.name().to_string());
        }
        set
    });

    let turn_request = AgentTurnRequest {
        provider: Arc::clone(&active_provider),
        history: std::mem::take(&mut history),
        tools_registry: Arc::clone(&ctx.tools_registry),
        provider_name: route.provider.clone(),
        model: route.model.clone(),
        temperature: ctx.temperature,
        silent: true,
        channel_name: msg.channel.clone(),
        multimodal: ctx.multimodal.clone(),
        max_tool_iterations: ctx.max_tool_iterations,
        on_delta: None, // on_progress handles text deltas now
        target_agent_id: scoping.target_agent_id,
        visible_tool_names,
        extra_tools: scoping.extra_tools,
        on_progress: progress_tx,
    };
    tracing::debug!(
        channel = %msg.channel,
        provider = %route.provider,
        model = %route.model,
        "[channels::dispatch] dispatching {AGENT_RUN_TURN_METHOD} via native bus"
    );
    let llm_result = tokio::time::timeout(Duration::from_secs(ctx.message_timeout_secs), async {
        request_native_global::<AgentTurnRequest, AgentTurnResponse>(
            AGENT_RUN_TURN_METHOD,
            turn_request,
        )
        .await
        .map(|resp| resp.text)
        .map_err(|err| match err {
            // Unwrap handler-returned errors so the underlying
            // message (e.g. "Agent exceeded maximum tool iterations")
            // flows through without being wrapped in bus-transport
            // layer prose. The error-formatting path downstream
            // treats this `anyhow::Error` the same way it did before
            // the bus migration.
            NativeRequestError::HandlerFailed { message, .. } => {
                anyhow::anyhow!(message)
            }
            // Bus-level errors (UnregisteredHandler / TypeMismatch /
            // NotInitialized) surface with their full Display so
            // startup wiring bugs are immediately obvious in logs.
            other => anyhow::anyhow!("[agent.run_turn dispatch] {other}"),
        })
    })
    .await;

    // Wait for draft updater to finish
    if let Some(handle) = draft_updater {
        let _ = handle.await;
    }

    if let Some(token) = typing_cancellation.as_ref() {
        token.cancel();
    }
    if let Some(handle) = typing_task {
        log_worker_join_result(handle.await);
    }

    let (success, response_text) = match llm_result {
        Ok(Ok(response)) => {
            // Save user + assistant turn to per-sender history
            {
                let mut histories = ctx
                    .conversation_histories
                    .lock()
                    .unwrap_or_else(|e| e.into_inner());
                let turns = histories.entry(history_key).or_default();
                turns.push(ChatMessage::user(&enriched_message));
                turns.push(ChatMessage::assistant(&response));
                // Trim to MAX_CHANNEL_HISTORY (keep recent turns)
                while turns.len() > MAX_CHANNEL_HISTORY {
                    turns.remove(0);
                }
            }
            println!(
                "  🤖 Reply ({}ms): {}",
                started_at.elapsed().as_millis(),
                truncate_with_ellipsis(&response, REPLY_LOG_TRUNCATE_CHARS)
            );
            if let Some(channel) = target_channel.as_ref() {
                if let Some(ref draft_id) = draft_message_id {
                    if let Err(e) = channel
                        .finalize_draft(
                            &msg.reply_target,
                            draft_id,
                            &response,
                            msg.thread_ts.as_deref(),
                        )
                        .await
                    {
                        tracing::warn!("Failed to finalize draft: {e}; sending as new message");
                        let _ = channel
                            .send(
                                &SendMessage::new(&response, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                } else if let Err(e) = channel
                    .send(
                        &SendMessage::new(&response, &msg.reply_target)
                            .in_thread(msg.thread_ts.clone()),
                    )
                    .await
                {
                    eprintln!("  ❌ Failed to reply on {}: {e}", channel.name());
                }
            }
            (true, response)
        }
        Ok(Err(e)) => {
            if is_context_window_overflow_error(&e) {
                let compacted = compact_sender_history(ctx.as_ref(), &history_key);
                let error_text = if compacted {
                    "⚠️ Context window exceeded for this conversation. I compacted recent history and kept the latest context. Please resend your last message."
                } else {
                    "⚠️ Context window exceeded for this conversation. Please resend your last message."
                };
                eprintln!(
                    "  ⚠️ Context window exceeded after {}ms; sender history compacted={}",
                    started_at.elapsed().as_millis(),
                    compacted
                );
                if let Some(channel) = target_channel.as_ref() {
                    if let Some(ref draft_id) = draft_message_id {
                        let _ = channel
                            .finalize_draft(
                                &msg.reply_target,
                                draft_id,
                                error_text,
                                msg.thread_ts.as_deref(),
                            )
                            .await;
                    } else {
                        let _ = channel
                            .send(
                                &SendMessage::new(error_text, &msg.reply_target)
                                    .in_thread(msg.thread_ts.clone()),
                            )
                            .await;
                    }
                }

                publish_global(DomainEvent::ChannelMessageProcessed {
                    channel: msg.channel.clone(),
                    message_id: msg.id.clone(),
                    sender: msg.sender.clone(),
                    reply_target: msg.reply_target.clone(),
                    content: msg.content.clone(),
                    thread_ts: msg.thread_ts.clone(),
                    response: error_text.to_string(),
                    elapsed_ms: started_at.elapsed().as_millis() as u64,
                    success: false,
                    workspace_dir: ctx.workspace_dir.as_ref().clone(),
                });
                return;
            }

            let error_response = format!("⚠️ Error: {e}");
            eprintln!(
                "  ❌ LLM error after {}ms: {e}",
                started_at.elapsed().as_millis()
            );
            // The typed `AgentError` is flattened to a `String` at the
            // native-bus boundary (`agent::bus` map_err → `e.to_string()`),
            // so the downcast that works in `Agent::run_single` is not an
            // option here — fall back to canonical-phrase substring match.
            // The max-tool-iterations cap is a deterministic agent-state
            // outcome and is already surfaced to the user as the
            // chat-rendered "⚠️ Error: …" message just above. Skip the
            // Sentry funnel (OPENHUMAN-TAURI-98) and emit `log::info!`
            // instead — `Err` propagation through the surrounding match
            // arm is unchanged.
            if crate::openhuman::agent::error::is_max_iterations_error(&e.to_string()) {
                log::info!(
                    target: "channels",
                    "[channels.dispatch] suppressed Sentry emission for max-iteration cap \
                     channel={} provider={} message={}",
                    msg.channel.as_str(),
                    route.provider.as_str(),
                    e
                );
            } else {
                // Route through `report_error_or_expected` so
                // transient-upstream provider HTTP errors that bubbled
                // up via `agent.run_single` (`OpenHuman API error
                // (502 Bad Gateway): …`) get demoted via
                // `is_transient_upstream_http_message` — the agent
                // re-emit at the dispatch layer was previously
                // unconditionally calling `report_error`, which firehoses
                // Sentry under `domain="channels"` even though the same
                // chain was already classified at the provider + agent
                // layers (OPENHUMAN-TAURI-4F ~157ev / -1C ~87ev / -8F
                // ~39ev: provider 5xx that the reliable layer retried
                // and exhausted, then the channels layer re-reported as
                // a fresh per-attempt event). Genuine bugs (404 / 500
                // / unrelated agent failures) still surface — the
                // classifier only demotes the canonical transient
                // shapes documented in
                // `crate::core::observability::expected_error_kind`.
                crate::core::observability::report_error_or_expected(
                    &e,
                    "channels",
                    "dispatch_llm_error",
                    &[
                        ("channel", msg.channel.as_str()),
                        ("provider", route.provider.as_str()),
                    ],
                );
            }
            if let Some(channel) = target_channel.as_ref() {
                if let Some(ref draft_id) = draft_message_id {
                    let _ = channel
                        .finalize_draft(
                            &msg.reply_target,
                            draft_id,
                            &error_response,
                            msg.thread_ts.as_deref(),
                        )
                        .await;
                } else {
                    let _ = channel
                        .send(
                            &SendMessage::new(&error_response, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
            }
            (false, error_response)
        }
        Err(_) => {
            let timeout_msg = format!("LLM response timed out after {}s", ctx.message_timeout_secs);
            eprintln!(
                "  ❌ {} (elapsed: {}ms)",
                timeout_msg,
                started_at.elapsed().as_millis()
            );
            crate::core::observability::report_error(
                timeout_msg.as_str(),
                "channels",
                "dispatch_llm_timeout",
                &[
                    ("channel", msg.channel.as_str()),
                    ("timeout_secs", &ctx.message_timeout_secs.to_string()),
                ],
            );
            let error_text =
                "⚠️ Request timed out while waiting for the model. Please try again.".to_string();
            if let Some(channel) = target_channel.as_ref() {
                if let Some(ref draft_id) = draft_message_id {
                    let _ = channel
                        .finalize_draft(
                            &msg.reply_target,
                            draft_id,
                            &error_text,
                            msg.thread_ts.as_deref(),
                        )
                        .await;
                } else {
                    let _ = channel
                        .send(
                            &SendMessage::new(&error_text, &msg.reply_target)
                                .in_thread(msg.thread_ts.clone()),
                        )
                        .await;
                }
            }
            (false, error_text)
        }
    };

    publish_global(DomainEvent::ChannelMessageProcessed {
        channel: msg.channel.clone(),
        message_id: msg.id.clone(),
        sender: msg.sender.clone(),
        reply_target: msg.reply_target.clone(),
        content: msg.content.clone(),
        thread_ts: msg.thread_ts.clone(),
        response: response_text,
        elapsed_ms: started_at.elapsed().as_millis() as u64,
        success,
        workspace_dir: ctx.workspace_dir.as_ref().clone(),
    });
}

pub(crate) async fn run_message_dispatch_loop(
    mut rx: tokio::sync::mpsc::Receiver<traits::ChannelMessage>,
    ctx: Arc<ChannelRuntimeContext>,
    max_in_flight_messages: usize,
) {
    let semaphore = Arc::new(tokio::sync::Semaphore::new(max_in_flight_messages));
    let mut workers = tokio::task::JoinSet::new();

    while let Some(msg) = rx.recv().await {
        let permit = match Arc::clone(&semaphore).acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => break,
        };

        let worker_ctx = Arc::clone(&ctx);
        workers.spawn(async move {
            let _permit = permit;
            process_channel_message(worker_ctx, msg).await;
        });

        while let Some(result) = workers.try_join_next() {
            log_worker_join_result(result);
        }
    }

    while let Some(result) = workers.join_next().await {
        log_worker_join_result(result);
    }
}

#[cfg(test)]
#[path = "dispatch_tests.rs"]
mod tests;
