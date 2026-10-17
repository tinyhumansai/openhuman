//! `GET /ws/live-voice`: one live voice session per WebSocket.
//!
//! The client opens with a `start` frame ([`ClientFrame::Start`]), then streams
//! microphone PCM16 as binary frames and may send `text`, `interrupt` and
//! `stop`. The core answers with [`ServerFrame`] JSON events and the agent's
//! speech as binary PCM16 at the `ready` frame's `output_sample_rate`. Final
//! transcripts are saved to the session's thread as they arrive.
//!
//! Logs carry the session id, provider and event kinds — never audio or
//! transcript text.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::ws::{Message, WebSocket};
use bytes::Bytes;
use futures::{SinkExt, StreamExt};
use tinyagents_live::tinyliveagents::{CloseReason, LiveEvent};
use tinyagents_live::LiveAgentEvent;

use super::error::LiveVoiceError;
use super::persist::TranscriptPersister;
use super::session::{self, LiveStartRequest};
use super::types::{ClientFrame, ServerFrame, TranscriptRole};
use crate::config::Config;

const LOG_PREFIX: &str = "[voice-live]";
/// How long a client may take to send its `start` frame.
pub(crate) const START_TIMEOUT: Duration = Duration::from_secs(15);

/// What one session event turns into on the socket.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Outbound {
    Json(ServerFrame),
    Audio(Bytes),
    Persist(TranscriptRole, String),
}

/// Translates one session event. `session_id` and `thread_id` fill the
/// `ready` frame.
pub(crate) fn map_event(
    event: LiveAgentEvent,
    session_id: &str,
    provider: &str,
    thread_id: &str,
) -> Vec<Outbound> {
    match event {
        LiveAgentEvent::ToolStarted { call_id, name } => {
            vec![Outbound::Json(ServerFrame::ToolStarted { call_id, name })]
        }
        LiveAgentEvent::ToolFinished {
            call_id,
            name,
            is_error,
            cancelled,
            ..
        } => vec![Outbound::Json(ServerFrame::ToolFinished {
            call_id,
            name,
            ok: !is_error,
            cancelled,
        })],
        LiveAgentEvent::Live(event) => match event {
            LiveEvent::Ready(info) => vec![Outbound::Json(ServerFrame::Ready {
                session_id: Some(info.session_id.unwrap_or_else(|| session_id.to_string())),
                provider: provider.to_string(),
                output_sample_rate: info.output_format.sample_rate,
                thread_id: Some(thread_id.to_string()),
            })],
            LiveEvent::Audio(pcm) => vec![Outbound::Audio(pcm)],
            LiveEvent::InputTranscript { text, is_final } => {
                transcript(TranscriptRole::User, text, is_final)
            }
            LiveEvent::OutputTranscript { text, is_final } => {
                transcript(TranscriptRole::Agent, text, is_final)
            }
            LiveEvent::Interrupted => vec![Outbound::Json(ServerFrame::Interrupted)],
            LiveEvent::TurnComplete { .. } => vec![Outbound::Json(ServerFrame::TurnComplete)],
            LiveEvent::Error { error, fatal } => {
                let error = LiveVoiceError::from(error);
                vec![Outbound::Json(ServerFrame::Error {
                    code: error.code.to_string(),
                    message: error.message,
                    fatal,
                })]
            }
            LiveEvent::Closed(reason) => {
                let mut out = Vec::new();
                let reason = match reason {
                    CloseReason::Client => "client".to_string(),
                    CloseReason::Remote { code, reason } => match code {
                        Some(code) => format!("remote ({code}) {reason}").trim().to_string(),
                        None => "remote".to_string(),
                    },
                    CloseReason::Error(error) => {
                        let error = LiveVoiceError::from(error);
                        out.push(Outbound::Json(ServerFrame::Error {
                            code: error.code.to_string(),
                            message: error.message,
                            fatal: true,
                        }));
                        "error".to_string()
                    }
                    _ => "closed".to_string(),
                };
                out.push(Outbound::Json(ServerFrame::Closed { reason }));
                out
            }
            // Tool calls are reported through ToolStarted/ToolFinished; the
            // rest has no client-visible meaning.
            _ => Vec::new(),
        },
    }
}

fn transcript(role: TranscriptRole, text: String, is_final: bool) -> Vec<Outbound> {
    let mut out = vec![Outbound::Json(ServerFrame::Transcript {
        role,
        text: text.clone(),
        is_final,
    })];
    if is_final {
        out.push(Outbound::Persist(role, text));
    }
    out
}

fn json_message(frame: &ServerFrame) -> Message {
    Message::Text(serde_json::to_string(frame).unwrap_or_default().into())
}

fn error_frame(error: &LiveVoiceError) -> ServerFrame {
    ServerFrame::Error {
        code: error.code.to_string(),
        message: error.message.clone(),
        fatal: true,
    }
}

