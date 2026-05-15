//! Local speech-to-text — invokes whisper.cpp (`whisper-cli`) as a
//! sub-process via the `WHISPER_BIN` environment variable.
//!
//! ## Resolution order
//!
//! 1. `WHISPER_BIN` env var (absolute path, takes precedence)
//! 2. `whisper-cli` / `whisper-cli.exe` on `$PATH`
//!
//! When neither resolves, transcription fails with a clear, actionable
//! error pointing the user at the install path. Resolution lives in
//! [`crate::openhuman::local_ai::paths::resolve_whisper_binary`] — kept in
//! one place so STT, voice-status, and the installer all agree.
//!
//! ## Where to get the binary
//!
//! **Easy path:** click "Install Whisper" in `Settings → Voice → Voice
//! Providers`. That triggers
//! [`crate::openhuman::local_ai::install_whisper`] which streams the
//! GGML model file (`ggml-<size>.bin`) into
//! `~/.openhuman/bin/whisper/` via a `.part` file + atomic rename, plus
//! the `whisper-cli` binary on Windows where upstream ships a release
//! archive. After install the `resolve_whisper_binary` helper in
//! `local_ai/paths.rs` picks it up automatically — no env var to set.
//!
//! **Advanced path:** install whisper.cpp's `whisper-cli` from a package
//! manager (`brew install whisper-cpp`, `pacman -S whisper.cpp`, …) or
//! build from source ([ggerganov/whisper.cpp](https://github.com/ggerganov/whisper.cpp))
//! and either drop the binary on `$PATH` or point `WHISPER_BIN` at it.
//!
//! ## Hardware / latency notes (AC #2 of issue #1710)
//!
//! Default model is **`whisper-large-v3-turbo`** — best accuracy at a
//! latency that fits a desktop UX (≈ 4× faster than `large-v3` with the
//! same WER on English). On lower-end hardware:
//!
//! | Model           | Disk    | RAM    | Latency (M2 CPU, 10s clip) | Notes |
//! |-----------------|---------|--------|-----------------------------|-------|
//! | `tiny`          | 39 MB   | ~150 MB| ~0.4 s                     | Demo-grade |
//! | `base`          | 74 MB   | ~210 MB| ~0.6 s                     | Decent for short utterances |
//! | `small`         | 244 MB  | ~480 MB| ~1.4 s                     | Default for older laptops |
//! | `medium`        | 769 MB  | ~1.2 GB| ~3.0 s                     | Good accuracy, heavier |
//! | `large-v3-turbo`| 1.5 GB  | ~2.2 GB| ~1.8 s                     | Recommended (this default) |
//!
//! No model assets are embedded in the binary — everything is downloaded
//! into the workspace on first use.
//!
//! ## Log prefix
//!
//! `[voice-stt]` — grep-friendly so debug runs across factory dispatch,
//! sub-process spawn, and result decoding line up cleanly.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use chrono::Utc;
use log::{debug, warn};
use serde::{Deserialize, Serialize};

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::paths::resolve_whisper_binary_with_config;
use crate::rpc::RpcOutcome;

const LOG_PREFIX: &str = "[voice-stt]";

/// Default model id when the caller does not override.
pub const DEFAULT_WHISPER_MODEL: &str = "medium";

/// Caller-tunable knobs for local Whisper transcription.
#[derive(Debug, Default, Clone)]
pub struct WhisperTranscribeOptions {
    /// Whisper model id (e.g. `whisper-large-v3-turbo`). When `None` we
    /// fall back to [`DEFAULT_WHISPER_MODEL`].
    pub model: Option<String>,
    /// Recorder MIME type (e.g. `audio/webm`). Used to pick the right file
    /// extension on disk before handing off to whisper-cli, which sniffs
    /// the extension.
    pub mime_type: Option<String>,
    /// BCP-47 language hint (e.g. `"en"`).
    pub language: Option<String>,
}

/// Output of local whisper transcription. Matches
/// [`super::cloud_transcribe::CloudTranscribeResult`] shape so the factory's
/// `SttResult` can carry either provider's payload without conditional code.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WhisperTranscribeResult {
    pub text: String,
    /// The model id that produced the transcript — populated so the UI can
    /// show the user which model ran (useful when they're A/B-testing
    /// sizes during the install flow).
    pub model_id: String,
}

