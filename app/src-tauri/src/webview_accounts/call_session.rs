//! Per-account call session manager.
//!
//! When a voice call is detected on an embedded webview account (Slack Huddle,
//! Discord VC, or WhatsApp call), the caller invokes `CallSessionManager::start`
//! to open a `CallSession`. The session subscribes to the CEF audio tap registry
//! for that browser, collects raw PCM samples during the call, and when
//! `CallSession::end` is called it:
//!
//!   1. Assembles the buffered samples into a 16-kHz mono WAV file (resampled
//!      from whatever CEF delivers, typically 48 kHz stereo).
//!   2. POSTs the WAV bytes to `openhuman.voice_transcribe_bytes` on the core
//!      JSON-RPC sidecar.
//!   3. Emits a `webview:call_transcript` Tauri event carrying the transcript
//!      text so the React service layer can persist it to memory.
//!
//! Only built when the `cef` feature flag is active — non-CEF builds include
//! the type definitions but all methods are no-ops.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Emitter, Runtime};

/// Target sample rate for Whisper STT (16 kHz mono).
const WHISPER_SAMPLE_RATE: u32 = 16_000;

/// Maximum call recording duration. Audio beyond this limit is silently
/// discarded to bound memory usage — a 2-hour call at 16kHz f32 mono uses
/// ~460 MB without this cap.
const MAX_CALL_DURATION_SECS: u64 = 7200; // 2 hours

/// Minimum call duration before we attempt transcription. Very short calls
/// (e.g. missed calls) are not worth sending to Whisper.
const MIN_CALL_DURATION_SECS: u64 = 5;

/// Silence RMS threshold — chunks below this level are filtered out before
/// sending audio to Whisper.
const SILENCE_RMS_THRESHOLD: f32 = 0.002;

/// One active call recording session for a single account.
pub struct CallSession {
    /// Account identifier this session belongs to.
    pub account_id: String,
    /// Provider name: "slack", "discord", "whatsapp".
    pub provider: String,
    /// Channel / contact name at call start (best-effort).
    pub channel_name: Option<String>,
    /// Wall-clock time when the call started.
    pub started_at: Instant,
    /// CEF browser id so we can snapshot the ring buffer at call end.
    pub browser_id: i32,
    /// Sample rate reported by CEF for this browser's audio stream.
    pub source_sample_rate: u32,
}

impl CallSession {
    fn new(
        account_id: String,
        provider: String,
        channel_name: Option<String>,
        browser_id: i32,
        source_sample_rate: u32,
    ) -> Self {
        log::info!(
            "[call_session] new session account={} provider={} channel={:?} browser_id={} sr={}Hz",
            account_id,
            provider,
            channel_name,
            browser_id,
            source_sample_rate,
        );
        Self {
            account_id,
            provider,
            channel_name,
            started_at: Instant::now(),
            browser_id,
            source_sample_rate,
        }
    }

    /// Duration since the call started.
    fn duration(&self) -> Duration {
        self.started_at.elapsed()
    }
}

/// Global map from `account_id` → active `CallSession`.
pub struct CallSessionManager {
    sessions: Mutex<HashMap<String, CallSession>>,
}

