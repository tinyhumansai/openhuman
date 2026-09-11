use crate::openhuman::config::Config;
use crate::openhuman::modules::voice as tinyvoice;
use crate::openhuman::voice::audio_capture::TARGET_SAMPLE_RATE;
use std::sync::atomic::{AtomicBool, Ordering};

const LOG_PREFIX: &str = "[voice::always_on]";

/// How long to wait before retrying a VAD session that would not open.
///
/// Long enough that a persistently unavailable module does not produce a call
/// per audio chunk, short enough that a module which finishes downloading
/// mid-session starts segmenting without the user restarting anything.
const SESSION_RETRY_INTERVAL: std::time::Duration = std::time::Duration::from_secs(30);

/// How many raw capture chunks may queue for the processor.
///
/// The channel was unbounded, which was survivable when the callback did the
/// downmix and resample itself: the processor's work was pure arithmetic and it
/// always outran the microphone. It no longer does — each chunk now costs three
/// module round trips — so a stalled or slow processor could let the queue grow
/// without limit behind a producer that never blocks.
///
/// A few seconds of chunks at typical `cpal` buffer sizes. Deliberately a
/// *count* rather than a byte budget: the callback must not do arithmetic to
/// decide whether to send.
const CAPTURE_QUEUE_CHUNKS: usize = 256;

/// Chunks the capture callback had to drop because the queue was full.
///
/// A process-wide counter rather than closure state: the callback is built once
/// per sample format and each closure must stay `Fn`, so the count cannot live
/// in a captured local. One always-on stream exists per process, so a single
/// counter is not an aggregation of unrelated streams.
static DROPPED_CHUNKS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// One chunk of raw capture, exactly as the device delivered it.
///
/// Interleaved and at the device's own rate: the callback converts the sample
/// format and nothing else, so `channels` and `source_rate` travel with the
/// samples for the processor to hand to the module.
struct RawChunk {
    /// Interleaved `f32` samples.
    samples: Vec<f32>,
}

/// The device format, learned once when the stream is built.
#[derive(Debug, Clone, Copy)]
struct CaptureFormat {
    /// Device sample rate, before resampling to [`TARGET_SAMPLE_RATE`].
    source_rate: u32,
    /// Interleaved channel count.
    channels: u16,
}

/// The capture thread + processor have been spawned (once per process).
static RUNNING: AtomicBool = AtomicBool::new(false);

/// Runtime on/off, mirrors `config.voice_server.always_on_enabled`. Toggling it
/// at runtime takes effect immediately: when false the processor drops all audio
/// (nothing is transcribed or sent). Lets the Settings toggle work without a
/// restart. (The mic stream itself stays open until the next launch.)
static ENABLED: AtomicBool = AtomicBool::new(false);

/// When true, the processor drops audio and resets the segmenter (privacy hook:
/// screen locked). Driven by [`spawn_lock_watcher`] on macOS.
static PAUSED: AtomicBool = AtomicBool::new(false);

/// VAD frame size. 20 ms at 16 kHz = 320 samples — small enough for responsive
/// onset/hangover detection, large enough for a stable RMS estimate.
const FRAME_MS: u32 = 20;
const FRAME_SAMPLES: usize = (TARGET_SAMPLE_RATE as usize / 1000) * FRAME_MS as usize;

/// Hard cap on a buffered utterance (defensive — the segmenter's
/// `max_utterance_ms` should flush first; this bounds memory if it doesn't).
const MAX_UTTERANCE_SAMPLES: usize = TARGET_SAMPLE_RATE as usize * 60;

