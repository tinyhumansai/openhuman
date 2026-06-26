//! JSON-RPC handlers for the `voice_assistant` domain.
//!
//! Five endpoints:
//!
//! - `start_session`   — open a voice assistant session
//! - `push_audio`      — feed PCM frames; may trigger a brain turn
//! - `poll_response`   — pull synthesized PCM + text out
//! - `get_status`      — query session state
//! - `stop_session`    — close + return summary
//!
//! Each handler is short — heavy lifting lives in `session.rs` (state)
//! and `brain.rs` (behavior).

use serde_json::{json, Map, Value};
use tracing::info;

use crate::rpc::RpcOutcome;

use super::brain;
use super::session::SessionRegistry;
use super::types::*;
use crate::openhuman::meet_agent::ops::VadEvent;
use crate::openhuman::meet_agent::wav::decode_pcm16le_b64;

const LOG_PREFIX: &str = "[voice-assistant-rpc]";

pub async fn handle_start_session(params: Map<String, Value>) -> Result<Value, String> {
    let req: StartSessionRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("{LOG_PREFIX} invalid start_session params: {e}"))?;

    let session_id = req
        .session_id
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());

    SessionRegistry::start(
        &session_id,
        &req.stt_provider,
        &req.tts_provider,
        req.language.as_deref(),
    )?;

    info!(
        "{LOG_PREFIX} start_session id={} stt={} tts={}",
        session_id, req.stt_provider, req.tts_provider
    );

    RpcOutcome::new(
        json!({
            "ok": true,
            "session_id": session_id,
            "stt_provider": req.stt_provider,
            "tts_provider": req.tts_provider,
        }),
        vec![],
    )
    .into_cli_compatible_json()
}

pub async fn handle_push_audio(params: Map<String, Value>) -> Result<Value, String> {
    let req: PushAudioRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("{LOG_PREFIX} invalid push_audio params: {e}"))?;

    let samples =
        decode_pcm16le_b64(&req.pcm_base64).map_err(|e| format!("{LOG_PREFIX} pcm decode: {e}"))?;

    let event = SessionRegistry::with_session(&req.session_id, |s| s.push_inbound_pcm(&samples))?;

    let turn_started = matches!(event, VadEvent::EndOfUtterance);
    if turn_started {
        // Acquire processing lock to prevent concurrent turns.
        let acquired = SessionRegistry::try_acquire_processing(&req.session_id).unwrap_or(false);
        if acquired {
            let session_id = req.session_id.clone();
            tokio::spawn(async move {
                if let Err(err) = brain::run_turn(&session_id).await {
                    tracing::warn!("{LOG_PREFIX} brain turn failed session={session_id} err={err}");
                    let _ = SessionRegistry::with_session(&session_id, |s| {
                        s.last_error = Some(err.clone());
                        s.state = super::types::SessionState::Listening;
                    });
                } else {
                    let _ = SessionRegistry::with_session(&session_id, |s| {
                        s.last_error = None;
                    });
                }
                SessionRegistry::release_processing(&session_id);
            });
        } else {
            tracing::debug!(
                "{LOG_PREFIX} skipping turn — already processing session={}",
                req.session_id
            );
        }
    }

    RpcOutcome::new(
        json!({
            "ok": true,
            "turn_started": turn_started,
        }),
        vec![],
    )
    .into_cli_compatible_json()
}

pub async fn handle_poll_response(params: Map<String, Value>) -> Result<Value, String> {
    let req: PollResponseRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("{LOG_PREFIX} invalid poll_response params: {e}"))?;

    let (pcm_base64, transcript, reply_text, utterance_done) =
        SessionRegistry::with_session(&req.session_id, |s| {
            let (pcm, done) = s.poll_outbound();
            (pcm, s.last_transcript.clone(), s.last_reply.clone(), done)
        })?;

    RpcOutcome::new(
        json!({
            "ok": true,
            "pcm_base64": pcm_base64,
            "transcript": transcript,
            "reply_text": reply_text,
            "utterance_done": utterance_done,
        }),
        vec![],
    )
    .into_cli_compatible_json()
}

pub async fn handle_get_status(params: Map<String, Value>) -> Result<Value, String> {
    let req: GetStatusRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("{LOG_PREFIX} invalid get_status params: {e}"))?;

    let (state, turns, stt, tts, last_error) =
        SessionRegistry::with_session(&req.session_id, |s| {
            (
                s.state,
                s.turn_count,
                s.stt_provider.clone(),
                s.tts_provider.clone(),
                s.last_error.clone(),
            )
        })?;

    RpcOutcome::new(
        json!({
            "ok": true,
            "session_id": req.session_id,
            "state": state,
            "total_turns": turns,
            "stt_provider": stt,
            "tts_provider": tts,
            "last_error": last_error,
        }),
        vec![],
    )
    .into_cli_compatible_json()
}

pub async fn handle_interrupt(params: Map<String, Value>) -> Result<Value, String> {
    let req: InterruptRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("{LOG_PREFIX} invalid interrupt params: {e}"))?;

    let (was_speaking, discarded) = SessionRegistry::with_session(&req.session_id, |s| {
        let was = s.state == SessionState::Speaking;
        let d = s.interrupt();
        (was, d)
    })?;

    info!(
        "{LOG_PREFIX} interrupt session={} was_speaking={was_speaking} discarded={discarded}",
        req.session_id
    );

    RpcOutcome::new(
        json!({
            "ok": true,
            "was_speaking": was_speaking,
            "discarded_samples": discarded,
        }),
        vec![],
    )
    .into_cli_compatible_json()
}

