//! Background recording → transcription pipeline: runs off the hotkey event
//! loop so rapid consecutive presses are never missed. Owns the gates
//! (min duration, silence, hallucination) and the initial-prompt / recent
//! transcript bookkeeping that improves STT continuity.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;

use log::{debug, info, warn};
use tokio::sync::Mutex;

use crate::config::Config;
use crate::modules::accessibility::{AccessibilityClient, FocusTarget, PasteRequest};
use crate::modules::voice::{
    gate_recording, is_hallucinated, GateVerdict, HallucinationMode, RecordingGates, SessionInput,
};
use crate::voice::audio_capture::RecordingHandle;

use super::recorder::Recorder;
use super::types::{VoiceServerConfig, MAX_INITIAL_PROMPT_CHARS, MAX_RECENT_TRANSCRIPTS};
use super::LOG_PREFIX;

/// Capture the frontmost app name at hotkey press so insertion can be validated later.
#[cfg(target_os = "macos")]
pub(super) async fn capture_expected_app_name(config: &Config) -> Option<String> {
    match AccessibilityClient::new(config)
        .focus(crate::modules::accessibility::FocusQuery { verbose: true })
        .await
    {
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
pub(super) async fn capture_expected_app_name(_config: &Config) -> Option<String> {
    None
}

/// Build the STT initial_prompt from custom dictionary + recent transcripts.
pub(super) async fn build_initial_prompt(
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
pub(super) async fn push_recent_transcript(recent_transcripts: &Mutex<Vec<String>>, text: &str) {
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
pub(super) async fn process_recording_bg(
    pipeline_id: &str,
    handle: RecordingHandle,
    config: &Config,
    server_config: &VoiceServerConfig,
    recorder: Recorder,
    transcription_count: Arc<std::sync::atomic::AtomicU64>,
    generation: u64,
    last_error: Arc<Mutex<Option<String>>>,
    recent_transcripts: Arc<Mutex<Vec<String>>>,
    expected_app: Option<String>,
) {
    let pipeline_started = Instant::now();
    info!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=start generation={generation}");
    recorder
        .advance(config, SessionInput::Transcribing { generation })
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

            // Gates 1 and 2 (minimum duration, silence) run in the module.
            //
            // Falls OPEN when the module cannot be reached, for the same
            // reason as the hallucination gate below.
            let gates = RecordingGates {
                min_duration_secs: server_config.min_duration_secs,
                silence_threshold: server_config.silence_threshold,
            };
            match gate_recording(config, result.duration_secs, result.peak_rms, gates).await {
                Ok(GateVerdict::Pass) => {}
                Ok(verdict) => {
                    warn!(
                        "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_recording DROPPED verdict={verdict:?} duration={:.1}s peak_rms={:.6}",
                        result.duration_secs, result.peak_rms
                    );
                    recorder
                        .advance(config, SessionInput::Finished { generation })
                        .await;
                    return;
                }
                Err(error) => warn!(
                    "{LOG_PREFIX} [pipeline={pipeline_id}] stage=gate_recording UNAVAILABLE ({error}); passing recording through"
                ),
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
            match crate::voice::voice_transcribe_bytes(
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
                        recorder
                            .advance(config, SessionInput::Finished { generation })
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
                            let receivers = crate::voice::dictation_listener::publish_transcription(
                                text.to_string(),
                            );
                            transcription_count.fetch_add(1, Ordering::Relaxed);
                            info!(
                                "{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_socketio receivers={receivers} total_pipeline_ms={}",
                                pipeline_started.elapsed().as_millis()
                            );
                        } else {
                            let insert_started = Instant::now();
                            if let Err(e) = AccessibilityClient::new(config)
                                .paste(PasteRequest {
                                    text: text.to_string(),
                                    target: FocusTarget {
                                        app: expected_app.clone(),
                                        ..Default::default()
                                    },
                                })
                                .await
                            {
                                warn!("{LOG_PREFIX} [pipeline={pipeline_id}] stage=deliver_paste FAILED: {e}");
                                *last_error.lock().await = Some(e.to_string());
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
    recorder
        .advance(config, SessionInput::Finished { generation })
        .await;
}

pub(super) fn truncate_for_log(s: &str, max: usize) -> String {
    let truncated: String = s.chars().take(max).collect();
    if truncated.len() < s.len() {
        format!("{truncated}...")
    } else {
        truncated
    }
}