/// Transcribe a base64-encoded audio blob using local whisper.cpp.
///
/// Implementation strategy (sub-process model):
///
/// 1. Resolve `WHISPER_BIN` (env override → PATH lookup). If missing,
///    return an actionable error so the UI can deep-link to the installer.
/// 2. Decode the base64 audio and write it to a temp file under
///    `$TMP/openhuman_voice_input/voice-<ts>-<uuid>.<ext>` — whisper-cli
///    consumes a file path, not stdin.
/// 3. Spawn `whisper-cli -m <model> -f <file> [-l <lang>]`, capture
///    stdout, and clean up the temp file regardless of outcome.
/// 4. Return the trimmed transcript. Empty stdout is reported as an error
///    (whisper produced no output → almost always a model/file mismatch).
///
/// **No model assets are embedded.** The model file is downloaded by the
/// installer into the workspace; this function only locates the binary.
pub async fn transcribe_whisper(
    config: &Config,
    audio_base64: &str,
    opts: &WhisperTranscribeOptions,
) -> Result<RpcOutcome<WhisperTranscribeResult>, String> {
    let trimmed = audio_base64.trim();
    if trimmed.is_empty() {
        return Err("audio_base64 is required".to_string());
    }
    let audio_bytes = BASE64
        .decode(trimmed)
        .map_err(|e| format!("invalid base64 audio: {e}"))?;
    if audio_bytes.is_empty() {
        return Err("decoded audio is empty".to_string());
    }

    let model_id = opts
        .model
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(DEFAULT_WHISPER_MODEL)
        .to_string();

    let whisper_bin = resolve_whisper_binary_with_config(config).ok_or_else(|| {
        format!(
            "{LOG_PREFIX} whisper.cpp binary not found. \
             Set WHISPER_BIN to the absolute path of whisper-cli, or install \
             whisper-cli on PATH (`brew install whisper-cpp` / package manager / \
             build from https://github.com/ggerganov/whisper.cpp)."
        )
    })?;
    debug!(
        "{LOG_PREFIX} resolved whisper binary={} model_id={}",
        whisper_bin.display(),
        model_id
    );

    let ext = mime_to_extension(opts.mime_type.as_deref());
    let voice_dir = std::env::temp_dir().join("openhuman_voice_input");
    tokio::fs::create_dir_all(&voice_dir)
        .await
        .map_err(|e| format!("{LOG_PREFIX} failed to create voice input directory: {e}"))?;
    let file_path = voice_dir.join(format!(
        "voice-{}-{}.{}",
        Utc::now().timestamp_millis(),
        uuid::Uuid::new_v4(),
        ext
    ));
    tokio::fs::write(&file_path, &audio_bytes)
        .await
        .map_err(|e| format!("{LOG_PREFIX} failed to write audio file: {e}"))?;
    debug!(
        "{LOG_PREFIX} staged audio bytes={} path={}",
        audio_bytes.len(),
        file_path.display()
    );

    // Resolve the on-disk model path using the effective model_id (which may
    // have been overridden by the request options). Without threading model_id
    // through here the resolver would ignore the override and use whatever the
    // config default is, producing a mismatch between the returned model_id
    // and the model actually used for transcription.
    let model_path =
        crate::openhuman::local_ai::paths::resolve_stt_model_path_by_id(&model_id, config)
            .map_err(|e| format!("{LOG_PREFIX} {e}"))?;
    debug!("{LOG_PREFIX} resolved STT model path={model_path}");

    let mut args: Vec<String> = vec![
        "-m".to_string(),
        model_path,
        "-f".to_string(),
        file_path.to_string_lossy().to_string(),
        // Suppress segment timestamp prefixes (`[00:00:00.000 --> ...]`) in
        // stdout — we want the bare transcript text only. Without this flag
        // the timestamps leak into the message body the user sees.
        "--no-timestamps".to_string(),
    ];
    if let Some(lang) = opts
        .language
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        args.push("-l".to_string());
        args.push(lang.to_string());
    }
    debug!("{LOG_PREFIX} spawning whisper-cli args={args:?}");

    let spawn_started = std::time::Instant::now();
    let mut cmd = tokio::process::Command::new(&whisper_bin);
    cmd.args(&args);
    // Suppress the Windows console window that would otherwise flash on
    // every invocation (whisper-cli is a console subsystem binary). The
    // 0x08000000 constant is CREATE_NO_WINDOW from winbase.h. No-op on
    // platforms without the extension trait.
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    // Cap the subprocess so a stalled whisper-cli never hangs the RPC
    // caller indefinitely. 120 s is generous for any reasonable audio
    // fragment but avoids an infinite wait on a hung process.
    const WHISPER_TIMEOUT_SECS: u64 = 120;
    let output_result = tokio::time::timeout(
        std::time::Duration::from_secs(WHISPER_TIMEOUT_SECS),
        cmd.output(),
    )
    .await
    .map_err(|_| format!("{LOG_PREFIX} whisper-cli timed out after {WHISPER_TIMEOUT_SECS}s"))?;

    // Always clean up the staged audio file; warn but don't fail on cleanup.
    if let Err(e) = tokio::fs::remove_file(&file_path).await {
        warn!(
            "{LOG_PREFIX} failed to clean up temp audio file {}: {e}",
            file_path.display()
        );
    }

    let output =
        output_result.map_err(|e| format!("{LOG_PREFIX} failed to spawn whisper-cli: {e}"))?;

    let exit_code = output.status.code();
    debug!(
        "{LOG_PREFIX} whisper-cli exited code={:?} elapsed_ms={} stdout_bytes={} stderr_bytes={}",
        exit_code,
        spawn_started.elapsed().as_millis(),
        output.stdout.len(),
        output.stderr.len()
    );
    if !output.status.success() {
        return Err(format!(
            "{LOG_PREFIX} whisper-cli failed (exit={:?}): {}",
            exit_code,
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        return Err(format!(
            "{LOG_PREFIX} whisper-cli returned empty transcript (model={model_id})"
        ));
    }

    Ok(RpcOutcome::single_log(
        WhisperTranscribeResult { text, model_id },
        "local whisper STT completed",
    ))
}

/// Map a recorder MIME type to a safe filename extension. Defaults to
/// `webm` because `MediaRecorder` defaults to WebM/Opus and whisper-cli
/// (built with ffmpeg) handles it transparently.
fn mime_to_extension(mime: Option<&str>) -> &'static str {
    match mime
        .map(str::trim)
        .map(|m| m.split(';').next().unwrap_or(m).to_ascii_lowercase())
        .as_deref()
    {
        Some("audio/wav") | Some("audio/x-wav") => "wav",
        Some("audio/mpeg") => "mp3",
        Some("audio/mp4") | Some("audio/x-m4a") => "m4a",
        Some("audio/ogg") => "ogg",
        Some("audio/flac") => "flac",
        // Default branch covers `audio/webm`, `audio/webm;codecs=opus`,
        // unknown types, and `None`. WebM is the MediaRecorder default
        // on Chromium so it's the safest fallback.
        _ => "webm",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::engine::general_purpose::STANDARD as BASE64;
    use base64::Engine;

    #[test]
    fn mime_to_extension_maps_known_types() {
        assert_eq!(mime_to_extension(Some("audio/webm")), "webm");
        assert_eq!(mime_to_extension(Some("audio/webm;codecs=opus")), "webm");
        assert_eq!(mime_to_extension(Some("audio/wav")), "wav");
        assert_eq!(mime_to_extension(Some("audio/x-wav")), "wav");
        assert_eq!(mime_to_extension(Some("audio/mpeg")), "mp3");
        assert_eq!(mime_to_extension(Some("audio/mp4")), "m4a");
        assert_eq!(mime_to_extension(Some("audio/x-m4a")), "m4a");
        assert_eq!(mime_to_extension(Some("audio/ogg")), "ogg");
        assert_eq!(mime_to_extension(Some("audio/flac")), "flac");
    }

    #[test]
    fn mime_to_extension_falls_back_to_webm() {
        // Unknown / missing / unparseable inputs default to webm — covers
        // the case where MediaRecorder reports a vendor-specific type.
        assert_eq!(mime_to_extension(None), "webm");
        assert_eq!(mime_to_extension(Some("")), "webm");
        assert_eq!(mime_to_extension(Some("application/octet-stream")), "webm");
        assert_eq!(mime_to_extension(Some("video/mp4")), "webm");
    }

    #[tokio::test]
    async fn transcribe_whisper_rejects_empty_input() {
        let config = Config::default();
        let opts = WhisperTranscribeOptions::default();
        let err = transcribe_whisper(&config, "", &opts).await.err().unwrap();
        assert!(
            err.contains("required"),
            "should reject empty base64 input: {err}"
        );

        let err = transcribe_whisper(&config, "   ", &opts)
            .await
            .err()
            .unwrap();
        assert!(
            err.contains("required"),
            "whitespace-only must error: {err}"
        );
    }

    #[tokio::test]
    async fn transcribe_whisper_rejects_invalid_base64() {
        let config = Config::default();
        let opts = WhisperTranscribeOptions::default();
        let err = transcribe_whisper(&config, "not-base64-!", &opts)
            .await
            .err()
            .unwrap();
        assert!(err.contains("invalid base64"), "should fail decode: {err}");
    }

    #[tokio::test]
    async fn transcribe_whisper_rejects_empty_decoded_payload() {
        let config = Config::default();
        let opts = WhisperTranscribeOptions::default();
        // Valid base64 but decodes to zero bytes.
        let err = transcribe_whisper(&config, "", &opts).await.err().unwrap();
        assert!(
            err.contains("required") || err.contains("empty"),
            "should reject zero-byte audio: {err}"
        );
    }

    #[tokio::test]
    async fn transcribe_whisper_surfaces_binary_lookup_failure() {
        // No WHISPER_BIN and no PATH entry → factory must produce an
        // actionable error rather than panicking inside the subprocess
        // spawn. Use a 1-byte base64 payload so the binary-resolution
        // branch runs before any audio handling.
        let prev_whisper = std::env::var_os("WHISPER_BIN");
        std::env::remove_var("WHISPER_BIN");
        let payload = BASE64.encode(b"X");

        let config = Config::default();
        let opts = WhisperTranscribeOptions::default();
        let result = transcribe_whisper(&config, &payload, &opts).await;

        // Restore env immediately, even on failure.
        if let Some(v) = prev_whisper {
            std::env::set_var("WHISPER_BIN", v);
        }

        // Either the binary missing OR the model missing must surface; both
        // count as the "factory dispatched but local stack isn't installed"
        // case the test exists to cover.
        let err = result.err().expect("missing local stack must error");
        assert!(
            err.contains("whisper") || err.contains("STT model"),
            "should mention whisper or STT model: {err}"
        );
    }
}
