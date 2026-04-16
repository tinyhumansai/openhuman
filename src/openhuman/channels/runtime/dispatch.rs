//! Channel runtime loop and message processing.

use crate::core::event_bus::{
    publish_global, request_native_global, DomainEvent, NativeRequestError,
};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
use crate::openhuman::agent::harness::definition::{
    AgentDefinition, AgentDefinitionRegistry, ToolScope,
};
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
use crate::openhuman::providers::{self, ChatMessage};
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

/// Build a `[CONNECTION_STATE]...[/CONNECTION_STATE]` block listing the
/// current Composio connection status for each connected or available
/// integration.
///
/// Fetches integration state at call time so the agent always sees the
/// up-to-date status for the user's current turn (including connections
/// that completed mid-conversation via OAuth in a browser). The fetch is
/// wrapped in a short timeout so Composio API latency never blocks the
/// channel turn.
///
/// Returns an empty string on any failure (API down, not authenticated,
/// timeout) so the caller can safely append it without branching.
async fn build_connection_state_block() -> String {
    // 3-second ceiling — connection state is best-effort context. If the
    // Composio API is slow, skip the block rather than delaying the turn.
    const COMPOSIO_FETCH_TIMEOUT_SECS: u64 = 3;

    let config = match tokio::time::timeout(
        Duration::from_secs(COMPOSIO_FETCH_TIMEOUT_SECS),
        Config::load_or_init(),
    )
    .await
    {
        Ok(Ok(c)) => c,
        Ok(Err(e)) => {
            tracing::debug!(
                error = %e,
                "[dispatch::connection_state] config load failed — skipping block"
            );
            return String::new();
        }
        Err(_) => {
            tracing::debug!(
                "[dispatch::connection_state] config load timed out — skipping block"
            );
            return String::new();
        }
    };

    let integrations = match tokio::time::timeout(
        Duration::from_secs(COMPOSIO_FETCH_TIMEOUT_SECS),
        fetch_connected_integrations(&config),
    )
    .await
    {
        Ok(list) => list,
        Err(_) => {
            tracing::debug!(
                "[dispatch::connection_state] Composio fetch timed out — skipping block"
            );
            return String::new();
        }
    };

    if integrations.is_empty() {
        tracing::debug!(
            "[dispatch::connection_state] no integrations returned — skipping block"
        );
        return String::new();
    }

    let mut lines = Vec::with_capacity(integrations.len());
    for integration in &integrations {
        let status = if integration.connected {
            // Include account identifier if available (first tool name often encodes it,
            // but the toolkit slug is the clearest label available here).
            format!("connected (toolkit: {})", integration.toolkit)
        } else {
            "not connected".to_string()
        };
        // Capitalize the toolkit name for readability (e.g. "gmail" → "Gmail").
        let display_name = {
            let mut chars = integration.toolkit.chars();
            match chars.next() {
                None => String::new(),
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
            }
        };
        lines.push(format!("{display_name}: {status}"));
    }

    tracing::debug!(
        integration_count = integrations.len(),
        "[dispatch::connection_state] built connection state block for welcome agent"
    );

    format!(
        "\n\n[CONNECTION_STATE]\n{}\n[/CONNECTION_STATE]",
        lines.join("\n")
    )
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
/// The selection is purely a function of
/// `config.chat_onboarding_completed`:
///
/// * **`false`** → route to the `welcome` agent. Welcome's TOML
///   restricts it to two tools (`complete_onboarding`, `memory_recall`)
///   so the LLM cannot accidentally send messages or write files
///   while guiding the user through setup. The welcome agent decides
///   when the user is ready and calls
///   `complete_onboarding(action="complete")`, which flips the flag.
///
/// * **`true`** → route to the `orchestrator` agent. Orchestrator
///   delegates real work to specialist subagents via a `subagents`
///   field in its TOML; this function expands that field into a list
///   of `delegate_*` tools spliced alongside the global registry.
///
/// We deliberately read `chat_onboarding_completed` and NOT the
/// React-UI-managed `onboarding_completed` flag. The latter is the
/// gate `OnboardingOverlay.tsx` uses to render its full-screen wizard
/// in the Tauri desktop app — by the time a desktop user can type a
/// chat message it's already `true`, so routing on it would mean
/// welcome could never run from the Tauri app. The chat flag is set
/// exclusively by the welcome agent itself when it calls
/// `complete_onboarding(complete)`, so it stays `false` for the
/// user's actual first message regardless of what the React layer
/// did. See `Config::chat_onboarding_completed` rustdoc for the full
/// rationale.
///
/// The next channel message after `complete_onboarding` flips the
/// flag is automatically routed to the orchestrator because
/// `Config::load_or_init()` reads from disk every call (no in-process
/// cache, verified at `config/schema/load.rs:409`), so the new value
/// is observed on the next turn without any explicit handoff event.
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

    let target_id = if config.chat_onboarding_completed {
        "orchestrator"
    } else {
        "welcome"
    };

    tracing::info!(
        channel = %channel,
        target_agent = target_id,
        chat_onboarding_completed = config.chat_onboarding_completed,
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
    let extra_tools = if !definition.subagents.is_empty() {
        let connected = fetch_connected_integrations(&config).await;
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
///   in `extra_tools` (e.g. `research`, `delegate_gmail`).
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
            category_filter: None,
            max_iterations: 8,
            timeout_secs: None,
            sandbox_mode: SandboxMode::None,
            background: false,
            uses_fork_context: false,
            subagents: vec![],
            delegate_name: None,
            source: DefinitionSource::Builtin,
        }
    }

    /// `ToolScope::Wildcard` must yield `None` — the prompt builder
    /// treats `None` as "no filter, every tool visible", which is the
    /// correct behaviour for agents like `skills_agent` that want the
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
    /// This is the welcome agent's path: 2 tools in TOML, no
    /// delegation, no extras → 2 entries in the visibility whitelist.
    #[test]
    fn named_scope_without_extras_returns_named_only() {
        let def = def_with_scope(ToolScope::Named(vec![
            "complete_onboarding".into(),
            "memory_recall".into(),
        ]));
        let set = build_visible_tool_set(&def, &[]).expect("named scope yields Some");
        assert_eq!(set.len(), 2);
        assert!(set.contains("complete_onboarding"));
        assert!(set.contains("memory_recall"));
    }

    /// `ToolScope::Named` with extras returns the union of the TOML
    /// named list and the extras' names. This is the orchestrator's
    /// path: 4 direct tools from the TOML + N synthesised delegation
    /// tools (`research`, `plan`, `delegate_gmail`, …) → all of them
    /// visible to the orchestrator's LLM.
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
    /// tool list, while the latter means "no filter at all". This is
    /// the welcome agent's emergency fallback if its TOML somehow
    /// shipped without any tools.
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
            let safe_err = providers::sanitize_api_error(&err.to_string());
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
                &autosave_key,
                &msg.content,
                crate::openhuman::memory::MemoryCategory::Conversation,
                None,
            )
            .await;
    }

    let enriched_message = if memory_context.is_empty() {
        msg.content.clone()
    } else {
        format!("{memory_context}{}", msg.content)
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
    let (delta_tx, delta_rx) = if use_streaming {
        let (tx, rx) = tokio::sync::mpsc::channel::<String>(64);
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

    // Spawn a task to forward streaming deltas to draft updates
    let draft_updater = if let (Some(mut rx), Some(draft_id_ref), Some(channel_ref)) = (
        delta_rx,
        draft_message_id.as_deref(),
        target_channel.as_ref(),
    ) {
        let channel = Arc::clone(channel_ref);
        let reply_target = msg.reply_target.clone();
        let draft_id = draft_id_ref.to_string();
        Some(tokio::spawn(async move {
            let mut accumulated = String::new();
            while let Some(delta) = rx.recv().await {
                accumulated.push_str(&delta);
                if let Err(e) = channel
                    .update_draft(&reply_target, &draft_id, &accumulated)
                    .await
                {
                    tracing::debug!("Draft update failed: {e}");
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
    // Pick the active agent for this turn (welcome pre-onboarding,
    // orchestrator post) and synthesise its delegation tool surface.
    // Fresh disk read of `Config::onboarding_completed` happens inside
    // `resolve_target_agent` — see the `[dispatch::routing]` traces.
    let scoping = resolve_target_agent(&msg.channel).await;

    // When routing to the welcome agent, inject up-to-date Composio connection
    // state into the last user message so the agent always knows which
    // integrations are live without burning a tool call to check. The block is
    // appended — not prepended — so it does not interfere with memory context
    // that was already prepended to `enriched_message`. Scoped strictly to the
    // welcome agent: orchestrator turns are not annotated.
    if scoping.target_agent_id.as_deref() == Some("welcome") {
        let conn_block = build_connection_state_block().await;
        if !conn_block.is_empty() {
            if let Some(last_user_msg) = history.iter_mut().rev().find(|m| m.role == "user") {
                last_user_msg.content.push_str(&conn_block);
                tracing::debug!(
                    block_chars = conn_block.len(),
                    "[dispatch::connection_state] appended CONNECTION_STATE block to welcome-agent turn"
                );
            }
        }
    }

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
        on_delta: delta_tx,
        target_agent_id: scoping.target_agent_id,
        visible_tool_names: scoping.visible_tool_names,
        extra_tools: scoping.extra_tools,
        on_progress: None,
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
                });
                return;
            }

            let error_response = format!("⚠️ Error: {e}");
            eprintln!(
                "  ❌ LLM error after {}ms: {e}",
                started_at.elapsed().as_millis()
            );
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
mod tests {
    use super::*;

    #[test]
    fn contains_any_hits_at_least_one_word() {
        assert!(contains_any("hello world", &["world"]));
        assert!(contains_any("hello world", &["not there", "world"]));
    }

    #[test]
    fn contains_any_returns_false_when_none_match() {
        assert!(!contains_any("hello world", &["nope"]));
        assert!(!contains_any("hello world", &[]));
    }

    #[test]
    fn starts_with_any_detects_leading_prefix() {
        assert!(starts_with_any("hello world", &["hello"]));
        assert!(starts_with_any("hey you", &["yo", "hey"]));
    }

    #[test]
    fn starts_with_any_returns_false_when_none_match() {
        assert!(!starts_with_any("bonjour", &["hello", "hey"]));
        assert!(!starts_with_any("x", &[]));
    }

    // ── select_acknowledgment_reaction ────────────────────────────

    fn is_in(emoji: &str, options: &[&str]) -> bool {
        options.contains(&emoji)
    }

    #[test]
    fn ack_reaction_gratitude_category() {
        for msg in ["thanks a lot", "Thank you", "THX friend", "I appreciate it"] {
            let r = select_acknowledgment_reaction(msg);
            assert!(is_in(r, &["❤️", "🙏"]), "`{msg}` → {r}");
        }
    }

    #[test]
    fn ack_reaction_celebration_category() {
        for msg in ["amazing job", "this is awesome", "incredible!!"] {
            let r = select_acknowledgment_reaction(msg);
            assert!(is_in(r, &["🔥", "🎉"]), "`{msg}` → {r}");
        }
    }

    #[test]
    fn ack_reaction_crypto_category() {
        for msg in ["BTC price today", "ETH pump", "gm on the defi timeline"] {
            let r = select_acknowledgment_reaction(msg);
            assert!(is_in(r, &["💯", "⚡"]), "`{msg}` → {r}");
        }
    }

    #[test]
    fn ack_reaction_technical_category() {
        for msg in ["deploy the api", "debug this code", "rust question"] {
            let r = select_acknowledgment_reaction(msg);
            assert!(is_in(r, &["👨‍💻", "🤓"]), "`{msg}` → {r}");
        }
    }

    #[test]
    fn ack_reaction_greeting_category() {
        for msg in ["hi there", "hello", "hey friend", "yo"] {
            let r = select_acknowledgment_reaction(msg);
            assert!(is_in(r, &["🤗", "😁"]), "`{msg}` → {r}");
        }
    }

    #[test]
    fn ack_reaction_question_category() {
        for msg in [
            "what is this?",
            "how does it work",
            "can you help",
            "is this correct",
        ] {
            let r = select_acknowledgment_reaction(msg);
            assert!(is_in(r, &["🤔", "✍️"]), "`{msg}` → {r}");
        }
    }

    #[test]
    fn ack_reaction_default_category() {
        let r = select_acknowledgment_reaction("the task is running");
        assert!(is_in(r, &["👀", "✍️"]));
    }

    #[test]
    fn ack_reaction_is_deterministic() {
        let a = select_acknowledgment_reaction("thanks");
        let b = select_acknowledgment_reaction("thanks");
        assert_eq!(a, b, "same input should always yield same reaction");
    }

    #[test]
    fn ack_reaction_handles_empty_input_without_panic() {
        // `content.chars().next()` is None on empty input — must not panic.
        let r = select_acknowledgment_reaction("");
        assert!(!r.is_empty());
    }

    #[test]
    fn ack_reaction_handles_single_char() {
        let r = select_acknowledgment_reaction("?");
        // Single "?" falls into question category (contains '?').
        assert!(is_in(r, &["🤔", "✍️"]));
    }
}