/// Apply the always-on config: set the runtime ENABLED gate and, when enabled,
/// open the continuous microphone stream (once per process). Safe to call at
/// boot **and** at runtime (the Settings toggle calls it via the config RPC):
/// toggling off flips `ENABLED` so the processor immediately stops transcribing/
/// delivering; toggling on starts capture live without a restart.
///
/// Opens a continuous mic stream, segments it through the `tinyvoice` module, and
/// routes each finished utterance through STT and the dictation delivery bus (so
/// it reaches the agent exactly like a hotkey dictation, and lights up the notch).
pub async fn start_if_enabled(app_config: &Config) {
    let on = app_config.voice_server.always_on_enabled;
    ENABLED.store(on, Ordering::SeqCst);
    if !on {
        log::info!("{LOG_PREFIX} disabled — capture idle (toggle off)");
        return;
    }
    if RUNNING.swap(true, Ordering::SeqCst) {
        log::info!("{LOG_PREFIX} re-enabled; capture already running");
        return;
    }

    let vad = tinyvoice::vad_config_from_server_config(&app_config.voice_server);
    let config = app_config.clone();
    log::info!(
        "{LOG_PREFIX} enabled — onset={:.4} hangover={}ms min_speech={}ms max_utt={}ms",
        vad.onset_threshold,
        vad.hangover_ms,
        vad.min_speech_ms,
        vad.max_utterance_ms
    );

    // The cpal stream is `!Send`, so it lives on a dedicated thread that pushes
    // RAW interleaved chunks over a channel to the async processor below —
    // deliberately raw: every transform now happens off the audio callback.
    // `spawn_capture_thread` blocks on a synchronous readiness handshake while
    // the OS builds the input stream — cold WASAPI init on Windows can take a
    // while — so run it on the blocking pool. This function is polled
    // concurrently with the other login-gated services (#3490), and blocking an
    // async worker here would stall them.
    let (tx, mut rx) = tokio::sync::mpsc::channel::<RawChunk>(CAPTURE_QUEUE_CHUNKS);
    log::debug!(
        "{LOG_PREFIX} starting microphone capture (blocking readiness handshake on the blocking pool)"
    );
    // Distinguish a Tokio join failure (the blocking task itself panicked) from a
    // `spawn_capture_thread` setup error (e.g. no input device), so the log points
    // at the right layer instead of flattening both into one message.
    let format = match tokio::task::spawn_blocking(move || spawn_capture_thread(tx)).await {
        Ok(Ok(format)) => {
            log::debug!("{LOG_PREFIX} microphone capture stream ready");
            format
        }
        Ok(Err(e)) => {
            log::warn!("{LOG_PREFIX} could not start microphone capture: {e}");
            RUNNING.store(false, Ordering::SeqCst);
            return;
        }
        Err(join_err) => {
            log::error!(
                "{LOG_PREFIX} microphone capture setup task failed to join (panicked): {join_err}"
            );
            RUNNING.store(false, Ordering::SeqCst);
            return;
        }
    };

    // Privacy hook: pause capture while the screen is locked.
    spawn_lock_watcher();

    let onset_threshold = vad.onset_threshold;
    tokio::spawn(async move {
        // The segmenter lives in the module now, so opening it can fail for
        // reasons unrelated to audio — a download that has not happened yet, a
        // host with no published artifact.
        //
        // That failure must NOT end this task. The capture thread is already
        // running and owns the microphone for the process lifetime; returning
        // here would clear `RUNNING` while the stream stays live, and the next
        // `start_if_enabled` would sail past the `RUNNING` guard and open a
        // *second* microphone stream. So the session is opened lazily and
        // retried, and audio is dropped until there is one.
        let mut session: Option<tinyvoice::VadSession> = None;
        let mut last_open_attempt: Option<std::time::Instant> = None;

        let mut pending: Vec<f32> = Vec::new();
        let mut utterance: Vec<f32> = Vec::new();
        // Test-build diagnostics: confirm audio actually flows from the mic and
        // surface live input levels vs the onset threshold (every ~5s) so the VAD
        // can be tuned per mic/room without guessing. Levels are loudness, not PII.
        let mut first_chunk_logged = false;
        let mut level_peak: f32 = 0.0;
        let mut level_frames: u32 = 0;
        let mut last_level_log = std::time::Instant::now();

        while let Some(chunk) = rx.recv().await {
            if !first_chunk_logged {
                first_chunk_logged = true;
                log::info!(
                    "{LOG_PREFIX} first audio chunk received from mic (samples={}) — capture pipeline live",
                    chunk.samples.len()
                );
            }
            // Drop audio and abandon any in-flight utterance while paused
            // (screen locked) or toggled off — nothing is captured or sent.
            if PAUSED.load(Ordering::Relaxed) || !ENABLED.load(Ordering::Relaxed) {
                // Reset unconditionally rather than checking `is_speaking`
                // first: that check would be a second bus call to save a cheap
                // idempotent one, and the privacy path should be the shortest
                // path, not the cleverest.
                if let Some(open) = session.as_ref() {
                    if let Err(error) = open.reset(&config).await {
                        // Same reasoning as a failed push: a reset that did not
                        // land leaves a segmenter we cannot vouch for, and this
                        // is the privacy path, so discard it rather than trust
                        // it to have dropped the partial utterance.
                        log::warn!(
                            "{LOG_PREFIX} could not reset the VAD session ({error}); reopening"
                        );
                        session = None;
                        last_open_attempt = Some(std::time::Instant::now());
                    }
                }
                pending.clear();
                utterance.clear();
                continue;
            }

            // Open the segmenter on first use, retrying on a cooldown so a
            // module that becomes available later heals this without a restart.
            if session.is_none() {
                let due = last_open_attempt.is_none_or(|at| at.elapsed() >= SESSION_RETRY_INTERVAL);
                if !due {
                    continue;
                }
                last_open_attempt = Some(std::time::Instant::now());
                match tinyvoice::VadSession::open(&config, vad).await {
                    Ok(opened) => {
                        log::info!("{LOG_PREFIX} VAD session open; segmenting live audio");
                        session = Some(opened);
                    }
                    Err(error) => {
                        log::warn!(
                            "{LOG_PREFIX} could not open a VAD session ({error}); \
                             dropping audio and retrying in {}s",
                            SESSION_RETRY_INTERVAL.as_secs()
                        );
                        continue;
                    }
                }
            }

            // Downmix + resample in the module. This is the work that used to
            // happen inside the cpal callback.
            let mono16k = match tinyvoice::prepare_frames(
                &config,
                &chunk.samples,
                format.source_rate,
                format.channels,
            )
            .await
            {
                Ok(samples) => samples,
                Err(error) => {
                    log::warn!("{LOG_PREFIX} could not prepare capture frames: {error}");
                    continue;
                }
            };
            pending.extend_from_slice(&mono16k);

            // Whole frames only; the remainder stays in `pending` for the next
            // chunk so no audio is dropped at a chunk boundary.
            let whole = pending.len() / FRAME_SAMPLES * FRAME_SAMPLES;
            if whole == 0 {
                continue;
            }
            // Measure BEFORE draining. Draining first and then failing would
            // discard the frames outright — a module hiccup would eat the
            // user's audio rather than delay it.
            let energies =
                match tinyvoice::frame_energies(&config, &pending[..whole], FRAME_SAMPLES as u32)
                    .await
                {
                    Ok(energies) => energies,
                    Err(error) => {
                        log::warn!(
                            "{LOG_PREFIX} could not measure frame energies ({error}); \
                         retrying these frames on the next chunk"
                        );
                        continue;
                    }
                };
            let frames: Vec<f32> = pending.drain(..whole).collect();

            for rms in &energies {
                level_peak = level_peak.max(*rms);
            }
            level_frames += energies.len() as u32;
            if last_level_log.elapsed() >= std::time::Duration::from_secs(5) {
                log::info!(
                    "{LOG_PREFIX} mic level peak_rms={level_peak:.4} onset={onset_threshold:.4} frames={level_frames} ({})",
                    if level_peak >= onset_threshold {
                        "speech would trigger"
                    } else {
                        "below onset — lower vad_onset_threshold or check mic gain"
                    }
                );
                level_peak = 0.0;
                level_frames = 0;
                last_level_log = std::time::Instant::now();
            }

            // One push per chunk rather than per frame: same events, same
            // order, one round trip instead of N.
            let push = match session.as_ref() {
                Some(open) => open.push(&config, FRAME_MS, &energies).await,
                None => continue,
            };
            let events = match push {
                Ok(events) => events,
                Err(error) => {
                    // Drop the handle, do not just skip the chunk. A push fails
                    // when the module went away or the session is no longer
                    // open, and neither heals by itself — keeping the handle
                    // would reuse a dead session forever, because the lazy-open
                    // retry above only runs while `session` is `None`.
                    log::warn!("{LOG_PREFIX} VAD push failed ({error}); reopening the session");
                    session = None;
                    last_open_attempt = Some(std::time::Instant::now());
                    // The partial utterance belonged to the dead segmenter, so
                    // its boundaries mean nothing to the next one.
                    pending.clear();
                    utterance.clear();
                    continue;
                }
            };

            // `frame` indexes `frames`; slice the audio at the same boundaries
            // the segmenter reported so an utterance carries exactly the
            // samples it was measured from.
            let mut cursor = 0usize;
            for indexed in events {
                let frame = indexed.frame;
                match indexed.event {
                    tinyvoice::VadEvent::SpeechStart => {
                        let at = frame * FRAME_SAMPLES;
                        log::info!(
                            "{LOG_PREFIX} speech onset rms={:.4} (onset={onset_threshold:.4})",
                            energies.get(frame).copied().unwrap_or_default()
                        );
                        utterance.clear();
                        cursor = at;
                        notch_status("Listening", 2500); // pill: capturing speech
                    }
                    tinyvoice::VadEvent::SpeechEnd {
                        emit, voiced_ms, ..
                    } => {
                        let upto = ((frame + 1) * FRAME_SAMPLES).min(frames.len());
                        if upto > cursor && utterance.len() < MAX_UTTERANCE_SAMPLES {
                            utterance.extend_from_slice(&frames[cursor..upto]);
                        }
                        cursor = upto;
                        let captured = std::mem::take(&mut utterance);
                        log::info!(
                            "{LOG_PREFIX} utterance end voiced_ms={voiced_ms} emit={emit} samples={}",
                            captured.len()
                        );
                        if emit {
                            let cfg = config.clone();
                            tokio::spawn(async move {
                                transcribe_and_deliver(&cfg, captured).await;
                            });
                        }
                    }
                }
            }

            // Whatever is still open after the reported events belongs to the
            // utterance in progress.
            if cursor < frames.len()
                && !frames.is_empty()
                && utterance.len() < MAX_UTTERANCE_SAMPLES
                && match session.as_ref() {
                    Some(open) => open.is_speaking(&config).await.unwrap_or(false),
                    None => false,
                }
            {
                utterance.extend_from_slice(&frames[cursor..]);
            }
        }

        if let Some(open) = session.as_ref() {
            if let Err(error) = open.close(&config).await {
                log::warn!("{LOG_PREFIX} could not close the VAD session: {error}");
            }
        }
        log::info!("{LOG_PREFIX} capture channel closed; processor exiting");
        RUNNING.store(false, Ordering::SeqCst);
    });
}

