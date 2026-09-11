//! WebSocket streaming transcription endpoint.
//!
//! Accepts a WebSocket connection that receives PCM16 audio chunks (16kHz mono),
//! accumulates them, and transcribes the completed utterance through the hosted
//! STT engine when the client stops.
//!
//! Protocol (unchanged — the client contract predates the engine swap):
//!   Client → Server: binary frames containing PCM16 LE audio bytes (16kHz mono)
//!   Server → Client: JSON text frames:
//!     { "type": "partial",  "text": "..." }          — interim transcription
//!     { "type": "final",    "text": "...", "raw_text": "..." } — after client sends
//!                                                        `{"type":"stop"}` text frame
//!     { "type": "error",    "message": "..." }        — on error
//!   Client → Server: text frame `{"type":"stop"}`     — end recording, get final result
//!
//! ## No partial results
//! `partial` frames are part of the protocol but are never emitted any more.
//! They came from the bundled in-process whisper.cpp engine, which could
//! re-decode a 15-second sliding window every 500 ms for free. That engine is
//! gone — STT is a hosted round trip now (`voice_server.stt_engine`) — and
//! re-uploading the window on every tick would multiply the request count and
//! the bill by ~30× for text the client discards a moment later. The frame type
//! stays in the protocol so a future streaming-capable engine can start sending
//! it without a client change; `dictation.streaming` only controls whether we
//! log that partials were requested.
//!
//! # Security notes
//!
//! ## Authentication
//! `GET /ws/dictation` is authenticated at the upgrade boundary (C4 / issue #1924).
//! The browser WebSocket API cannot set arbitrary request headers on upgrade, so the
//! check lives in `dictation_ws_handler` (`src/core/jsonrpc.rs`), not here: it requires
//! the per-process core bearer via `Authorization: Bearer <token>` (native callers) or
//! `?token=<token>` (browser clients), plus the same origin allowlist Socket.IO enforces,
//! and rejects the upgrade with 401/403 before this function runs. Do NOT add a
//! Bearer-header check in this function — by the time `handle_dictation_ws` is reached the
//! upgrade has already been authenticated, and a header check here would not work from
//! browsers anyway.
//!
//! ## Memory cap
//! The full-audio accumulation buffer (`full_audio_buf`) is bounded by
//! `MAX_FULL_AUDIO_SAMPLES` (~5 min at 16 kHz). Clients that stream beyond this limit
//! are disconnected with an error frame; see `append_stream_samples`.

use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde::Deserialize;
use tokio::sync::Mutex;

use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

use super::postprocess;
use crate::openhuman::config::Config;
use crate::openhuman::voice::{create_stt_provider, effective_stt_provider};

const LOG_PREFIX: &str = "[voice-stream]";
const AUDIO_SAMPLE_RATE: usize = 16_000;
/// Sliding window retained alongside the full-audio buffer. Nothing consumes it
/// today (see the "No partial results" note above); it is kept — and kept
/// bounded — so a streaming-capable engine can be wired to it without
/// re-deriving the windowing rules.
const MAX_STREAM_BUFFER_SAMPLES: usize = AUDIO_SAMPLE_RATE * 15; // 15s sliding window

/// Hard cap on the full-audio accumulation buffer.
///
/// Derived from `AUDIO_SAMPLE_RATE` (16 kHz mono PCM16) × 60 s × 5 min = 4 800 000 samples
/// ≈ 9.6 MiB per connection. Clients that send audio beyond this limit are disconnected
/// gracefully with a `{"type":"error"}` frame so the server never OOMs (issue #1924).
const MAX_FULL_AUDIO_SAMPLES: usize = AUDIO_SAMPLE_RATE * 60 * 5; // ~5 minutes

#[derive(Debug, Deserialize)]
struct ClientCommand {
    #[serde(rename = "type")]
    cmd_type: String,
}