/// Waits for and parses the `start` frame.
pub(crate) fn parse_start(message: &Message) -> Result<LiveStartRequest, LiveVoiceError> {
    let Message::Text(text) = message else {
        return Err(LiveVoiceError::invalid(
            "the first frame must be a start frame",
        ));
    };
    match serde_json::from_str::<ClientFrame>(text) {
        Ok(ClientFrame::Start {
            provider,
            thread_id,
            client_id,
            input_sample_rate,
        }) => Ok(LiveStartRequest {
            provider,
            thread_id,
            client_id,
            input_sample_rate,
        }),
        Ok(_) => Err(LiveVoiceError::invalid(
            "the first frame must be a start frame",
        )),
        Err(error) => Err(LiveVoiceError::invalid(format!("bad start frame: {error}"))),
    }
}

/// Runs one live voice WebSocket to completion.
///
/// `ctx` is the context the accepting request ran under: an upgraded socket
/// outlives (and is spawned off) its request task, so it cannot be read back
/// from the task.
pub async fn handle_live_voice_ws(
    socket: WebSocket,
    ctx: crate::core::runtime::DispatchCtx,
    config: Arc<Config>,
) {
    let session_id = uuid::Uuid::new_v4().to_string();
    let (mut sink, mut stream) = socket.split();

    let first = match tokio::time::timeout(START_TIMEOUT, stream.next()).await {
        Ok(Some(Ok(message))) => message,
        _ => {
            tracing::debug!(session_id, "{LOG_PREFIX} no start frame; closing");
            let _ = sink.close().await;
            return;
        }
    };
    let request = match parse_start(&first) {
        Ok(request) => request,
        Err(error) => {
            let _ = sink.send(json_message(&error_frame(&error))).await;
            let _ = sink.close().await;
            return;
        }
    };

    let started = match session::start(&ctx, config.clone(), request, &session_id).await {
        Ok(started) => started,
        Err(error) => {
            tracing::warn!(
                session_id,
                code = error.code,
                "{LOG_PREFIX} session failed to start"
            );
            let _ = sink.send(json_message(&error_frame(&error))).await;
            let _ = sink
                .send(json_message(&ServerFrame::Closed {
                    reason: "error".into(),
                }))
                .await;
            let _ = sink.close().await;
            return;
        }
    };
    let session::StartedLiveSession {
        mut session,
        provider,
        thread_id,
    } = started;
    tracing::info!(session_id, provider = %provider, "{LOG_PREFIX} session started");
    let sender = session.sender();
    let mut persister = TranscriptPersister::new(
        ctx.clone(),
        config.workspace_dir.clone(),
        thread_id.clone(),
        session_id.clone(),
        provider.clone(),
    );

    loop {
        tokio::select! {
            incoming = stream.next() => {
                let message = match incoming {
                    None | Some(Err(_)) | Some(Ok(Message::Close(_))) => {
                        let _ = sender.close().await;
                        break;
                    }
                    Some(Ok(message)) => message,
                };
                let result = match message {
                    Message::Binary(pcm) => sender.send_audio(pcm).await,
                    Message::Text(text) => match serde_json::from_str::<ClientFrame>(&text) {
                        Ok(ClientFrame::Text { text }) => {
                            persister.save(TranscriptRole::User, &text).await;
                            sender.send_text(text).await
                        }
                        Ok(ClientFrame::Interrupt) => sender.interrupt().await,
                        Ok(ClientFrame::Stop) => {
                            let _ = sender.close().await;
                            Ok(())
                        }
                        Ok(ClientFrame::Start { .. }) | Err(_) => {
                            tracing::debug!(session_id, "{LOG_PREFIX} ignoring unexpected client frame");
                            Ok(())
                        }
                    },
                    _ => Ok(()),
                };
                if result.is_err() {
                    tracing::debug!(session_id, "{LOG_PREFIX} provider session already closed");
                }
            }
            event = session.recv() => {
                let Some(event) = event else { break };
                let closing = matches!(event, LiveAgentEvent::Live(LiveEvent::Closed(_)));
                for outbound in map_event(event, &session_id, &provider, &thread_id) {
                    let sent = match outbound {
                        Outbound::Json(frame) => sink.send(json_message(&frame)).await,
                        Outbound::Audio(pcm) => sink.send(Message::Binary(pcm)).await,
                        Outbound::Persist(role, text) => {
                            persister.save(role, &text).await;
                            Ok(())
                        }
                    };
                    if sent.is_err() {
                        tracing::debug!(session_id, "{LOG_PREFIX} client went away");
                        let _ = sender.close().await;
                    }
                }
                if closing {
                    break;
                }
            }
        }
    }
    tracing::info!(session_id, provider = %provider, "{LOG_PREFIX} session ended");
    let _ = sink.close().await;
}

#[cfg(test)]
#[path = "ws_tests.rs"]
mod tests;
