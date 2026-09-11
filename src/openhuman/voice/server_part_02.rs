
// ── Background processing (free functions, spawnable) ─────────────────

/// Capture the frontmost app name at hotkey press so insertion can be validated later.
#[cfg(target_os = "macos")]
fn capture_expected_app_name() -> Option<String> {
    match accessibility::focused_text_context_verbose() {
        Ok(ctx) => {
            let app = ctx
                .app_name
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty());
            if let Some(app_name) = app {
                debug!("{LOG_PREFIX} captured focused app on press: '{app_name}'");
                Some(app_name.to_string())
            } else {
                debug!("{LOG_PREFIX} focus query returned no app name on press");
                None
            }
        }
        Err(e) => {
            warn!("{LOG_PREFIX} failed to capture focused app on press: {e}");
            None
        }
    }
}

#[cfg(not(target_os = "macos"))]
fn capture_expected_app_name() -> Option<String> {
    None
}

/// Build the STT initial_prompt from custom dictionary + recent transcripts.
async fn build_initial_prompt(
    config: &VoiceServerConfig,
    recent_transcripts: &Mutex<Vec<String>>,
) -> Option<String> {
    let mut parts: Vec<String> = Vec::new();

    if !config.custom_dictionary.is_empty() {
        parts.push(config.custom_dictionary.join(", "));
    }

    let recent = recent_transcripts.lock().await;
    if !recent.is_empty() {
        parts.push(recent.join(" "));
    }

    if parts.is_empty() {
        return None;
    }

    let mut prompt = parts.join(". ");
    if prompt.chars().count() > MAX_INITIAL_PROMPT_CHARS {
        prompt = prompt.chars().take(MAX_INITIAL_PROMPT_CHARS).collect();
        if let Some(last_space) = prompt.rfind(' ') {
            prompt.truncate(last_space);
        }
    }
    debug!(
        "{LOG_PREFIX} built initial_prompt ({} chars): '{}'",
        prompt.len(),
        truncate_for_log(&prompt, 100)
    );
    Some(prompt)
}

/// Add a transcript to the rolling recent buffer.
async fn push_recent_transcript(recent_transcripts: &Mutex<Vec<String>>, text: &str) {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return;
    }
    let mut recent = recent_transcripts.lock().await;
    recent.push(trimmed.to_string());
    while recent.len() > MAX_RECENT_TRANSCRIPTS {
        recent.remove(0);
    }
}