fn decode_pcm16le_frame(data: &[u8]) -> Option<Vec<i16>> {
    if !data.len().is_multiple_of(2) {
        return None;
    }

    Some(
        data.chunks_exact(2)
            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
            .collect(),
    )
}

/// Append `samples` to both the sliding window buffer and the full-audio accumulation
/// buffer, enforcing the hard cap on the latter.
///
/// Returns `true` when the full-audio buffer is within the allowed limit (normal path).
/// Returns `false` when appending `samples` would push `full_audio_buf` beyond
/// `MAX_FULL_AUDIO_SAMPLES`; in that case the samples are **not** appended and the caller
/// must disconnect the client to prevent unbounded memory growth (issue #1924).
fn append_stream_samples(
    audio_buf: &mut Vec<i16>,
    full_audio_buf: &mut Vec<i16>,
    samples: &[i16],
) -> bool {
    // Enforce hard cap on the full-audio accumulation buffer first.
    if full_audio_buf.len().saturating_add(samples.len()) > MAX_FULL_AUDIO_SAMPLES {
        log::warn!(
            "{LOG_PREFIX} full_audio_buf cap reached ({} / {} samples); refusing to append {} \
             more samples — client will be disconnected",
            full_audio_buf.len(),
            MAX_FULL_AUDIO_SAMPLES,
            samples.len(),
        );
        return false;
    }

    full_audio_buf.extend_from_slice(samples);
    audio_buf.extend_from_slice(samples);
    if audio_buf.len() > MAX_STREAM_BUFFER_SAMPLES {
        let drop_count = audio_buf.len() - MAX_STREAM_BUFFER_SAMPLES;
        audio_buf.drain(..drop_count);
        log::debug!(
            "{LOG_PREFIX} sliding window trimmed {} samples, kept {}",
            drop_count,
            audio_buf.len()
        );
    }
    true
}

fn is_stop_command(text: &str) -> bool {
    serde_json::from_str::<ClientCommand>(text)
        .map(|cmd| cmd.cmd_type == "stop")
        .unwrap_or(false)
}

