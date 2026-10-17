//! Transcription, wake-word gating, and local fast-path command execution for
//! a finished always-on utterance.

use super::{notch_status, LOG_PREFIX};
use crate::config::Config;
use crate::modules::voice as tinyvoice;
use crate::voice::audio_capture::TARGET_SAMPLE_RATE;

/// Transcribe a finished utterance and hand the text to the dictation bus,
/// which delivers it to the agent (auto-send) and the notch — the same path the
/// hotkey dictation uses.
pub(super) async fn transcribe_and_deliver(config: &Config, samples_16k: Vec<f32>) {
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
    let provider_name = crate::voice::effective_stt_provider(config);
    // Which STT backend is doing the work matters when diagnosing slow/failed
    // transcription across machines.
    log::info!(
        "{LOG_PREFIX} transcribing utterance: provider={provider_name} model=<provider default> samples={sample_count} wav_bytes={}",
        wav.len()
    );
    let provider = match crate::voice::create_stt_provider(&provider_name, "", config) {
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
            // Wake-word sequencing is owned by the tinyvoice module. A module
            // failure must not deliver an unaddressed utterance to the agent:
            // this gate fails CLOSED.
            use crate::modules::voice::UtteranceDisposition as UD;
            let disposition =
                match tinyvoice::sequence_utterance(config, &text, &config.voice_server.wake_word)
                    .await
                {
                    Ok(d) => d,
                    Err(error) => {
                        log::warn!(
                        "{LOG_PREFIX} wake-word gate unavailable ({error}); dropping the utterance"
                    );
                        return;
                    }
                };
            match disposition {
                UD::Command { command } => {
                    // Redacted: never log the raw spoken command (always-on mic PII).
                    log::info!("{LOG_PREFIX} wake word matched → cmd_len={}", command.len());
                    notch_status("Processing", 12000); // pill: running the command
                    deliver_command(config, command).await;
                }
                UD::BareWake => {
                    // Wake word spoken with no trailing command ("Hey Tiny").
                    // Acknowledge with an agent turn so the user gets a reply.
                    log::info!("{LOG_PREFIX} bare wake word → acknowledging");
                    notch_status("Listening…", 8000);
                    deliver_command(config, "hello".to_string()).await;
                }
                UD::Ignored => {
                    log::info!(
                        "{LOG_PREFIX} no wake word ({:?}) in transcript_len={}; ignored",
                        config.voice_server.wake_word,
                        text.len()
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
    use crate::modules::voice::{route, VoiceIntent};
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
        crate::voice::dictation_listener::publish_transcription(cmd);
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
            crate::voice::dictation_listener::publish_transcription(cmd);
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
    intent: crate::modules::voice::VoiceIntent,
) -> Result<String, String> {
    use crate::modules::voice::VoiceIntent as VI;
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
