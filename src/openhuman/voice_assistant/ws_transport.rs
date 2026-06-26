//! WebSocket streaming audio transport for voice assistant.
//!
//! Provides a bidirectional WebSocket connection for real-time audio streaming,
//! eliminating the polling overhead of the JSON-RPC push_audio/poll_response cycle.
//!
//! ## Protocol
//!
//! Client → Server (binary): Raw PCM16LE frames @ 16kHz mono.
//! Server → Client (binary): Raw PCM16LE TTS output frames.
//! Server → Client (text): JSON status messages:
//!   `{"type":"transcript","text":"...","is_final":true}`
//!   `{"type":"state","state":"listening"|"processing"|"speaking"}`
//!   `{"type":"emotion","label":"positive","confidence":0.8}`
//!   `{"type":"language","code":"en","confidence":0.9}`
//!   `{"type":"error","message":"..."}`
//!
//! ## Connection lifecycle
//!
//! 1. Client connects to `/ws/voice/{session_id}`
//! 2. Server validates session exists
//! 3. Client streams PCM binary frames
//! 4. Server streams back TTS PCM + JSON status updates
//! 5. Either side can close the connection
//!
//! ## Log prefix
//!
//! `[voice-assistant-ws]`

use serde::Serialize;
use tracing::debug;

use super::session::SessionRegistry;
use super::types::SessionState;
use crate::openhuman::meet_agent::ops::VadEvent;

const LOG_PREFIX: &str = "[voice-assistant-ws]";

/// Maximum binary frame size: 32KB (1 second @ 16kHz 16-bit).
const MAX_FRAME_SIZE: usize = 32_768;

/// WebSocket status message types.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum WsMessage {
    Transcript { text: String, is_final: bool },
    State { state: SessionState },
    Emotion { label: String, confidence: f64 },
    Language { code: String, confidence: f64 },
    Error { message: String },
    Interrupted { discarded_samples: usize },
}

/// Process an incoming binary PCM frame from WebSocket.
/// Returns any outbound messages to send back.
pub fn process_ws_frame(session_id: &str, pcm_bytes: &[u8]) -> Vec<WsOutbound> {
    if pcm_bytes.len() > MAX_FRAME_SIZE {
        return vec![WsOutbound::Text(
            serde_json::to_string(&WsMessage::Error {
                message: format!("frame too large: {} > {MAX_FRAME_SIZE}", pcm_bytes.len()),
            })
            .unwrap_or_default(),
        )];
    }

    // Decode PCM16LE.
    if pcm_bytes.len() % 2 != 0 {
        return vec![WsOutbound::Text(
            serde_json::to_string(&WsMessage::Error {
                message: "odd byte count in PCM frame".into(),
            })
            .unwrap_or_default(),
        )];
    }

    let samples: Vec<i16> = pcm_bytes
        .chunks_exact(2)
        .map(|c| i16::from_le_bytes([c[0], c[1]]))
        .collect();

    let mut outbound = Vec::new();

    // Push to session and check VAD.
    let event = match SessionRegistry::with_session(session_id, |s| s.push_inbound_pcm(&samples)) {
        Ok(event) => event,
        Err(e) => {
            return vec![WsOutbound::Text(
                serde_json::to_string(&WsMessage::Error { message: e }).unwrap_or_default(),
            )];
        }
    };

    // If VAD fired, trigger brain turn (same as RPC push_audio path).
    if matches!(event, VadEvent::EndOfUtterance) {
        outbound.push(WsOutbound::Text(
            serde_json::to_string(&WsMessage::State {
                state: SessionState::Processing,
            })
            .unwrap_or_default(),
        ));
        // Spawn brain turn with processing lock (prevents concurrent turns).
        let sid = session_id.to_string();
        let acquired = SessionRegistry::try_acquire_processing(&sid).unwrap_or(false);
        if acquired {
            tokio::spawn(async move {
                struct Guard(String);
                impl Drop for Guard {
                    fn drop(&mut self) {
                        SessionRegistry::release_processing(&self.0);
                    }
                }
                let _guard = Guard(sid.clone());
                if let Err(e) = super::brain::run_turn(&sid).await {
                    debug!("{LOG_PREFIX} brain turn failed for ws session {sid}: {e}");
                }
            });
        }
    }

    // Check for outbound audio.
    if let Ok((pcm_b64, transcript, reply, state, emotion, language)) =
        SessionRegistry::with_session(session_id, |s| {
            let (pcm, _done) = s.poll_outbound();
            (
                pcm,
                s.last_transcript.clone(),
                s.last_reply.clone(),
                s.state,
                s.detected_emotion.clone(),
                s.detected_language.clone(),
            )
        })
    {
        // Send state update.
        outbound.push(WsOutbound::Text(
            serde_json::to_string(&WsMessage::State { state }).unwrap_or_default(),
        ));

        // Send transcript if available.
        if !transcript.is_empty() {
            outbound.push(WsOutbound::Text(
                serde_json::to_string(&WsMessage::Transcript {
                    text: transcript,
                    is_final: true,
                })
                .unwrap_or_default(),
            ));
        }

        // Send emotion if detected.
        if let Some(label) = emotion {
            outbound.push(WsOutbound::Text(
                serde_json::to_string(&WsMessage::Emotion {
                    label,
                    confidence: 0.8,
                })
                .unwrap_or_default(),
            ));
        }

        // Send language if detected.
        if let Some(code) = language {
            outbound.push(WsOutbound::Text(
                serde_json::to_string(&WsMessage::Language {
                    code,
                    confidence: 0.9,
                })
                .unwrap_or_default(),
            ));
        }

        // Send outbound PCM as binary.
        if !pcm_b64.is_empty() {
            if let Ok(bytes) =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, &pcm_b64)
            {
                outbound.push(WsOutbound::Binary(bytes));
            }
        }
    }

    outbound
}