/// Handle an upgraded WebSocket connection for streaming dictation.
pub async fn handle_dictation_ws(mut socket: WebSocket, config: Arc<Config>) {
    log::info!("{LOG_PREFIX} new streaming dictation connection");

    let audio_buf: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
    let full_audio_buf: Arc<Mutex<Vec<i16>>> = Arc::new(Mutex::new(Vec::new()));
    if config.dictation.streaming {
        log::debug!(
            "{LOG_PREFIX} dictation.streaming is on but the configured STT engine has no partial \
             transcription path — only the final result will be sent"
        );
    }

    loop {
        match socket.recv().await {
            Some(Ok(Message::Binary(data))) => {
                let Some(samples) = decode_pcm16le_frame(&data) else {
                    log::warn!("{LOG_PREFIX} received odd-length binary frame, skipping");
                    continue;
                };

                let cap_exceeded = {
                    let mut full = full_audio_buf.lock().await;
                    let mut buf = audio_buf.lock().await;
                    let ok = append_stream_samples(&mut buf, &mut full, &samples);
                    if ok {
                        log::trace!(
                            "{LOG_PREFIX} buffered {} new samples, total {}",
                            samples.len(),
                            buf.len()
                        );
                    }
                    !ok
                };

                if cap_exceeded {
                    // Send an error frame and close — never OOM.
                    let err_msg = serde_json::json!({
                        "type": "error",
                        "message": format!(
                            "Recording limit reached: maximum {} minutes of audio per session",
                            MAX_FULL_AUDIO_SAMPLES / AUDIO_SAMPLE_RATE / 60
                        ),
                    });
                    let _ = socket.send(Message::Text(err_msg.to_string().into())).await;
                    log::warn!(
                        "{LOG_PREFIX} disconnecting client: full_audio_buf cap ({} samples, \
                         {} min at 16 kHz) exceeded",
                        MAX_FULL_AUDIO_SAMPLES,
                        MAX_FULL_AUDIO_SAMPLES / AUDIO_SAMPLE_RATE / 60,
                    );
                    return;
                }
            }

            Some(Ok(Message::Text(text))) => {
                if is_stop_command(&text) {
                    log::info!("{LOG_PREFIX} stop command received, running final transcription");
                    break; // fall through to final transcription
                }
            }

            Some(Ok(Message::Close(_))) | None => {
                log::info!("{LOG_PREFIX} client disconnected");
                return;
            }

            Some(Err(e)) => {
                log::warn!("{LOG_PREFIX} websocket error: {e}");
                return;
            }

            _ => {}
        }
    }

    // Run the final transcription on the complete buffer.
    let final_samples = full_audio_buf.lock().await.clone();
    if final_samples.is_empty() {
        let msg = serde_json::json!({
            "type": "final",
            "text": "",
            "raw_text": "",
        });
        let _ = socket.send(Message::Text(msg.to_string().into())).await;
        return;
    }

    log::info!(
        "{LOG_PREFIX} transcribing {} samples ({:.1}s) via the configured STT engine",
        final_samples.len(),
        final_samples.len() as f64 / AUDIO_SAMPLE_RATE as f64
    );

    // The client streams headerless PCM16LE; the hosted endpoint needs a
    // container, so wrap it in a WAV before upload. `EncodeWavPcm16` keeps the
    // samples exact rather than round-tripping them through `f32`.
    let wav_bytes = match crate::openhuman::modules::voice::encode_wav_pcm16(
        &config,
        &final_samples,
        AUDIO_SAMPLE_RATE as u32,
        1,
    )
    .await
    {
        Ok(bytes) => bytes,
        Err(error) => {
            // Tell the client before dropping the socket. WAV framing became
            // fallible when it moved to the module, and a client waiting for
            // `final` or `error` would otherwise get neither — every other
            // failure path in this handler sends a frame first.
            //
            // The frame carries no module detail, matching the redaction the
            // transcription-failure path below uses: the reason is a host
            // concern and goes to the log, not to the renderer.
            log::warn!("{LOG_PREFIX} could not frame dictation audio as WAV: {error}");
            let err_msg = serde_json::json!({
                "type": "error",
                "message": "Could not prepare the recording for transcription",
            });
            let _ = socket.send(Message::Text(err_msg.to_string().into())).await;
            return;
        }
    };
    let provider_name = effective_stt_provider(&config);
    let audio_base64 = BASE64.encode(&wav_bytes);
    let raw_text = match async {
        let provider =
            create_stt_provider(&provider_name, "", &config).map_err(|error| error.to_string())?;
        provider
            .transcribe(
                &config,
                &audio_base64,
                Some("audio/wav"),
                Some("dictation.wav"),
                None,
            )
            .await
    }
    .await
    {
        Ok(outcome) => outcome.value.text,
        Err(e) => {
            log::warn!("{LOG_PREFIX} configured transcription failed: {e}");
            let msg = serde_json::json!({
                "type": "error",
                "message": format!("Transcription failed: {e}"),
            });
            let _ = socket.send(Message::Text(msg.to_string().into())).await;
            return;
        }
    };

    // LLM refinement if enabled
    let refined_text = if config.dictation.llm_refinement && !raw_text.is_empty() {
        postprocess::cleanup_transcription(&config, &raw_text, None).await
    } else {
        raw_text.clone()
    };

    let msg = serde_json::json!({
        "type": "final",
        "text": refined_text,
        "raw_text": raw_text,
    });
    let _ = socket.send(Message::Text(msg.to_string().into())).await;
    log::info!("{LOG_PREFIX} streaming session complete");
    // Socket is dropped here, which sends a close frame automatically
}

#[cfg(test)]
#[path = "streaming_tests.rs"]
mod tests;