/// Process a completed recording in the background.
///
/// This is a free function (not `&self`) so it can be spawned via
/// `tokio::spawn` without blocking the hotkey event loop. All shared
/// state is passed as `Arc` handles.
#[allow(clippy::too_many_arguments)]
async fn process_recording_bg(
    pipeline_id: &str,
    handle: RecordingHandle,
    config: &Config,
    server_config: &VoiceServerConfig,
    state: Arc<Mutex<ServerState>>,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    session_generation: Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
    last_error: Arc<Mutex<Option<String>>>,
    recent_transcripts: Arc<Mutex<Vec<String>>>,
    expected_app: Option<String>,
) {
    let pipeline_started = Instant::now();
    info!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=start generation={generation}");
    update_state_if_current(
        &state,
        &session_generation,
        generation,
        ServerState::Transcribing,
        "transcribing",
    )
    .await;

    let stop_started = Instant::now();
    match handle.stop(config).await {
        Ok(result) => {
            let stop_elapsed = stop_started.elapsed();
            info!(
                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=stop_recording duration={:.1}s bytes={} peak_rms={:.6} stop_elapsed_ms={}",
                result.duration_secs,
                result.wav_bytes.len(),
                result.peak_rms,
                stop_elapsed.as_millis()
            );

            // Gate 1: minimum duration.
            if result.duration_secs < server_config.min_duration_secs {
                warn!(
                    "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_duration DROPPED ({:.1}s < {:.1}s min)",
                    result.duration_secs,
                    server_config.min_duration_secs
                );
                update_state_if_current(
                    &state,
                    &session_generation,
                    generation,
                    ServerState::Idle,
                    "idle_after_short_recording",
                )
                .await;
                return;
            }

            // Gate 2: silence detection.
            if result.peak_rms < server_config.silence_threshold {
                warn!(
                    "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_silence DROPPED (peak_rms={:.6} < threshold={:.6})",
                    result.peak_rms,
                    server_config.silence_threshold
                );
                update_state_if_current(
                    &state,
                    &session_generation,
                    generation,
                    ServerState::Idle,
                    "idle_after_silence",
                )
                .await;
                return;
            }

            // Build initial_prompt from dictionary + recent transcripts.
            let initial_prompt = build_initial_prompt(server_config, &recent_transcripts).await;
            let context = initial_prompt
                .as_deref()
                .or(server_config.context.as_deref());
            if let Some(app) = expected_app.as_deref() {
                debug!("{LOG_PREFIX} [pipeline={pipeline_id}] insertion target: app='{app}'");
            } else {
                debug!("{LOG_PREFIX} [pipeline={pipeline_id}] insertion target unknown");
            }

            info!(
                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=transcribe skip_cleanup={} context={}",
                server_config.skip_cleanup,
                context.map_or("none".to_string(), |c| format!("{}chars", c.len()))
            );

            let transcribe_started = Instant::now();
            match crate::openhuman::voice::voice_transcribe_bytes(
                config,
                &result.wav_bytes,
                Some("wav".to_string()),
                context,
                server_config.skip_cleanup,
            )
            .await
            {
                Ok(outcome) => {
                    let transcribe_elapsed = transcribe_started.elapsed();
                    let text = &outcome.value.text;
                    info!(
                        "{LOG_PREFIX} [pipeline={pipeline_id}] stage=transcription_result text='{}' chars={} elapsed_ms={}",
                        truncate_for_log(text, 80),
                        text.len(),
                        transcribe_elapsed.as_millis()
                    );

                    // Gate 3: filter hallucinated/blank output.
                    //
                    // Falls OPEN when the module cannot be reached. Dictation
                    // inserts into the user's active text field, so a stray
                    // "Thank you." is visible and undoable, while a filter that
                    // failed closed would swallow real dictation with no trace.
                    let hallucinated =
                        match is_hallucinated(config, text, HallucinationMode::Dictation).await {
                            Ok(verdict) => verdict,
                            Err(error) => {
                                warn!(
                                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_hallucination \
                                     UNAVAILABLE ({error}); passing text through"
                            );
                                false
                            }
                        };
                    if hallucinated {
                        warn!(
                            "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_hallucination DROPPED text='{}'",
                            truncate_for_log(text, 60)
                        );
                        update_state_if_current(
                            &state,
                            &session_generation,
                            generation,
                            ServerState::Idle,
                            "idle_after_hallucination",
                        )
                        .await;
                        return;
                    }

                    if !text.trim().is_empty() {
                        push_recent_transcript(&recent_transcripts, text).await;

                        // When the Tauri app itself is focused, deliver via
                        // Socket.IO so the frontend inserts into the chat.
                        // Otherwise paste via OS-level Cmd+V into the
                        // external app.
                        let is_self = expected_app
                            .as_deref()
                            .map(|app| app.to_lowercase().contains("openhuman"))
                            .unwrap_or(false);

                        if is_self {
                            let receivers =
                                super::dictation_listener::publish_transcription(text.to_string());
                            transcription_count.fetch_add(1, Ordering::Relaxed);
                            info!(
                                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_socketio receivers={receivers} total_pipeline_ms={}",
                                pipeline_started.elapsed().as_millis()
                            );
                        } else {
                            let insert_started = Instant::now();
                            if let Err(e) = text_input::insert_text(text, expected_app.as_deref()) {
                                warn!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_paste FAILED: {e}");
                                *last_error.lock().await = Some(e);
                            } else {
                                let insert_elapsed = insert_started.elapsed();
                                transcription_count.fetch_add(1, Ordering::Relaxed);
                                info!(
                                    "{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_paste insert_ms={} total_pipeline_ms={}",
                                    insert_elapsed.as_millis(),
                                    pipeline_started.elapsed().as_millis()
                                );
                            }
                        }
                    } else {
                        warn!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_empty DROPPED (transcription was blank)");
                    }
                }
                Err(e) => {
                    // Windows DLL-not-found errors are classified at the
                    // subprocess layer and logged with a 5-minute backoff.
                    // Demote to warn! so they don't flood Sentry (issue #5168).
                    if e.contains("STATUS_DLL_NOT_FOUND") {
                        warn!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=transcribe DLL_UNAVAILABLE: {e}");
                    } else {
                        warn!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=transcribe FAILED: {e}");
                    }
                    *last_error.lock().await = Some(e);
                }
            }
        }
        Err(e) => {
            warn!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=stop_recording FAILED: {e}");
            *last_error.lock().await = Some(e);
        }
    }

    info!(
        "{LOG_PREFIX} [pipeline={pipeline_id}] stage=done total_pipeline_ms={}",
        pipeline_started.elapsed().as_millis()
    );
    update_state_if_current(
        &state,
        &session_generation,
        generation,
        ServerState::Idle,
        "idle_after_processing",
    )
    .await;
}

