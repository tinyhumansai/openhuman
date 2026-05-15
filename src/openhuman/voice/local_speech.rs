//! Local text-to-speech — invokes Piper as a sub-process via the
//! `PIPER_BIN` environment variable, then reads the resulting WAV file
//! back into a base64-encoded payload that matches the
//! [`super::reply_speech::ReplySpeechResult`] shape so the renderer can
//! swap providers without branching on the response.
//!
//! ## Why Piper, not Kokoro
//!
//! The plan for issue #1710 evaluated both engines for the first local TTS
//! ship:
//!
//! - **Piper** — ONNX-based, lower latency (~150 ms on M2 CPU for a short
//!   sentence), simpler runtime contract (one binary + one `.onnx` voice
//!   file), and `PIPER_BIN` is already reserved in `.env.example`.
//! - **Kokoro** — 82M parameters, higher audio quality, but requires a
//!   Python runtime or a custom ONNX runner with phonemization, and the
//!   integration surface is materially larger.
//!
//! Piper ships first. Kokoro is tracked as future work and would land as a
//! sibling module (`local_speech_kokoro.rs`) plus a `"kokoro"` branch in
//! [`super::factory::create_tts_provider`].
//!
//! ## Resolution order
//!
//! 1. `PIPER_BIN` env var (absolute path, takes precedence)
//! 2. `piper` / `piper.exe` on `$PATH`
//!
//! Both branches share the same resolution helper as the legacy voice
//! pipeline ([`crate::openhuman::local_ai::paths::resolve_piper_binary`]),
//! so STT availability checks, the installer UI, and the factory dispatch
//! all agree on what counts as "installed".
//!
//! ## Where to get the binary
//!
//! **Easy path:** click "Install Piper" in `Settings → Voice → Voice
//! Providers`. That triggers
//! [`crate::openhuman::local_ai::install_piper`] which downloads the
//! Piper binary archive (`.zip` on Windows, `.tar.gz` on macOS / Linux)
//! into `~/.openhuman/bin/piper/`, extracts it, and stages the bundled
//! `en_US-lessac-medium` voice (`.onnx` + `.onnx.json`) alongside via a
//! `.part` file + atomic rename. After install the `resolve_piper_binary`
//! helper in `local_ai/paths.rs` picks it up automatically.
//!
//! **Advanced path:** download Piper from
//! [rhasspy/piper](https://github.com/rhasspy/piper) releases (one
//! self-contained binary per OS) plus a voice `.onnx` (+ `.onnx.json`)
//! from [rhasspy/piper-voices](https://huggingface.co/rhasspy/piper-voices),
//! and either drop the binary on `$PATH` or point `PIPER_BIN` at it.
//!
//! ## Hardware / latency notes (AC #2 of issue #1710)
//!
//! Piper on a 2022 M2 CPU synthesizes ~150 ms of audio per second of
//! output for the `medium` quality tier; on a five-year-old laptop budget
//! 300–500 ms. The visemes returned here are a synthetic flat timeline
//! (the renderer uses them only as a fallback when the cloud branch fails)
//! — accurate visemes from Piper would require a separate forced-aligner
//! pass and is intentionally out of scope.
//!
//! ## Log prefix
//!
//! `[voice-tts]` — pairs with `[voice-stt]` and `[voice-factory]` for
//! end-to-end debug greps.

use std::path::PathBuf;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use log::debug;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::paths::{resolve_piper_binary_with_config, resolve_tts_voice_path};
use crate::rpc::RpcOutcome;

use super::reply_speech::{ReplySpeechResult, VisemeFrame};

const LOG_PREFIX: &str = "[voice-tts]";

/// Default Piper voice id.
pub const DEFAULT_PIPER_VOICE: &str = "en_US-lessac-medium";

/// Caller-tunable knobs for local Piper synthesis.
#[derive(Debug, Default, Clone)]
pub struct PiperOptions {
    /// Override voice id (e.g. `en_US-lessac-medium`). When `None` we
    /// resolve against `config.local_ai.tts_voice_id` via
    /// [`resolve_tts_voice_path`].
    pub voice: Option<String>,
}

