//! Phase 2 — always-on listening.
//!
//! Instead of a hotkey gating each recording, always-on mode keeps the mic
//! open continuously and uses **voice-activity detection (VAD)** to carve the
//! audio stream into utterances: an utterance opens when energy rises above an
//! onset threshold and closes after a configurable run of silence (the
//! "hangover"). Each completed utterance is transcribed and pushed onto the
//! dictation bus, so it reaches the agent and the notch exactly like a hotkey
//! dictation.
//!
//! Layers:
//!   - [`VadSegmenter`] — a pure state machine over per-frame RMS energies,
//!     unit-tested deterministically (no audio backend).
//!   - [`start_if_enabled`] — opens a continuous cpal mic stream on a dedicated
//!     thread, slices 16 kHz mono frames, drives the segmenter, transcribes each
//!     utterance via the configured STT provider, then applies the wake-word
//!     gate ([`extract_command`], default "Hey Tiny") before delivering the
//!     command to the agent via `publish_transcription`.
//!   - [`spawn_lock_watcher`] — privacy hook: pauses capture while the screen is
//!     locked (macOS via the Quartz session dictionary).
//!
//! Privacy: always-on is **opt-in** (`config.voice_server.always_on_enabled`,
//! default false) and pauses when the screen is locked.

use crate::openhuman::config::VoiceServerConfig as CfgVoiceServer;

const LOG_PREFIX: &str = "[voice::always_on]";

/// Tuning for the VAD segmenter, distilled from [`CfgVoiceServer`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct VadConfig {
    /// Peak-RMS energy above which a frame counts as speech.
    pub onset_threshold: f32,
    /// How long energy must stay below `onset_threshold` before the current
    /// utterance is closed. Bridges natural mid-sentence pauses.
    pub hangover_ms: u32,
    /// Minimum voiced duration for a segment to be emitted; shorter blips
    /// (cough, door) are dropped.
    pub min_speech_ms: u32,
    /// Hard ceiling on a single utterance — forces a flush so a continuous
    /// noise source can't grow an unbounded recording.
    pub max_utterance_ms: u32,
}

impl VadConfig {
    /// Build VAD tuning from the persisted voice-server config.
    pub fn from_server_config(c: &CfgVoiceServer) -> Self {
        Self {
            onset_threshold: c.vad_onset_threshold,
            hangover_ms: c.vad_hangover_ms,
            min_speech_ms: c.vad_min_speech_ms,
            max_utterance_ms: (c.vad_max_utterance_secs * 1000.0).round().max(1.0) as u32,
        }
    }
}

/// An event emitted by the segmenter as the audio stream is consumed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VadEvent {
    /// Energy crossed the onset threshold — an utterance has begun.
    SpeechStart,
    /// An utterance closed. `voiced_ms` is the accumulated speech duration
    /// (excluding the trailing silence); `emit` is false when it fell below
    /// `min_speech_ms` (drop it); `forced` is true when the close was caused
    /// by the `max_utterance_ms` ceiling rather than a silence hangover.
    SpeechEnd {
        voiced_ms: u32,
        emit: bool,
        forced: bool,
    },
}

#[derive(Debug, Clone, Copy)]
enum State {
    /// No active utterance — waiting for energy to cross the onset threshold.
    Silent,
    /// Inside an utterance.
    Speaking {
        /// Total elapsed time since the utterance opened (voiced + silence).
        total_ms: u32,
        /// Accumulated voiced time (frames above onset).
        voiced_ms: u32,
        /// Consecutive below-onset time since the last voiced frame.
        silence_run_ms: u32,
    },
}

/// Pure VAD state machine. Drive it by calling [`push_frame`](Self::push_frame)
/// with the RMS energy of each fixed-size audio frame; it returns at most one
/// [`VadEvent`] per frame.
#[derive(Debug)]
pub struct VadSegmenter {
    cfg: VadConfig,
    state: State,
}

impl VadSegmenter {
    pub fn new(cfg: VadConfig) -> Self {
        Self {
            cfg,
            state: State::Silent,
        }
    }

    /// True while inside an utterance (between `SpeechStart` and `SpeechEnd`).
    pub fn is_speaking(&self) -> bool {
        matches!(self.state, State::Speaking { .. })
    }