impl CallSessionManager {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            sessions: Mutex::new(HashMap::new()),
        })
    }

    /// Begin recording audio for an account. If a session already exists it
    /// is replaced (handles pathological double-start).
    pub fn start(
        &self,
        account_id: &str,
        provider: &str,
        channel_name: Option<String>,
        browser_id: i32,
    ) {
        #[cfg(feature = "cef")]
        {
            use tauri_runtime_cef::audio_tap_registry as audio_tap;
            let sr = audio_tap::get_sample_rate(browser_id).unwrap_or(48_000) as u32;
            let session = CallSession::new(
                account_id.to_string(),
                provider.to_string(),
                channel_name,
                browser_id,
                sr,
            );
            let mut g = self.sessions.lock().unwrap();
            g.insert(account_id.to_string(), session);
            log::debug!(
                "[call_session] started account={} provider={} browser_id={} sr={}Hz",
                account_id,
                provider,
                browser_id,
                sr,
            );
        }
        #[cfg(not(feature = "cef"))]
        {
            let _ = (account_id, provider, channel_name, browser_id);
            log::debug!("[call_session] start: cef feature not enabled, no-op");
        }
    }

    /// Returns `true` if there is an active call session for `account_id`.
    pub fn is_active(&self, account_id: &str) -> bool {
        self.sessions.lock().unwrap().contains_key(account_id)
    }

    /// End the call session, transcribe accumulated audio, and emit the result.
    ///
    /// This is async because it POSTs to the core RPC sidecar. The call is
    /// fire-and-forget from the scanner — callers should `tokio::spawn` it.
    pub async fn end<R: Runtime>(&self, app: &AppHandle<R>, account_id: &str, reason: &str) {
        let session = {
            let mut g = self.sessions.lock().unwrap();
            g.remove(account_id)
        };
        let session = match session {
            Some(s) => s,
            None => {
                log::debug!(
                    "[call_session] end: no active session for account={}",
                    account_id
                );
                return;
            }
        };

        let dur = session.duration();
        log::info!(
            "[call_session] ended account={} provider={} channel={:?} duration={:.1}s reason={} browser_id={}",
            account_id,
            session.provider,
            session.channel_name,
            dur.as_secs_f32(),
            reason,
            session.browser_id,
        );

        if dur.as_secs() < MIN_CALL_DURATION_SECS {
            log::debug!(
                "[call_session] call too short ({:.1}s < {}s), skipping transcription",
                dur.as_secs_f32(),
                MIN_CALL_DURATION_SECS
            );
            return;
        }

        // Snapshot the ring buffer from the audio tap for this browser.
        // The ring buffer holds up to 30 seconds of mono f32 at the source
        // sample rate. We cap at the call duration to avoid using unrelated
        // pre-call audio.
        #[allow(unused_mut)]
        let mut samples: Vec<f32> = Vec::new();
        #[cfg(feature = "cef")]
        {
            use tauri_runtime_cef::audio_tap_registry as audio_tap;
            if session.browser_id >= 0 {
                let ring = audio_tap::snapshot_ring_buffer(session.browser_id);
                // Cap to call duration at source rate.
                let max_samples = (session.source_sample_rate as u64
                    * dur.as_secs().min(MAX_CALL_DURATION_SECS))
                    as usize;
                let start = ring.len().saturating_sub(max_samples);
                samples = ring[start..].to_vec();
            }
        }

        if samples.is_empty() {
            log::debug!(
                "[call_session] no audio samples in ring buffer for account={}, skipping transcription",
                account_id
            );
            return;
        }

        // Filter out silent samples before encoding.
        let samples: Vec<f32> = samples
            .chunks(480) // ~10ms at 48kHz
            .filter(|chunk| chunk_rms(chunk) >= SILENCE_RMS_THRESHOLD)
            .flat_map(|chunk| chunk.iter().copied())
            .collect();

        if samples.is_empty() {
            log::debug!(
                "[call_session] audio is all silence for account={}, skipping transcription",
                account_id
            );
            return;
        }

        // Resample from source rate → 16 kHz.
        let wav_bytes = match build_wav_16k(&samples, session.source_sample_rate) {
            Ok(b) => b,
            Err(e) => {
                log::error!(
                    "[call_session] WAV encode failed for account={}: {}",
                    account_id,
                    e
                );
                return;
            }
        };

        log::info!(
            "[call_session] sending {} bytes of WAV audio to core RPC for account={}",
            wav_bytes.len(),
            account_id
        );

        match transcribe_via_core_rpc(&wav_bytes).await {
            Ok(text) => {
                if text.trim().is_empty() {
                    log::debug!(
                        "[call_session] transcription returned empty text for account={}",
                        account_id
                    );
                    return;
                }
                log::info!(
                    "[call_session] transcript ready account={} len={}",
                    account_id,
                    text.len()
                );
                let now_ms = chrono_now_millis();
                let transcript_evt = json!({
                    "account_id": account_id,
                    "provider": session.provider,
                    "kind": "call_transcript",
                    "payload": {
                        "provider": session.provider,
                        "channelName": session.channel_name,
                        "transcript": text,
                        "durationSecs": dur.as_secs(),
                        "reason": reason,
                        "startedAt": now_ms - dur.as_millis() as i64,
                        "endedAt": now_ms,
                    },
                    "ts": now_ms,
                });
                if let Err(e) = app.emit("webview:event", &transcript_evt) {
                    log::warn!(
                        "[call_session] call_transcript emit failed for account={}: {}",
                        account_id,
                        e
                    );
                }
            }
            Err(e) => {
                log::error!(
                    "[call_session] transcription failed for account={}: {}",
                    account_id,
                    e
                );
            }
        }
    }
}

impl Default for CallSessionManager {
    fn default() -> Self {
        Self {
            sessions: Mutex::new(HashMap::new()),
        }
    }
}

// ─── Audio helpers ──────────────────────────────────────────────────────────

/// Compute RMS of a mono sample slice.
fn chunk_rms(samples: &[f32]) -> f32 {
    if samples.is_empty() {
        return 0.0;
    }
    let sum_sq: f32 = samples.iter().map(|&s| s * s).sum();
    (sum_sq / samples.len() as f32).sqrt()
}

/// Resample `samples` (recorded at `src_rate` Hz) to 16 kHz using linear
/// interpolation and encode as a 16-bit mono WAV. Returns the WAV bytes.
fn build_wav_16k(samples: &[f32], src_rate: u32) -> Result<Vec<u8>, String> {
    // Resample to 16 kHz using linear interpolation.
    let resampled = if src_rate == WHISPER_SAMPLE_RATE {
        samples.to_vec()
    } else {
        linear_resample(samples, src_rate, WHISPER_SAMPLE_RATE)
    };

    // Encode as 16-bit signed PCM WAV.
    let mut buf: Vec<u8> = Vec::with_capacity(44 + resampled.len() * 2);
    write_wav_header(&mut buf, WHISPER_SAMPLE_RATE, 1, resampled.len() as u32);
    for s in &resampled {
        let pcm16 = (s.clamp(-1.0, 1.0) * 32767.0) as i16;
        buf.extend_from_slice(&pcm16.to_le_bytes());
    }
    Ok(buf)
}