/// Disable always-on listening at runtime (logout). Flips the `ENABLED` gate so
/// the processor immediately drops all audio — nothing is transcribed or sent —
/// the symmetric counterpart to [`start_if_enabled`]. The cpal stream itself
/// stays open (it's spawned once per process and reused if the user logs back in
/// and re-enables), but no audio is processed while disabled.
pub fn stop() {
    if ENABLED.swap(false, Ordering::SeqCst) {
        log::info!("{LOG_PREFIX} stopped (logout) — capture idle, audio dropped");
    }
}
/// Push a listener status to the always-visible notch pill via the
/// `overlay:attention` channel. The notch maps "Listening" / "Processing" to the
/// right icon; when the message expires it falls back to "Ready". Fire-and-forget.
fn notch_status(status: &str, ttl_ms: u32) {
    let _ = crate::openhuman::desktop::overlay::publish_attention(
        crate::openhuman::desktop::overlay::OverlayAttentionEvent::new(status)
            .with_source("voice")
            .with_ttl_ms(ttl_ms),
    );
}

/// Transcribe a finished utterance and hand the text to the dictation bus,
/// which delivers it to the agent (auto-send) and the notch — the same path the
/// hotkey dictation uses.
async fn transcribe_and_deliver(config: &Config, samples_16k: Vec<f32>) {
    use base64::Engine as _;
    let sample_count = samples_16k.len();
    let wav = match tinyvoice::encode_wav(config, &samples_16k, TARGET_SAMPLE_RATE).await {
        Ok(wav) => wav,
        Err(error) => {
            log::warn!("{LOG_PREFIX} wav encode failed: {error}");
            return;
        }
    };
    // Route through the *configured* STT provider (cloud / slug) — the
    // same factory dispatch the `voice.stt_dispatch` RPC uses — so always-on
    // honors the user's choice of engine.
    let provider_name = crate::openhuman::voice::effective_stt_provider(config);
    // Which STT backend is doing the work matters when diagnosing slow/failed
    // transcription across machines.
    log::info!(
        "{LOG_PREFIX} transcribing utterance: provider={provider_name} model=<provider default> samples={sample_count} wav_bytes={}",
        wav.len()
    );
    let provider = match crate::openhuman::voice::create_stt_provider(&provider_name, "", config) {
        Ok(p) => p,
        Err(e) => {
            log::warn!("{LOG_PREFIX} STT provider '{provider_name}' unavailable: {e}");
            return;
        }
    };
    let audio_b64 = base64::engine::general_purpose::STANDARD.encode(&wav);
    let stt_started = std::time::Instant::now();
    // Force English transcription. Auto-detect was rendering the English wake
    // word "Hey Tiny" in Hindi/Bengali/etc. script ("हे टाइनी"), which could never
    // match the Latin wake word. The wake word + commands here are English.
    match provider
        .transcribe(
            config,
            &audio_b64,
            Some("audio/wav"),
            Some("utterance.wav"),
            Some("en"),
        )
        .await
    {
        Ok(outcome) => {
            let text = outcome.value.text.trim().to_string();
            log::info!(
                "{LOG_PREFIX} transcription ok in {}ms (provider={provider_name}, chars={})",
                stt_started.elapsed().as_millis(),
                text.len()
            );
            if text.is_empty() {
                log::info!("{LOG_PREFIX} empty transcript dropped");
                return;
            }
            // Wake-word gate: only act on utterances addressed to the agent
            // ("Hey Tiny, …"). Strip the wake phrase and deliver the command.
            // A module failure here must not deliver an unaddressed utterance
            // to the agent: this gate is what keeps a passing conversation out
            // of the assistant, so it fails CLOSED — the opposite of the
            // hallucination filter, where the risk runs the other way.
            let gated =
                match tinyvoice::extract_command(config, &text, &config.voice_server.wake_word)
                    .await
                {
                    Ok(gated) => gated,
                    Err(error) => {
                        log::warn!(
                        "{LOG_PREFIX} wake-word gate unavailable ({error}); dropping the utterance"
                    );
                        return;
                    }
                };
            match gated {
                Some(cmd) => {
                    // Redacted: never log the raw spoken command (always-on mic PII).
                    log::info!("{LOG_PREFIX} wake word matched → cmd_len={}", cmd.len());
                    notch_status("Processing", 12000); // pill: running the command
                    deliver_command(config, cmd).await;
                }
                None => {
                    // Presence is only used to choose between acknowledging and
                    // staying silent, so an error here degrades to silence.
                    let present =
                        tinyvoice::wake_word_present(config, &text, &config.voice_server.wake_word)
                            .await
                            .unwrap_or(false);
                    if present {
                        // Wake word spoken with no trailing command ("Hey Tiny").
                        // Acknowledge with an agent turn so the user gets a reply
                        // instead of silence, then they can follow up.
                        log::info!("{LOG_PREFIX} bare wake word → acknowledging");
                        notch_status("Listening…", 8000);
                        deliver_command(config, "hello".to_string()).await;
                    } else {
                        // Visible at info so the user can see WHAT was heard when the
                        // wake word didn't match (diagnoses "Hey Tiny not responding").
                        log::info!(
                            "{LOG_PREFIX} no wake word ({:?}) in transcript={text:?}; ignored",
                            config.voice_server.wake_word
                        );
                    }
                }
            }
        }
        Err(e) => log::warn!(
            "{LOG_PREFIX} transcription failed ({provider_name}) after {}ms: {e}",
            stt_started.elapsed().as_millis()
        ),
    }
}