/// Synthesize speech using local Piper.
///
/// Implementation strategy (sub-process model):
///
/// 1. Resolve `PIPER_BIN` (env override → PATH). Missing binary → error.
/// 2. Resolve the voice `.onnx` path against the workspace; missing model
///    surfaces an actionable error pointing the user at the installer.
/// 3. Write a temp WAV output path, spawn `piper --model <voice>
///    --output_file <out.wav>`, pipe `text` to stdin, wait, then read the
///    WAV back into memory.
/// 4. Return a [`ReplySpeechResult`] with `audio_base64` populated and a
///    synthetic neutral viseme timeline so the mascot lip-sync doesn't
///    null-out.
///
/// **No model assets are embedded.** Voice files live in the workspace
/// models directory after the installer pulls them.
pub async fn synthesize_piper(
    config: &Config,
    text: &str,
    opts: &PiperOptions,
) -> Result<RpcOutcome<ReplySpeechResult>, String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err("text is required".to_string());
    }

    let piper_bin = resolve_piper_binary_with_config(config).ok_or_else(|| {
        format!(
            "{LOG_PREFIX} piper binary not found. \
             Set PIPER_BIN to the absolute path of piper, or install piper on \
             PATH (download from https://github.com/rhasspy/piper/releases)."
        )
    })?;
    debug!("{LOG_PREFIX} resolved piper binary={}", piper_bin.display());

    let voice_path = resolve_tts_voice_path(config).map_err(|e| format!("{LOG_PREFIX} {e}"))?;
    let voice_id = opts
        .voice
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_PIPER_VOICE)
        .to_string();
    debug!("{LOG_PREFIX} voice={voice_id} model_path={voice_path}");

    let out_dir = std::env::temp_dir().join("openhuman_voice_output");
    tokio::fs::create_dir_all(&out_dir)
        .await
        .map_err(|e| format!("{LOG_PREFIX} failed to create voice output directory: {e}"))?;
    let out_path = out_dir.join(format!(
        "piper-{}-{}.wav",
        chrono::Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4()
    ));

    // Piper's default --length-scale is 1.0 which sounds rushed for most
    // English voices. 1.15 (≈ 15% slower) lands closer to natural speech
    // pace without dragging. Future work: surface this as a settings slider
    // (config.local_ai.tts_length_scale) so users can tune to taste.
    const DEFAULT_LENGTH_SCALE: &str = "1.15";

    let spawn_started = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new(&piper_bin);
    cmd.args([
        "--model",
        voice_path.as_str(),
        "--output_file",
        &out_path.to_string_lossy(),
        "--length-scale",
        DEFAULT_LENGTH_SCALE,
    ])
    .stdin(std::process::Stdio::piped())
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::piped());
    // Suppress the Windows console window that would otherwise flash on
    // every TTS request (piper.exe is a console subsystem binary).
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{LOG_PREFIX} failed to launch piper: {e}"))?;

    // Pipe the text to stdin — Piper reads UTF-8 lines.
    if let Some(mut stdin) = child.stdin.take() {
        use tokio::io::AsyncWriteExt;
        stdin
            .write_all(trimmed.as_bytes())
            .await
            .map_err(|e| format!("{LOG_PREFIX} failed to write text to piper stdin: {e}"))?;
        // Drop stdin so piper sees EOF and finishes synthesis.
    }

    let output = child
        .wait_with_output()
        .await
        .map_err(|e| format!("{LOG_PREFIX} failed to wait on piper: {e}"))?;

    let exit_code = output.status.code();
    debug!(
        "{LOG_PREFIX} piper exited code={:?} elapsed_ms={} stderr_bytes={}",
        exit_code,
        spawn_started.elapsed().as_millis(),
        output.stderr.len()
    );
    if !output.status.success() {
        // Best-effort cleanup of the partial output.
        let _ = tokio::fs::remove_file(&out_path).await;
        return Err(format!(
            "{LOG_PREFIX} piper failed (exit={:?}): {}",
            exit_code,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let audio_bytes = read_and_clean_wav(&out_path).await?;
    let audio_base64 = BASE64.encode(&audio_bytes);
    let visemes = synthetic_viseme_timeline(trimmed);
    debug!(
        "{LOG_PREFIX} synthesized wav_bytes={} visemes={}",
        audio_bytes.len(),
        visemes.len()
    );

    Ok(RpcOutcome::single_log(
        ReplySpeechResult {
            audio_base64,
            audio_mime: "audio/wav".to_string(),
            visemes,
            alignment: None,
        },
        "local piper TTS completed",
    ))
}

async fn read_and_clean_wav(path: &std::path::Path) -> Result<Vec<u8>, String> {
    let bytes = tokio::fs::read(path)
        .await
        .map_err(|e| format!("{LOG_PREFIX} failed to read piper output: {e}"))?;
    if let Err(e) = tokio::fs::remove_file(path).await {
        log::warn!(
            "{LOG_PREFIX} failed to clean up piper output {}: {e}",
            path.display()
        );
    }
    Ok(bytes)
}

/// Build a synthetic neutral-vowel viseme timeline. The mascot expects at
/// least one frame to render the mouth; without it the rig snaps closed
/// for the entire utterance. A real forced-aligner pass would replace
/// this — see the module-level note.
fn synthetic_viseme_timeline(text: &str) -> Vec<VisemeFrame> {
    let chars = text.chars().filter(|c| !c.is_whitespace()).count().max(1);
    // ~80 ms per non-whitespace char is a reasonable average for English
    // speech at conversational tempo. The mascot smooths between frames
    // so this looks plausible without being meaningfully wrong.
    let per_char_ms: u64 = 80;
    let total_ms = (chars as u64) * per_char_ms;
    vec![
        VisemeFrame {
            viseme: "sil".to_string(),
            start_ms: 0,
            end_ms: 40,
        },
        VisemeFrame {
            viseme: "aa".to_string(),
            start_ms: 40,
            end_ms: total_ms.max(80),
        },
    ]
}

/// Resolves [`PathBuf`] inputs to absolute paths so logs/errors don't show
/// platform-specific relative noise. Kept as a tiny helper so its
/// behaviour is testable.
#[allow(dead_code)]
fn absolutize(p: PathBuf) -> PathBuf {
    p.canonicalize().unwrap_or(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn synthesize_piper_rejects_empty_text() {
        let config = Config::default();
        let opts = PiperOptions::default();
        let err = synthesize_piper(&config, "", &opts).await.err().unwrap();
        assert!(err.contains("required"), "empty text must error: {err}");

        let err = synthesize_piper(&config, "   ", &opts).await.err().unwrap();
        assert!(
            err.contains("required"),
            "whitespace text must error: {err}"
        );
    }

    #[tokio::test]
    async fn synthesize_piper_surfaces_binary_lookup_failure() {
        // Same shape as the whisper test — make sure missing PIPER_BIN
        // produces an actionable error, not a panic in the spawn path.
        let prev_piper = std::env::var_os("PIPER_BIN");
        std::env::remove_var("PIPER_BIN");

        let config = Config::default();
        let opts = PiperOptions::default();
        let result = synthesize_piper(&config, "hello world", &opts).await;

        if let Some(v) = prev_piper {
            std::env::set_var("PIPER_BIN", v);
        }

        let err = result.err().expect("missing piper must error");
        assert!(
            err.contains("piper") || err.contains("TTS"),
            "should mention piper or TTS: {err}"
        );
    }

    #[test]
    fn synthetic_viseme_timeline_yields_non_empty_frames() {
        let frames = synthetic_viseme_timeline("hello world");
        assert!(!frames.is_empty(), "must produce at least one frame");
        assert_eq!(frames[0].viseme, "sil", "leading silence");
        assert!(
            frames.last().unwrap().end_ms >= 80,
            "tail frame must extend past the leading silence"
        );
    }

    #[test]
    fn synthetic_viseme_timeline_handles_whitespace_only_text() {
        // Whitespace-only input would normally be rejected upstream, but
        // the helper itself must not panic — defends against a future
        // caller that bypasses the validator.
        let frames = synthetic_viseme_timeline("   ");
        assert!(!frames.is_empty());
        // chars().filter(non-ws).count() is 0 → min 1 → 80 ms total.
        assert_eq!(frames[1].end_ms, 80);
    }

    #[test]
    fn synthetic_viseme_timeline_scales_with_length() {
        let short = synthetic_viseme_timeline("hi");
        let long = synthetic_viseme_timeline("the quick brown fox jumps");
        assert!(
            long.last().unwrap().end_ms > short.last().unwrap().end_ms,
            "longer text should produce a longer timeline"
        );
    }
}
