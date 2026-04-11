//! Channel runtime loop and message processing.

use crate::core::event_bus::{
    publish_global, request_native_global, DomainEvent, NativeRequestError,
};
use crate::openhuman::agent::bus::{AgentTurnRequest, AgentTurnResponse, AGENT_RUN_TURN_METHOD};
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
use crate::openhuman::providers::{self, ChatMessage};
use crate::openhuman::util::truncate_with_ellipsis;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio_util::sync::CancellationToken;

/// Maximum characters shown in the debug reply println. Large enough to not truncate
/// real responses while keeping terminal output readable.
const REPLY_LOG_TRUNCATE_CHARS: usize = 200;

fn channel_delivery_instructions(channel_name: &str) -> Option<&'static str> {
    match channel_name {
        "telegram" => Some(
            "When responding on Telegram you may send media attachments using markers: \
            [IMAGE:<url>], [DOCUMENT:<url>], [VIDEO:<url>], [AUDIO:<url>], [VOICE:<url>]. \
            You may also react to the user's message by placing [REACTION:<emoji>] at the \
            very start of your reply. The reaction replaces the automatic acknowledgment \
            the user already saw. Choose based on actual message intent — for example: \
            👍 agreement · ❤️ warmth/thanks · 🔥 excitement · 🤔 careful thought · \
            🤯 surprise · 💯 strong agreement · ⚡ urgency · 👨‍💻 technical topic · \
            🎉 celebration · 🙏 gratitude. \
            A reaction can be combined with a reply: [REACTION:🔥] Here's what I found… \
            Only react when it genuinely fits — skip it for neutral factual responses.",
        ),
        _ => None,
    }
}

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

    if let Some(instructions) = channel_delivery_instructions(&msg.channel) {
        history.push(ChatMessage::system(instructions));
    }

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