pub async fn handle_stop_session(params: Map<String, Value>) -> Result<Value, String> {
    let req: StopSessionRequest = serde_json::from_value(Value::Object(params))
        .map_err(|e| format!("{LOG_PREFIX} invalid stop_session params: {e}"))?;

    let session = SessionRegistry::stop(&req.session_id)?;
    info!(
        "{LOG_PREFIX} stop_session id={} turns={} listened={:.2}s spoken={:.2}s",
        session.session_id,
        session.turn_count,
        session.listened_seconds(),
        session.spoken_seconds()
    );

    RpcOutcome::new(
        json!({
            "ok": true,
            "session_id": session.session_id,
            "total_turns": session.turn_count,
            "listened_seconds": session.listened_seconds(),
            "spoken_seconds": session.spoken_seconds(),
        }),
        vec![],
    )
    .into_cli_compatible_json()
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

    fn b64_pcm(samples: &[i16]) -> String {
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        B64.encode(bytes)
    }

    #[tokio::test]
    async fn start_then_stop_round_trip() {
        let mut params = Map::new();
        params.insert("stt_provider".into(), json!("whisper"));
        params.insert("tts_provider".into(), json!("piper"));
        let out = handle_start_session(params).await.unwrap();
        assert_eq!(out.get("ok"), Some(&json!(true)));
        let sid = out.get("session_id").unwrap().as_str().unwrap().to_string();

        let mut stop = Map::new();
        stop.insert("session_id".into(), json!(sid));
        let out = handle_stop_session(stop).await.unwrap();
        assert_eq!(out.get("ok"), Some(&json!(true)));
        assert_eq!(out.get("total_turns"), Some(&json!(0)));
    }

    #[tokio::test]
    async fn push_audio_accepts_empty() {
        let mut start = Map::new();
        start.insert("stt_provider".into(), json!("whisper"));
        start.insert("tts_provider".into(), json!("piper"));
        let out = handle_start_session(start).await.unwrap();
        let sid = out.get("session_id").unwrap().as_str().unwrap().to_string();

        let mut push = Map::new();
        push.insert("session_id".into(), json!(sid.clone()));
        push.insert("pcm_base64".into(), json!(""));
        let out = handle_push_audio(push).await.unwrap();
        assert_eq!(out.get("ok"), Some(&json!(true)));
        assert_eq!(out.get("turn_started"), Some(&json!(false)));

        let mut stop = Map::new();
        stop.insert("session_id".into(), json!(sid));
        handle_stop_session(stop).await.unwrap();
    }

    #[tokio::test]
    async fn push_audio_accepts_silence() {
        let mut start = Map::new();
        start.insert("stt_provider".into(), json!("whisper"));
        start.insert("tts_provider".into(), json!("piper"));
        let out = handle_start_session(start).await.unwrap();
        let sid = out.get("session_id").unwrap().as_str().unwrap().to_string();

        let silence = vec![0i16; 1600];
        let mut push = Map::new();
        push.insert("session_id".into(), json!(sid.clone()));
        push.insert("pcm_base64".into(), json!(b64_pcm(&silence)));
        let out = handle_push_audio(push).await.unwrap();
        assert_eq!(out.get("ok"), Some(&json!(true)));

        let mut stop = Map::new();
        stop.insert("session_id".into(), json!(sid));
        handle_stop_session(stop).await.unwrap();
    }

    #[tokio::test]
    async fn get_status_returns_session_info() {
        let mut start = Map::new();
        start.insert("stt_provider".into(), json!("whisper"));
        start.insert("tts_provider".into(), json!("piper"));
        let out = handle_start_session(start).await.unwrap();
        let sid = out.get("session_id").unwrap().as_str().unwrap().to_string();

        let mut status = Map::new();
        status.insert("session_id".into(), json!(sid.clone()));
        let out = handle_get_status(status).await.unwrap();
        assert_eq!(out.get("ok"), Some(&json!(true)));
        assert_eq!(out.get("state"), Some(&json!("listening")));
        assert_eq!(out.get("stt_provider"), Some(&json!("whisper")));

        let mut stop = Map::new();
        stop.insert("session_id".into(), json!(sid));
        handle_stop_session(stop).await.unwrap();
    }

    #[test]
    fn decode_pcm16le_b64_handles_empty() {
        assert!(decode_pcm16le_b64("").unwrap().is_empty());
    }

    #[test]
    fn decode_pcm16le_b64_rejects_odd_length() {
        let odd = B64.encode([0u8, 1, 2]);
        assert!(decode_pcm16le_b64(&odd).is_err());
    }

    #[test]
    fn decode_pcm16le_b64_round_trips() {
        let samples = vec![100i16, -200, 32767, -32768];
        let encoded = b64_pcm(&samples);
        let decoded = decode_pcm16le_b64(&encoded).unwrap();
        assert_eq!(decoded, samples);
    }
}