    /// Abort any in-flight utterance and return to the idle state without
    /// emitting an event. Used by the privacy hook (screen lock) and on
    /// stream teardown.
    pub fn reset(&mut self) {
        self.state = State::Silent;
    }

    /// Feed one frame's RMS energy and its duration in milliseconds.
    pub fn push_frame(&mut self, rms: f32, frame_ms: u32) -> Option<VadEvent> {
        let above = rms >= self.cfg.onset_threshold;
        match self.state {
            State::Silent => {
                if above {
                    self.state = State::Speaking {
                        total_ms: frame_ms,
                        voiced_ms: frame_ms,
                        silence_run_ms: 0,
                    };
                    Some(VadEvent::SpeechStart)
                } else {
                    None
                }
            }
            State::Speaking {
                mut total_ms,
                mut voiced_ms,
                mut silence_run_ms,
            } => {
                total_ms = total_ms.saturating_add(frame_ms);
                if above {
                    voiced_ms = voiced_ms.saturating_add(frame_ms);
                    silence_run_ms = 0;
                } else {
                    silence_run_ms = silence_run_ms.saturating_add(frame_ms);
                }

                // Close on a silence hangover.
                if silence_run_ms >= self.cfg.hangover_ms {
                    self.state = State::Silent;
                    let emit = voiced_ms >= self.cfg.min_speech_ms;
                    return Some(VadEvent::SpeechEnd {
                        voiced_ms,
                        emit,
                        forced: false,
                    });
                }
                // Close on the hard utterance ceiling.
                if total_ms >= self.cfg.max_utterance_ms {
                    self.state = State::Silent;
                    let emit = voiced_ms >= self.cfg.min_speech_ms;
                    return Some(VadEvent::SpeechEnd {
                        voiced_ms,
                        emit,
                        forced: true,
                    });
                }

                self.state = State::Speaking {
                    total_ms,
                    voiced_ms,
                    silence_run_ms,
                };
                None
            }
        }
    }
}

// ── Continuous capture loop ─────────────────────────────────────────────────