/// Route a recognized command: run high-confidence intents locally (the fast
/// path, no LLM turn), and fall back to the agent for `Unknown` — or when a
/// local execution fails, so routing can only shortcut, never drop a command.
async fn deliver_command(config: &Config, cmd: String) {
    use crate::openhuman::modules::voice::{route, VoiceIntent};
    // A module that will not load costs the fast path, not the command: an
    // unroutable transcript goes to the agent, which is exactly what
    // `VoiceIntent::Unknown` already means.
    let intent = match route(config, &cmd).await {
        Ok(intent) => intent,
        Err(error) => {
            log::warn!("{LOG_PREFIX} intent routing unavailable ({error}); deferring to agent");
            VoiceIntent::Unknown
        }
    };
    // Log only the intent *kind* + lengths — never the transcript-derived query /
    // app / result text (always-on mic PII).
    if matches!(intent, VoiceIntent::Unknown) {
        log::info!(
            "{LOG_PREFIX} no fast intent → agent (cmd_len={})",
            cmd.len()
        );
        crate::openhuman::voice::dictation_listener::publish_transcription(cmd);
        return;
    }
    log::info!(
        "{LOG_PREFIX} fast intent={} (local execution)",
        intent.kind()
    );
    match execute_intent(config, intent).await {
        Ok(msg) => {
            log::info!("{LOG_PREFIX} fast route done (summary_len={})", msg.len());
            notch_status(&msg, 2500);
        }
        Err(_e) => {
            log::warn!("{LOG_PREFIX} fast route failed; falling back to agent");
            crate::openhuman::voice::dictation_listener::publish_transcription(cmd);
        }
    }
}