async fn update_state_if_current(
    state: &Arc<Mutex<ServerState>>,
    session_generation: &Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
    next_state: ServerState,
    reason: &str,
) {
    let latest_generation = session_generation.load(Ordering::Relaxed);
    if latest_generation != generation {
        debug!(
            "{LOG_PREFIX} skipped stale state update reason={} generation={} latest_generation={} next_state={next_state:?}",
            reason,
            generation,
            latest_generation
        );
        return;
    }

    debug!(
        "{LOG_PREFIX} state update reason={} generation={} next_state={next_state:?}",
        reason, generation
    );
    *state.lock().await = next_state;
}

/// Global voice server instance, lazily initialized.
static VOICE_SERVER: once_cell::sync::OnceCell<Arc<VoiceServer>> = once_cell::sync::OnceCell::new();

/// Get or initialize the global voice server instance.
pub fn global_server(config: VoiceServerConfig) -> Arc<VoiceServer> {
    VOICE_SERVER
        .get_or_init(|| Arc::new(VoiceServer::new(config)))
        .clone()
}

/// Get the global voice server if already initialized.
pub fn try_global_server() -> Option<Arc<VoiceServer>> {
    VOICE_SERVER.get().cloned()
}

/// Start the embedded global voice server when config enables auto-start.
///
/// This is intended for core process startup. The server runs in the background
/// and reuses the process-global singleton so RPC status/stop calls continue to
/// operate on the same instance.
pub async fn start_if_enabled(app_config: &Config) {
    if !app_config.voice_server.auto_start {
        info!("{LOG_PREFIX} auto-start disabled in config, skipping embedded voice server");
        return;
    }

    let server_config = VoiceServerConfig {
        hotkey: app_config.voice_server.hotkey.clone(),
        activation_mode: match app_config.voice_server.activation_mode {
            crate::openhuman::config::VoiceActivationMode::Tap => ActivationMode::Tap,
            crate::openhuman::config::VoiceActivationMode::Push => ActivationMode::Push,
        },
        skip_cleanup: app_config.voice_server.skip_cleanup,
        context: None,
        min_duration_secs: app_config.voice_server.min_duration_secs,
        silence_threshold: app_config.voice_server.silence_threshold,
        custom_dictionary: app_config.voice_server.custom_dictionary.clone(),
    };

    if let Some(existing) = try_global_server() {
        let status = existing.status().await;
        if status.state != ServerState::Stopped {
            info!(
                "{LOG_PREFIX} embedded voice server already running: hotkey={} mode={:?}",
                status.hotkey, status.activation_mode
            );
            return;
        }
    }

    info!(
        "{LOG_PREFIX} auto-start enabled, launching embedded voice server: hotkey={} mode={:?}",
        server_config.hotkey, server_config.activation_mode
    );

    let server = global_server(server_config);
    let config_for_run = app_config.clone();
    let server_for_err = server.clone();
    tokio::spawn(async move {
        if let Err(e) = server.run(&config_for_run).await {
            warn!("{LOG_PREFIX} embedded voice server exited with error: {e}");
            server_for_err.set_last_error(&e).await;
        }
    });
}

/// Run the voice server standalone (blocking). Intended for CLI usage.
///
/// Creates a fresh `VoiceServer` that is **not** registered in the global
/// singleton used by `voice_server_status` RPC. This keeps CLI-started
/// instances isolated from the core RPC lifecycle.
pub async fn run_standalone(
    app_config: Config,
    server_config: VoiceServerConfig,
) -> Result<(), String> {
    info!("{LOG_PREFIX} starting standalone voice server");
    info!("{LOG_PREFIX} hotkey: {}", server_config.hotkey);
    info!("{LOG_PREFIX} mode: {:?}", server_config.activation_mode);
    info!("{LOG_PREFIX} press the hotkey to start dictating");

    let server = VoiceServer::new(server_config);

    // Handle Ctrl+C gracefully.
    let server_arc = Arc::new(server);
    let server_for_signal = server_arc.clone();

    tokio::spawn(async move {
        if let Ok(()) = tokio::signal::ctrl_c().await {
            info!("{LOG_PREFIX} Ctrl+C received, shutting down");
            server_for_signal.stop().await;
        }
    });

    // This is safe because we hold the Arc and nothing else moves it.
    // The server.run() borrows &self, and we await it to completion.
    server_arc.run(&app_config).await
}

// Hallucination detection is now in the shared `hallucination` module.
use crate::openhuman::modules::voice::{is_hallucinated, HallucinationMode};

fn truncate_for_log(s: &str, max: usize) -> String {
    let truncated: String = s.chars().take(max).collect();
    if truncated.len() < s.len() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