use crate::openhuman::config::Config;
use crate::openhuman::voice::audio_capture::{
    chunk_rms, encode_wav_16k, resample, to_mono, TARGET_SAMPLE_RATE,
};
use std::sync::atomic::{AtomicBool, Ordering};

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
/// Opens a continuous mic stream, segments it with the [`VadSegmenter`], and
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

    let vad = VadConfig::from_server_config(&app_config.voice_server);
    let config = app_config.clone();
    log::info!(
        "{LOG_PREFIX} enabled — onset={:.4} hangover={}ms min_speech={}ms max_utt={}ms",
        vad.onset_threshold,
        vad.hangover_ms,
        vad.min_speech_ms,
        vad.max_utterance_ms
    );

    // The cpal stream is `!Send`, so it lives on a dedicated thread that pushes
    // 16 kHz mono frames over a channel to the async processor below.
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<Vec<f32>>();
    if let Err(e) = spawn_capture_thread(tx) {
        log::error!("{LOG_PREFIX} could not start microphone capture: {e}");
        RUNNING.store(false, Ordering::SeqCst);
        return;
    }

    // Privacy hook: pause capture while the screen is locked.
    spawn_lock_watcher();

    let onset_threshold = vad.onset_threshold;
    tokio::spawn(async move {
        let mut seg = VadSegmenter::new(vad);
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
                    chunk.len()
                );
            }
            // Drop audio and abandon any in-flight utterance while paused
            // (screen locked) or toggled off — nothing is captured or sent.
            if PAUSED.load(Ordering::Relaxed) || !ENABLED.load(Ordering::Relaxed) {
                if seg.is_speaking() {
                    seg.reset();
                }
                pending.clear();
                utterance.clear();
                continue;
            }
            pending.extend_from_slice(&chunk);
            while pending.len() >= FRAME_SAMPLES {
                let frame: Vec<f32> = pending.drain(..FRAME_SAMPLES).collect();
                let rms = chunk_rms(&frame);
                level_peak = level_peak.max(rms);
                level_frames += 1;
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
                match seg.push_frame(rms, FRAME_MS) {
                    Some(VadEvent::SpeechStart) => {
                        log::info!(
                            "{LOG_PREFIX} speech onset rms={rms:.4} (onset={onset_threshold:.4})"
                        );
                        utterance.clear();
                        utterance.extend_from_slice(&frame);
                        notch_status("Listening", 2500); // pill: capturing speech
                    }
                    Some(VadEvent::SpeechEnd {
                        emit, voiced_ms, ..
                    }) => {
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
                    None => {
                        if seg.is_speaking() && utterance.len() < MAX_UTTERANCE_SAMPLES {
                            utterance.extend_from_slice(&frame);
                        }
                    }
                }
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
    let _ = crate::openhuman::overlay::publish_attention(
        crate::openhuman::overlay::OverlayAttentionEvent::new(status)
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
    let wav = match encode_wav_16k(&samples_16k) {
        Ok(w) => w,
        Err(e) => {
            log::warn!("{LOG_PREFIX} wav encode failed: {e}");
            return;
        }
    };
    // Route through the *configured* STT provider (cloud / whisper / slug) — the
    // same factory dispatch the `voice.stt_dispatch` RPC uses — so always-on
    // honors the user's choice instead of forcing local whisper.
    let provider_name = crate::openhuman::voice::effective_stt_provider(config);
    let model = crate::openhuman::voice::DEFAULT_WHISPER_MODEL.to_string();
    // Which STT backend is doing the work matters when diagnosing slow/failed
    // transcription across machines (local whisper download state vs cloud).
    log::info!(
        "{LOG_PREFIX} transcribing utterance: provider={provider_name} model={model} samples={sample_count} wav_bytes={}",
        wav.len()
    );
    let provider =
        match crate::openhuman::voice::create_stt_provider(&provider_name, &model, config) {
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
            match extract_command(&text, &config.voice_server.wake_word) {
                Some(cmd) => {
                    // Redacted: never log the raw spoken command (always-on mic PII).
                    log::info!("{LOG_PREFIX} wake word matched → cmd_len={}", cmd.len());
                    notch_status("Processing", 12000); // pill: running the command
                    deliver_command(config, cmd).await;
                }
                None => {
                    // Visible at info so the user can see WHAT was heard when the
                    // wake word didn't match (diagnoses "Hey Tiny not responding").
                    log::info!(
                        "{LOG_PREFIX} no wake word ({:?}) in transcript={text:?}; ignored",
                        config.voice_server.wake_word
                    );
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
    use crate::openhuman::voice::command_router::{route, VoiceIntent};
    let intent = route(&cmd);
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
/// volume go through `osascript`; app launch reuses the `launch_app` platform
/// launcher; "play X" runs the `automate` Music fast-path.
async fn execute_intent(
    config: &Config,
    intent: crate::openhuman::voice::command_router::VoiceIntent,
) -> Result<String, String> {
    use crate::openhuman::voice::command_router::VoiceIntent as VI;
    match intent {
        VI::Play { query } => {
            let backend =
                crate::openhuman::accessibility::automate::RealBackend::new(config.clone());
            let out = crate::openhuman::accessibility::automate::run(
                "Music",
                &format!("play {query}"),
                &backend,
                crate::openhuman::accessibility::automate::AutomateOptions::default(),
            )
            .await;
            if out.success {
                Ok(out.summary)
            } else {
                Err(out.summary)
            }
        }
        VI::OpenApp { app } => {
            crate::openhuman::tools::implementations::system::launch_platform(&app).await
        }
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

/// Apply the wake-word gate to a transcript.
///
/// Returns the command to send to the agent (the text after the wake phrase),
/// or `None` when the wake word isn't present (the utterance wasn't addressed to
/// the agent). An empty `wake_word` disables the gate (every utterance passes).
/// Matching is tolerant: case-insensitive, punctuation-insensitive, and the
/// phrase may appear after leading filler ("um, hey tiny, play music").
pub(crate) fn extract_command(transcript: &str, wake_word: &str) -> Option<String> {
    let tokens = |s: &str| -> Vec<String> {
        s.to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { ' ' })
            .collect::<String>()
            .split_whitespace()
            .map(String::from)
            .collect()
    };
    let wake = tokens(wake_word);
    let t = tokens(transcript);
    if wake.is_empty() {
        // No wake word configured → deliver everything (non-empty).
        return if t.is_empty() {
            None
        } else {
            Some(t.join(" "))
        };
    }

    // Anchor on the most distinctive (longest) wake token, e.g. "tiny" — STT
    // mangles the greeting ("hey"→"a"/"ok") and the exact spelling
    // ("tiny"→"tony"/"tinny"), so fuzzy-match the anchor near the start and take
    // everything after it as the command. Bounded to the first 3 tokens to avoid
    // mid-sentence false triggers.
    let anchor = wake.iter().max_by_key(|w| w.len()).cloned().unwrap();
    let max_dist = if anchor.chars().count() <= 4 { 1 } else { 2 };
    for i in 0..t.len().min(3) {
        if levenshtein(&t[i], &anchor) <= max_dist {
            let cmd = t[i + 1..].join(" ");
            return if cmd.trim().is_empty() {
                None // wake word alone, no command
            } else {
                Some(cmd)
            };
        }
    }
    None
}

/// Classic Levenshtein edit distance (small inputs — wake-word tokens).
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    let mut cur = vec![0usize; b.len() + 1];
    for (i, ca) in a.iter().enumerate() {
        cur[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let cost = if ca == cb { 0 } else { 1 };
            cur[j + 1] = (prev[j + 1] + 1).min(cur[j] + 1).min(prev[j] + cost);
        }
        std::mem::swap(&mut prev, &mut cur);
    }
    prev[b.len()]
}

/// Spawn the dedicated cpal capture thread. Blocks until the stream is set up
/// (or fails), mirroring `audio_capture::start_recording`'s readiness handshake.
fn spawn_capture_thread(tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>) -> Result<(), String> {
    let (setup_tx, setup_rx) = std::sync::mpsc::sync_channel::<Result<(), String>>(1);
    std::thread::Builder::new()
        .name("voice-always-on".into())
        .spawn(move || {
            if let Err(e) = capture_on_thread(tx, &setup_tx) {
                log::error!("{LOG_PREFIX} capture thread error: {e}");
                let _ = setup_tx.send(Err(e));
            }
        })
        .map_err(|e| format!("failed to spawn always-on capture thread: {e}"))?;
    match setup_rx.recv() {
        Ok(Ok(())) => Ok(()),
        Ok(Err(e)) => Err(e),
        Err(_) => Err("always-on capture thread exited before signalling readiness".to_string()),
    }
}

/// Owns the cpal stream for the process lifetime. Each callback downmixes to
/// mono, resamples to 16 kHz, and forwards samples to the async processor.
fn capture_on_thread(
    tx: tokio::sync::mpsc::UnboundedSender<Vec<f32>>,
    setup_tx: &std::sync::mpsc::SyncSender<Result<(), String>>,
) -> Result<(), String> {
    use crate::openhuman::accessibility::{detect_microphone_permission, PermissionState};
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    use cpal::{SampleFormat, StreamConfig};

    // Surface the mic permission state explicitly — a denied/Unknown state is the
    // most common reason always-on "does nothing" and it differs per OS (macOS TCC
    // prompt, Windows privacy settings), so log it on every test build.
    let permission = detect_microphone_permission();
    log::info!("{LOG_PREFIX} microphone permission: {permission:?}");
    if matches!(permission, PermissionState::Denied) {
        log::error!("{LOG_PREFIX} microphone permission denied — always-on cannot capture audio");
        return Err("microphone permission denied".to_string());
    }

    let host = cpal::default_host();
    log::info!("{LOG_PREFIX} audio host: {:?}", host.id());
    let device = host
        .default_input_device()
        .ok_or_else(|| "no default audio input device".to_string())?;
    let device_name = device.name().unwrap_or_else(|e| format!("<unknown: {e}>"));
    let supported = device
        .default_input_config()
        .map_err(|e| format!("no default input config: {e}"))?;
    let source_rate = supported.sample_rate().0;
    let channels = supported.channels() as usize;
    let sample_format = supported.sample_format();
    let stream_config: StreamConfig = supported.into();
    // Name + source rate/channels/format vary across M-chip, Intel, and Windows
    // mics; capturing them makes a "wrong device" or "unsupported format" failure
    // obvious from the log alone. We resample everything to 16 kHz mono downstream.
    log::info!(
        "{LOG_PREFIX} capture device ready name='{device_name}' rate={source_rate}->{TARGET_SAMPLE_RATE} channels={channels} format={sample_format:?}"
    );

    // Forward one resampled-to-16k mono chunk per callback.
    let forward = move |mono_src: Vec<f32>| {
        let mono16k = resample(&mono_src, source_rate);
        // Ignore send errors — they mean the processor task is gone (shutdown).
        let _ = tx.send(mono16k);
    };

    let err_fn = |e| log::warn!("{LOG_PREFIX} cpal stream error: {e}");
    let stream = match sample_format {
        SampleFormat::F32 => device.build_input_stream(
            &stream_config,
            move |data: &[f32], _| forward(to_mono(data, channels)),
            err_fn,
            None,
        ),
        SampleFormat::I16 => device.build_input_stream(
            &stream_config,
            move |data: &[i16], _| {
                let floats: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0).collect();
                forward(to_mono(&floats, channels));
            },
            err_fn,
            None,
        ),
        SampleFormat::U16 => device.build_input_stream(
            &stream_config,
            move |data: &[u16], _| {
                let floats: Vec<f32> = data.iter().map(|&s| s as f32 / 32768.0 - 1.0).collect();
                forward(to_mono(&floats, channels));
            },
            err_fn,
            None,
        ),
        other => return Err(format!("unsupported sample format: {other:?}")),
    }
    .map_err(|e| format!("failed to build input stream: {e}"))?;

    stream
        .play()
        .map_err(|e| format!("failed to start stream: {e}"))?;
    let _ = setup_tx.send(Ok(()));
    log::info!("{LOG_PREFIX} microphone stream live");

    // Keep the stream (and thus this thread) alive for the process lifetime.
    loop {
        std::thread::sleep(std::time::Duration::from_secs(3600));
    }
}

/// Poll the screen-lock state and drive [`PAUSED`] so always-on never captures
/// what is spoken at the lock screen. macOS-only for now (uses the Quartz
/// session dictionary); other platforms never pause (no lock signal yet).
fn spawn_lock_watcher() {
    #[cfg(target_os = "macos")]
    tokio::spawn(async move {
        let mut last = false;
        loop {
            let locked = macos_lock::is_screen_locked();
            if locked != last {
                log::info!(
                    "{LOG_PREFIX} screen {} → {}",
                    if locked { "locked" } else { "unlocked" },
                    if locked { "pausing" } else { "resuming" }
                );
                PAUSED.store(locked, Ordering::Relaxed);
                last = locked;
            }
            tokio::time::sleep(std::time::Duration::from_secs(2)).await;
        }
    });
    #[cfg(not(target_os = "macos"))]
    {
        log::info!("{LOG_PREFIX} screen-lock watcher unavailable on this platform");
    }
}

/// macOS screen-lock detection via the Quartz session dictionary.
///
/// `CGSessionCopyCurrentDictionary` exposes `CGSSessionScreenIsLocked`; we read
/// it defensively (null dict ⇒ no session, treated as locked; missing/odd value
/// ⇒ unlocked) and never assume the CF value's concrete type without checking.
#[cfg(target_os = "macos")]
mod macos_lock {
    use std::ffi::{c_void, CString};

    type CFTypeRef = *const c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    extern "C" {
        fn CGSessionCopyCurrentDictionary() -> CFTypeRef;
    }
    #[link(name = "CoreFoundation", kind = "framework")]
    extern "C" {
        fn CFDictionaryGetValue(dict: CFTypeRef, key: CFTypeRef) -> CFTypeRef;
        fn CFStringCreateWithCString(alloc: CFTypeRef, c: *const i8, enc: u32) -> CFTypeRef;
        fn CFGetTypeID(v: CFTypeRef) -> usize;
        fn CFBooleanGetTypeID() -> usize;
        fn CFBooleanGetValue(b: CFTypeRef) -> u8;
        fn CFNumberGetTypeID() -> usize;
        fn CFNumberGetValue(n: CFTypeRef, the_type: i64, out: *mut c_void) -> u8;
        fn CFRelease(v: CFTypeRef);
    }
    const KCF_STRING_ENCODING_UTF8: u32 = 0x0800_0100;
    const KCF_NUMBER_SINT32: i64 = 3;

    /// True when the screen is locked (or there is no active GUI session).
    pub fn is_screen_locked() -> bool {
        // SAFETY: standard Quartz/CoreFoundation calls. Ownership: the session
        // dict and the key string are +1 (Create/Copy) and released here; the
        // dictionary value is borrowed and must not be released.
        unsafe {
            let dict = CGSessionCopyCurrentDictionary();
            if dict.is_null() {
                return true; // no session (loginwindow) — treat as locked
            }
            let Ok(key_c) = CString::new("CGSSessionScreenIsLocked") else {
                CFRelease(dict);
                return false;
            };
            let key = CFStringCreateWithCString(
                std::ptr::null(),
                key_c.as_ptr(),
                KCF_STRING_ENCODING_UTF8,
            );
            if key.is_null() {
                CFRelease(dict);
                return false;
            }
            let value = CFDictionaryGetValue(dict, key); // borrowed
            let locked = if value.is_null() {
                false
            } else {
                let tid = CFGetTypeID(value);
                if tid == CFBooleanGetTypeID() {
                    CFBooleanGetValue(value) != 0
                } else if tid == CFNumberGetTypeID() {
                    let mut n: i32 = 0;
                    CFNumberGetValue(value, KCF_NUMBER_SINT32, &mut n as *mut i32 as *mut c_void);
                    n != 0
                } else {
                    false
                }
            };
            CFRelease(key);
            CFRelease(dict);
            locked
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stop_clears_enabled_gate() {
        // stop() flips the runtime gate off so the processor drops all audio.
        ENABLED.store(true, Ordering::SeqCst);
        stop();
        assert!(
            !ENABLED.load(Ordering::SeqCst),
            "stop() must clear the ENABLED gate so capture goes idle on logout"
        );
    }

    fn cfg() -> VadConfig {
        VadConfig {
            onset_threshold: 0.01,
            hangover_ms: 100,
            min_speech_ms: 60,
            max_utterance_ms: 1000,
        }
    }

    /// Drive `n` frames of constant `rms` at `frame_ms` each, collecting events.
    fn drive(seg: &mut VadSegmenter, rms: f32, frame_ms: u32, n: u32) -> Vec<VadEvent> {
        (0..n)
            .filter_map(|_| seg.push_frame(rms, frame_ms))
            .collect()
    }

    #[test]
    fn silence_emits_nothing() {
        let mut seg = VadSegmenter::new(cfg());
        assert!(drive(&mut seg, 0.0, 20, 50).is_empty());
        assert!(!seg.is_speaking());
    }

    #[test]
    fn onset_then_hangover_emits_one_utterance() {
        let mut seg = VadSegmenter::new(cfg());
        // First loud frame opens the utterance.
        assert_eq!(seg.push_frame(0.2, 20), Some(VadEvent::SpeechStart));
        assert!(seg.is_speaking());
        // More speech, no event yet.
        assert!(drive(&mut seg, 0.2, 20, 5).is_empty());
        // Silence shorter than hangover: still open.
        assert!(seg.push_frame(0.0, 20).is_none()); // 20ms silence
        assert!(seg.push_frame(0.0, 20).is_none()); // 40ms
        assert!(seg.push_frame(0.0, 20).is_none()); // 60ms
        assert!(seg.push_frame(0.0, 20).is_none()); // 80ms
                                                    // Crossing the 100ms hangover closes it.
        let ev = seg.push_frame(0.0, 20).unwrap(); // 100ms
        match ev {
            VadEvent::SpeechEnd { emit, forced, .. } => {
                assert!(emit, "120ms voiced should clear the 60ms min");
                assert!(!forced);
            }
            other => panic!("expected SpeechEnd, got {other:?}"),
        }
        assert!(!seg.is_speaking());
    }

    #[test]
    fn short_blip_is_dropped() {
        let mut seg = VadSegmenter::new(cfg());
        // One 20ms loud frame (below the 60ms min), then silence to close.
        assert_eq!(seg.push_frame(0.2, 20), Some(VadEvent::SpeechStart));
        let mut ev = None;
        for _ in 0..5 {
            if let Some(e) = seg.push_frame(0.0, 20) {
                ev = Some(e);
                break;
            }
        }
        match ev.expect("utterance should close") {
            VadEvent::SpeechEnd {
                voiced_ms, emit, ..
            } => {
                assert_eq!(voiced_ms, 20);
                assert!(!emit, "20ms < 60ms min_speech ⇒ dropped");
            }
            other => panic!("expected SpeechEnd, got {other:?}"),
        }
    }

    #[test]
    fn mid_utterance_pause_does_not_split() {
        let mut seg = VadSegmenter::new(cfg());
        seg.push_frame(0.2, 20);
        // 80ms pause (< 100ms hangover) then speech resumes — one utterance.
        for _ in 0..4 {
            assert!(seg.push_frame(0.0, 20).is_none());
        }
        assert!(
            seg.is_speaking(),
            "pause under hangover keeps utterance open"
        );
        assert!(drive(&mut seg, 0.2, 20, 3).is_empty());
        assert!(seg.is_speaking());
    }

    #[test]
    fn max_utterance_forces_flush() {
        let mut seg = VadSegmenter::new(cfg()); // max 1000ms
        seg.push_frame(0.2, 20);
        // Keep talking past the ceiling; silence never triggers the close.
        let mut forced_seen = false;
        for _ in 0..60 {
            if let Some(VadEvent::SpeechEnd { forced, emit, .. }) = seg.push_frame(0.2, 20) {
                assert!(forced, "loud-throughout close must be the ceiling");
                assert!(emit);
                forced_seen = true;
                break;
            }
        }
        assert!(forced_seen, "should force-flush at max_utterance_ms");
        assert!(!seg.is_speaking());
    }

    #[test]
    fn reset_aborts_without_event() {
        let mut seg = VadSegmenter::new(cfg());
        seg.push_frame(0.2, 20);
        assert!(seg.is_speaking());
        seg.reset();
        assert!(!seg.is_speaking());
        // After reset, a fresh onset starts a new utterance.
        assert_eq!(seg.push_frame(0.2, 20), Some(VadEvent::SpeechStart));
    }

    #[test]
    fn from_server_config_maps_seconds_to_ms() {
        let mut c = CfgVoiceServer::default();
        c.vad_max_utterance_secs = 2.5;
        c.vad_hangover_ms = 750;
        let v = VadConfig::from_server_config(&c);
        assert_eq!(v.max_utterance_ms, 2500);
        assert_eq!(v.hangover_ms, 750);
        assert_eq!(v.onset_threshold, c.vad_onset_threshold);
    }

    #[test]
    fn wake_word_extracts_command() {
        // Case/punctuation tolerant; strips the phrase, keeps the command.
        assert_eq!(
            extract_command("Hey Tiny, play Numb by Linkin Park", "Hey Tiny").as_deref(),
            Some("play numb by linkin park")
        );
        assert_eq!(
            extract_command("hey tiny open slack", "Hey Tiny").as_deref(),
            Some("open slack")
        );
        // Leading filler before the wake phrase is tolerated.
        assert_eq!(
            extract_command("um, hey tiny what time is it", "Hey Tiny").as_deref(),
            Some("what time is it")
        );
    }

    #[test]
    fn wake_word_tolerates_stt_homophones() {
        // STT often mangles "Hey Tiny" — accept close variants of the anchor.
        assert_eq!(
            extract_command("Hey Tony, play music", "Hey Tiny").as_deref(),
            Some("play music")
        );
        assert_eq!(
            extract_command("a tinny open slack", "Hey Tiny").as_deref(),
            Some("open slack")
        );
        // Anchor too far in / absent → not a command.
        assert_eq!(
            extract_command("the tiny details matter here a lot", "Hey Tiny").as_deref(),
            // "tiny" at index 1 → command is the rest; documents the known
            // trade-off that an early "tiny" can trigger.
            Some("details matter here a lot")
        );
    }

    #[test]
    fn wake_word_absent_is_ignored() {
        assert_eq!(extract_command("play some music", "Hey Tiny"), None);
        // Wake word alone with no command → nothing to do.
        assert_eq!(extract_command("Hey Tiny", "Hey Tiny"), None);
        assert_eq!(extract_command("hey tiny!", "Hey Tiny"), None);
    }

    #[test]
    fn empty_wake_word_passes_everything() {
        assert_eq!(
            extract_command("just say this", "").as_deref(),
            Some("just say this")
        );
        assert_eq!(extract_command("   ", ""), None);
    }
}
