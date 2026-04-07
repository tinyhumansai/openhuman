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

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::extract::ws::{Message, WebSocket};
use serde::Deserialize;
use tokio::sync::Mutex;

use super::postprocess;
use crate::openhuman::config::Config;
use crate::openhuman::local_ai;
use crate::openhuman::local_ai::whisper_engine;

const LOG_PREFIX: &str = "[voice-stream]";
const AUDIO_SAMPLE_RATE: usize = 16_000;
const MIN_PARTIAL_SAMPLES: usize = AUDIO_SAMPLE_RATE / 2; // 0.5s
const MAX_STREAM_BUFFER_SAMPLES: usize = AUDIO_SAMPLE_RATE * 15; // 15s sliding window

#[derive(Debug, Deserialize)]
struct ClientCommand {
    #[serde(rename = "type")]
    cmd_type: String,
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
                                &result.text[..result.text.len().min(80)]
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
                        // PCM16 LE bytes → i16 samples
                        if data.len() % 2 != 0 {
                            log::warn!("{LOG_PREFIX} received odd-length binary frame, skipping");
                            continue;
                        }
                        let samples: Vec<i16> = data
                            .chunks_exact(2)
                            .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
                            .collect();

                        full_audio_buf.lock().await.extend_from_slice(&samples);
                        let mut buf = audio_buf.lock().await;
                        buf.extend_from_slice(&samples);
                        if buf.len() > MAX_STREAM_BUFFER_SAMPLES {
                            let drop_count = buf.len() - MAX_STREAM_BUFFER_SAMPLES;
                            buf.drain(..drop_count);
                            log::debug!(
                                "{LOG_PREFIX} sliding window trimmed {} samples, kept {}",
                                drop_count,
                                buf.len()
                            );
                        }
                        audio_revision.fetch_add(1, Ordering::Relaxed);
                        log::trace!(
                            "{LOG_PREFIX} buffered {} new samples, total {}",
                            samples.len(),
                            buf.len()
                        );
                    }

                    Some(Ok(Message::Text(text))) => {
                        if let Ok(cmd) = serde_json::from_str::<ClientCommand>(&text) {
                            if cmd.cmd_type == "stop" {
                                log::info!("{LOG_PREFIX} stop command received, running final inference");
                                break; // fall through to final transcription
                            }
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
