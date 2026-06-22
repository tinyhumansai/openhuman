//! WebSocket streaming transcription endpoint.
//!
//! Accepts a WebSocket connection that receives PCM16 audio chunks (16kHz mono)
//! and periodically runs whisper inference on the accumulated buffer, sending
//! back partial transcription results as JSON messages.
//!
//! Protocol:
//!   Client → Server: binary frames containing PCM16 LE audio bytes (16kHz mono)
//!   Server → Client: JSON text frames:
//!     { "type": "partial",  "text": "..." }          — interim transcription
//!     { "type": "final",    "text": "...", "raw_text": "..." } — after client sends
//!                                                        `{"type":"stop"}` text frame
//!     { "type": "error",    "message": "..." }        — on error
//!   Client → Server: text frame `{"type":"stop"}`     — end recording, get final result
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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::postprocess;
use crate::openhuman::config::Config;
use crate::openhuman::inference::local as local_ai;
use crate::openhuman::inference::local::service::whisper_engine;
use crate::openhuman::util::utf8_safe_prefix_at_byte_boundary;

const LOG_PREFIX: &str = "[voice-stream]";
const AUDIO_SAMPLE_RATE: usize = 16_000;
const MIN_PARTIAL_SAMPLES: usize = AUDIO_SAMPLE_RATE / 2; // 0.5s
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
    let audio_revision = Arc::new(AtomicU64::new(0));
    let interval_ms = config.dictation.streaming_interval_ms;
    let do_streaming = config.dictation.streaming;

    // Periodic inference task — runs every `interval_ms` on the accumulated buffer
    let buf_clone = audio_buf.clone();
    let revision_clone = audio_revision.clone();
    let config_clone = config.clone();
    let (partial_tx, mut partial_rx) = tokio::sync::mpsc::channel::<String>(8);

    let inference_handle = if do_streaming {
        let handle = tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_millis(interval_ms.max(500)));
            let mut last_seen_revision = 0u64;

            loop {
                interval.tick().await;

                let current_revision = revision_clone.load(Ordering::Relaxed);
                if current_revision == last_seen_revision {
                    continue;
                }
                last_seen_revision = current_revision;

                let samples: Vec<i16> = {
                    let guard = buf_clone.lock().await;
                    if guard.len() < MIN_PARTIAL_SAMPLES {
                        // Less than 0.5s of audio — skip
                        continue;
                    }
                    guard.clone()
                };

                let service = local_ai::global(&config_clone);
                match whisper_engine::transcribe_pcm_i16(&service.whisper, &samples, None, None) {
                    Ok(result) => {
                        if !result.text.is_empty() {
                            log::debug!(
                                "{LOG_PREFIX} partial transcription ({} samples, avg_logprob={:.3}): {}",
                                samples.len(),
                                result.avg_logprob.unwrap_or(0.0),
                                utf8_safe_prefix_at_byte_boundary(&result.text, 80)
                            );
                            if partial_tx.send(result.text).await.is_err() {
                                break; // receiver dropped
                            }
                        }
                    }
                    Err(e) => {
                        log::warn!("{LOG_PREFIX} partial inference error: {e}");
                    }
                }
            }
        });
        Some(handle)
    } else {
        None
    };

    loop {
        tokio::select! {
            // Forward partial results to the client
            Some(partial_text) = partial_rx.recv() => {
                let msg = serde_json::json!({
                    "type": "partial",
                    "text": partial_text,
                });
                if socket.send(Message::Text(msg.to_string().into())).await.is_err() {
                    log::debug!("{LOG_PREFIX} client disconnected while sending partial");
                    break;
                }
            }

            // Receive audio data or commands from the client
            msg = socket.recv() => {
                match msg {
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
                                audio_revision.fetch_add(1, Ordering::Relaxed);
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
                            let _ = socket
                                .send(Message::Text(err_msg.to_string().into()))
                                .await;
                            log::warn!(
                                "{LOG_PREFIX} disconnecting client: full_audio_buf cap ({} samples, \
                                 {} min at 16 kHz) exceeded",
                                MAX_FULL_AUDIO_SAMPLES,
                                MAX_FULL_AUDIO_SAMPLES / AUDIO_SAMPLE_RATE / 60,
                            );
                            if let Some(h) = inference_handle {
                                h.abort();
                            }
                            return;
                        }
                    }

                    Some(Ok(Message::Text(text))) => {
                        if is_stop_command(&text) {
                            log::info!("{LOG_PREFIX} stop command received, running final inference");
                            break; // fall through to final transcription
                        }
                    }

                    Some(Ok(Message::Close(_))) | None => {
                        log::info!("{LOG_PREFIX} client disconnected");
                        if let Some(h) = inference_handle {
                            h.abort();
                        }
                        return;
                    }

                    Some(Err(e)) => {
                        log::warn!("{LOG_PREFIX} websocket error: {e}");
                        if let Some(h) = inference_handle {
                            h.abort();
                        }
                        return;
                    }

                    _ => {}
                }
            }
        }
    }

    // Stop the periodic inference task
    if let Some(h) = inference_handle {
        h.abort();
    }

    // Run final transcription on the complete buffer
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
        "{LOG_PREFIX} running final inference on {} samples ({:.1}s)",
        final_samples.len(),
        final_samples.len() as f64 / 16000.0
    );

    let service = local_ai::global(&config);
    let raw_text =
        match whisper_engine::transcribe_pcm_i16(&service.whisper, &final_samples, None, None) {
            Ok(result) => result.text,
            Err(e) => {
                log::error!("{LOG_PREFIX} final inference error: {e}");
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
mod tests {
    use super::*;

    #[test]
    fn decode_pcm16le_frame_rejects_odd_length() {
        assert!(decode_pcm16le_frame(&[1, 2, 3]).is_none());
    }

    #[test]
    fn decode_pcm16le_frame_decodes_samples() {
        let samples = decode_pcm16le_frame(&[0x01, 0x00, 0xff, 0xff]).expect("decode");
        assert_eq!(samples, vec![1, -1]);
    }

    #[test]
    fn append_stream_samples_keeps_full_audio_and_trims_window() {
        let mut audio = vec![0; MAX_STREAM_BUFFER_SAMPLES - 2];
        let mut full = vec![1, 2];
        let ok = append_stream_samples(&mut audio, &mut full, &[3, 4, 5, 6]);

        assert!(ok, "should succeed when under cap");
        assert_eq!(full, vec![1, 2, 3, 4, 5, 6]);
        assert_eq!(audio.len(), MAX_STREAM_BUFFER_SAMPLES);
        assert_eq!(&audio[audio.len() - 4..], &[3, 4, 5, 6]);
    }

    /// Feed enough samples to hit the full-audio cap and verify:
    /// 1. The buffer does NOT grow past `MAX_FULL_AUDIO_SAMPLES`.
    /// 2. `append_stream_samples` returns `false` (cap-exceeded signal) when the next
    ///    chunk would overflow.
    #[test]
    fn append_stream_samples_enforces_full_audio_cap() {
        let chunk_size = 1_024usize;
        let mut audio = Vec::new();
        let mut full = Vec::new();

        // Fill up to exactly the cap in chunks.
        let full_chunks = MAX_FULL_AUDIO_SAMPLES / chunk_size;
        let chunk = vec![0i16; chunk_size];
        for _ in 0..full_chunks {
            let ok = append_stream_samples(&mut audio, &mut full, &chunk);
            assert!(ok, "should succeed while under cap");
        }

        // The buffer may now be at or just below MAX_FULL_AUDIO_SAMPLES (depending on
        // whether MAX_FULL_AUDIO_SAMPLES is an exact multiple of chunk_size).
        assert!(
            full.len() <= MAX_FULL_AUDIO_SAMPLES,
            "full_audio_buf must not exceed cap before overflow chunk"
        );

        // One more chunk must be rejected.
        let extra = vec![1i16; chunk_size];
        let ok = append_stream_samples(&mut audio, &mut full, &extra);
        assert!(
            !ok,
            "must return false (cap exceeded) when appending would overflow"
        );

        // The buffer must not have grown.
        assert!(
            full.len() <= MAX_FULL_AUDIO_SAMPLES,
            "full_audio_buf must not exceed MAX_FULL_AUDIO_SAMPLES after cap is hit"
        );
    }

    /// A single oversized chunk that would exceed the cap on its own must also be rejected.
    #[test]
    fn append_stream_samples_rejects_single_oversized_chunk() {
        let mut audio = Vec::new();
        let mut full = Vec::new();

        // Pre-fill to near the cap (1 sample short).
        let near_full = vec![0i16; MAX_FULL_AUDIO_SAMPLES - 1];
        let ok = append_stream_samples(&mut audio, &mut full, &near_full);
        assert!(ok, "pre-fill should succeed");

        // A 2-sample chunk would push us 1 sample over the cap.
        let ok = append_stream_samples(&mut audio, &mut full, &[7, 8]);
        assert!(!ok, "must return false when chunk crosses the cap boundary");
        assert!(
            full.len() <= MAX_FULL_AUDIO_SAMPLES,
            "full_audio_buf must not exceed cap"
        );
    }

    #[test]
    fn append_stream_samples_returns_false_when_full_audio_cap_reached() {
        let mut audio = vec![];
        let mut full = vec![0i16; MAX_FULL_AUDIO_SAMPLES];
        let ok = append_stream_samples(&mut audio, &mut full, &[1, 2, 3]);

        assert!(!ok, "should return false once cap is reached");
        assert_eq!(
            full.len(),
            MAX_FULL_AUDIO_SAMPLES,
            "full buf must not grow past cap"
        );
        assert!(
            audio.is_empty(),
            "sliding window must not receive new samples"
        );
    }

    #[test]
    fn is_stop_command_only_accepts_stop_type() {
        assert!(is_stop_command(r#"{"type":"stop"}"#));
        assert!(!is_stop_command(r#"{"type":"continue"}"#));
        assert!(!is_stop_command("not json"));
    }
}