/// Linear interpolation resampler. Not pitch-perfect but adequate for speech.
fn linear_resample(input: &[f32], src_rate: u32, dst_rate: u32) -> Vec<f32> {
    if input.is_empty() {
        return Vec::new();
    }
    let ratio = src_rate as f64 / dst_rate as f64;
    let out_len = ((input.len() as f64) / ratio).ceil() as usize;
    let mut out = Vec::with_capacity(out_len);
    for i in 0..out_len {
        let src_pos = i as f64 * ratio;
        let src_idx = src_pos as usize;
        let frac = (src_pos - src_idx as f64) as f32;
        let a = *input.get(src_idx).unwrap_or(&0.0);
        let b = *input.get(src_idx + 1).unwrap_or(&a);
        out.push(a + frac * (b - a));
    }
    out
}

/// Write a minimal 44-byte PCM WAV header into `buf`.
fn write_wav_header(buf: &mut Vec<u8>, sample_rate: u32, channels: u16, num_samples: u32) {
    let bits_per_sample: u16 = 16;
    let byte_rate = sample_rate * channels as u32 * bits_per_sample as u32 / 8;
    let block_align = channels * bits_per_sample / 8;
    let data_size = num_samples * channels as u32 * bits_per_sample as u32 / 8;
    let chunk_size = 36 + data_size;

    buf.extend_from_slice(b"RIFF");
    buf.extend_from_slice(&chunk_size.to_le_bytes());
    buf.extend_from_slice(b"WAVE");
    buf.extend_from_slice(b"fmt ");
    buf.extend_from_slice(&16u32.to_le_bytes()); // subchunk1 size
    buf.extend_from_slice(&1u16.to_le_bytes()); // PCM format
    buf.extend_from_slice(&channels.to_le_bytes());
    buf.extend_from_slice(&sample_rate.to_le_bytes());
    buf.extend_from_slice(&byte_rate.to_le_bytes());
    buf.extend_from_slice(&block_align.to_le_bytes());
    buf.extend_from_slice(&bits_per_sample.to_le_bytes());
    buf.extend_from_slice(b"data");
    buf.extend_from_slice(&data_size.to_le_bytes());
}

// ─── Core RPC transcription ─────────────────────────────────────────────────

/// Resolve the core JSON-RPC URL from the environment or use the default.
fn core_rpc_url() -> String {
    std::env::var("OPENHUMAN_CORE_RPC_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:7788/rpc".to_string())
}

/// POST `audio_bytes` (WAV) to `openhuman.voice_transcribe_bytes` on the core
/// sidecar and return the transcript text.
async fn transcribe_via_core_rpc(audio_bytes: &[u8]) -> Result<String, String> {
    // The core RPC `voice_transcribe_bytes` method accepts base64-encoded audio
    // bytes with an `extension` field indicating the container format.
    let encoded = base64_encode(audio_bytes);

    let body = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "openhuman.voice_transcribe_bytes",
        "params": {
            "audio_bytes": encoded,
            "extension": "wav",
            "context": "voice call transcript",
        },
    });

    let url = core_rpc_url();
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(120))
        .build()
        .map_err(|e| format!("build http client: {e}"))?;

    let resp = client
        .post(&url)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("POST {url}: {e}"))?;

    let status = resp.status();
    if !status.is_success() {
        let body_text = resp.text().await.unwrap_or_default();
        return Err(format!("core RPC error {status}: {body_text}"));
    }

    let v: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("decode core RPC response: {e}"))?;

    if let Some(err) = v.get("error") {
        return Err(format!("core RPC returned error: {err}"));
    }

    // The voice transcribe result is in `result.value.text`.
    let text = v
        .pointer("/result/value/text")
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string();

    log::debug!(
        "[call_session] core RPC transcription: {} chars",
        text.len()
    );
    Ok(text)
}

/// Minimal base64 encoding (no external dep needed — the Tauri shell already
/// links `base64` indirectly but we just need encode here).
fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() * 4 + 2) / 3);
    for chunk in data.chunks(3) {
        let b0 = chunk[0];
        let b1 = *chunk.get(1).unwrap_or(&0);
        let b2 = *chunk.get(2).unwrap_or(&0);
        out.push(CHARS[((b0 >> 2) & 0x3F) as usize] as char);
        out.push(CHARS[(((b0 & 0x3) << 4) | ((b1 >> 4) & 0xF)) as usize] as char);
        out.push(if chunk.len() > 1 {
            CHARS[(((b1 & 0xF) << 2) | ((b2 >> 6) & 0x3)) as usize] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            CHARS[(b2 & 0x3F) as usize] as char
        } else {
            '='
        });
    }
    out
}

fn chrono_now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