/// Execute a fast-path [`VoiceIntent`] directly (no LLM). Media transport and
/// volume go through `osascript`. App launch and "play X" have no local
/// fast-path (the desktop-control automation backend was removed); those
/// intents return `Err` so the caller defers to the agent (the LLM fallback) —
/// routing can only *shortcut*, never *block*.
async fn execute_intent(
    _config: &Config,
    intent: crate::openhuman::modules::voice::VoiceIntent,
) -> Result<String, String> {
    use crate::openhuman::modules::voice::VoiceIntent as VI;
    match intent {
        VI::Play { .. } => Err("play has no local fast-path; defer to agent".to_string()),
        VI::OpenApp { .. } => Err("app launch has no local fast-path; defer to agent".to_string()),
        VI::Pause => osa("tell application \"Music\" to pause")
            .await
            .map(|_| "Paused".to_string()),
        VI::Resume => osa("tell application \"Music\" to play")
            .await
            .map(|_| "Resumed".to_string()),
        VI::Next => osa("tell application \"Music\" to next track")
            .await
            .map(|_| "Next track".to_string()),
        VI::Previous => osa("tell application \"Music\" to previous track")
            .await
            .map(|_| "Previous track".to_string()),
        VI::SetVolume { percent } => osa(&format!("set volume output volume {percent}"))
            .await
            .map(|_| format!("Volume {percent}%")),
        VI::VolumeUp => {
            osa("set volume output volume (output volume of (get volume settings) + 12)")
                .await
                .map(|_| "Louder".to_string())
        }
        VI::VolumeDown => {
            osa("set volume output volume (output volume of (get volume settings) - 12)")
                .await
                .map(|_| "Quieter".to_string())
        }
        VI::Mute => osa("set volume with output muted")
            .await
            .map(|_| "Muted".to_string()),
        VI::Unmute => osa("set volume without output muted")
            .await
            .map(|_| "Unmuted".to_string()),
        VI::Unknown => Err("unknown intent".to_string()),
    }
}