/// Outbound WebSocket message.
#[derive(Debug)]
pub enum WsOutbound {
    Text(String),
    Binary(Vec<u8>),
}

/// Build an Axum Router with the WebSocket voice endpoint mounted.
///
/// Mount this on your HTTP server:
/// ```ignore
/// let app = your_router.merge(ws_router());
/// ```
pub fn ws_router() -> axum::Router {
    use axum::{
        extract::{ws::WebSocket, Path, WebSocketUpgrade},
        response::IntoResponse,
        routing::get,
        Router,
    };

    async fn ws_handler(Path(session_id): Path<String>, ws: WebSocketUpgrade) -> impl IntoResponse {
        ws.on_upgrade(move |socket| handle_ws(session_id, socket))
    }

    async fn handle_ws(session_id: String, mut socket: WebSocket) {
        use axum::extract::ws::Message;
        use tracing::info;

        // Validate session exists.
        if SessionRegistry::with_session(&session_id, |_| {}).is_err() {
            let _ = socket
                .send(Message::Text(
                    serde_json::to_string(&WsMessage::Error {
                        message: format!("session not found: {session_id}"),
                    })
                    .unwrap_or_default()
                    .into(),
                ))
                .await;
            return;
        }

        info!("{LOG_PREFIX} ws connected session={session_id}");

        while let Some(Ok(msg)) = socket.recv().await {
            match msg {
                Message::Binary(data) => {
                    let responses = process_ws_frame(&session_id, &data);
                    for resp in responses {
                        let ws_msg = match resp {
                            WsOutbound::Text(t) => Message::Text(t.into()),
                            WsOutbound::Binary(b) => Message::Binary(b.into()),
                        };
                        if socket.send(ws_msg).await.is_err() {
                            break;
                        }
                    }
                }
                Message::Close(_) => break,
                _ => {}
            }
        }

        info!("{LOG_PREFIX} ws disconnected session={session_id}");
    }

    Router::new().route("/ws/voice/{session_id}", get(ws_handler))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ws_message_serializes() {
        let msg = WsMessage::Transcript {
            text: "hello".into(),
            is_final: true,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"type\":\"transcript\""));
        assert!(json.contains("\"is_final\":true"));
    }

    #[test]
    fn ws_message_state_serializes() {
        let msg = WsMessage::State {
            state: SessionState::Listening,
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains("\"state\":\"listening\""));
    }

    #[test]
    fn process_ws_frame_rejects_oversized() {
        let big = vec![0u8; MAX_FRAME_SIZE + 1];
        let result = process_ws_frame("nonexistent", &big);
        assert_eq!(result.len(), 1);
        match &result[0] {
            WsOutbound::Text(t) => assert!(t.contains("frame too large")),
            _ => panic!("expected text error"),
        }
    }

    #[test]
    fn process_ws_frame_rejects_odd_bytes() {
        let odd = vec![0u8; 3];
        let result = process_ws_frame("nonexistent", &odd);
        assert_eq!(result.len(), 1);
        match &result[0] {
            WsOutbound::Text(t) => assert!(t.contains("odd byte count")),
            _ => panic!("expected text error"),
        }
    }
}