/// Run a one-line AppleScript (macOS). Used for media transport + volume.
async fn osa(script: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        // Bound the subprocess so a hung osascript can't stall deliver_command
        // (which would block the agent fallback). 5s is ample for a one-liner.
        let out = tokio::time::timeout(
            std::time::Duration::from_secs(5),
            tokio::process::Command::new("osascript")
                .arg("-e")
                .arg(script)
                .output(),
        )
        .await
        .map_err(|_| "osascript timed out".to_string())?
        .map_err(|e| format!("osascript spawn failed: {e}"))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(format!(
                "osascript error: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ))
        }
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = script;
        Err("media/volume control is macOS-only".to_string())
    }
}

/// Spawn the dedicated cpal capture thread. Blocks until the stream is set up
/// (or fails), mirroring `audio_capture::start_recording`'s readiness handshake.
fn spawn_capture_thread(tx: tokio::sync::mpsc::Sender<RawChunk>) -> Result<CaptureFormat, String> {
    let (setup_tx, setup_rx) = std::sync::mpsc::sync_channel::<Result<CaptureFormat, String>>(1);
    std::thread::Builder::new()
        .name("voice-always-on".into())
        .spawn(move || {
            if let Err(e) = capture_on_thread(tx, &setup_tx) {
                log::warn!("{LOG_PREFIX} capture thread error: {e}");
                let _ = setup_tx.send(Err(e));
            }
        })
        .map_err(|e| format!("failed to spawn always-on capture thread: {e}"))?;
    match setup_rx.recv() {
        Ok(Ok(format)) => Ok(format),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("always-on capture thread exited before signalling readiness".to_string()),
    }
}
