//! Per-call session state for the meet-agent loop.
//!
//! A `MeetAgentSession` holds the state that has to live for the
//! duration of a Google Meet call: the inbound PCM ring buffer (kept
//! short — VAD chops it into utterances), the outbound TTS queue (PCM
//! the brain has produced and the shell hasn't drained yet), VAD state,
//! transcript log, and counters for the smoke test.
//!
//! Sessions are keyed by `request_id` (the same UUID `meet/` mints) and
//! live in a process-wide `OnceLock<Mutex<HashMap<...>>>`. The locking
//! pattern matches `meet_call::MeetCallState` on the shell side.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

use super::ops::{self, Vad, VadEvent};
use super::types::{SessionEvent, SessionEventKind};

/// Cap on the inbound buffer so a runaway shell push (e.g. shell never
/// stops, brain never drains) can't grow memory unboundedly. 30s @ 16kHz
/// mono = 960 KB per session — generous for any reasonable utterance.
const MAX_INBOUND_SAMPLES: usize = 30 * 16_000;
/// Same idea for outbound: cap synthesized backlog at 30s. Brain trims
/// older audio if the shell hasn't polled fast enough.
const MAX_OUTBOUND_SAMPLES: usize = 30 * 16_000;
/// Keep the most recent N session events. Bounded so a noisy call
/// can't grow the log forever.
const MAX_EVENTS: usize = 256;

#[derive(Debug)]
pub struct MeetAgentSession {
    pub request_id: String,
    pub sample_rate_hz: u32,
    /// Wall-clock start. Used by the smoke-test response and to stamp
    /// session events.
    pub started_at: Instant,
    /// PCM samples awaiting brain processing. Drained per utterance.
    inbound: Vec<i16>,
    /// PCM samples the brain has synthesized but the shell hasn't
    /// pulled yet. Front-of-vec is "next bytes the shell will consume".
    outbound: Vec<i16>,
    /// True when the *current* outbound batch represents a complete
    /// utterance — the shell uses this to flush + drop back to silence.
    outbound_done: bool,
    vad: Vad,
    events: Vec<SessionEvent>,
    /// Total samples ever pushed in. Counter, not a buffer length —
    /// the inbound vec is drained per utterance, so we track separately
    /// for the smoke-test seconds-listened metric.
    total_inbound_samples: u64,
    total_outbound_samples: u64,
    pub turn_count: u32,
}

impl MeetAgentSession {
    pub fn new(request_id: String, sample_rate_hz: u32) -> Self {
        Self {
            request_id,
            sample_rate_hz,
            started_at: Instant::now(),
            inbound: Vec::new(),
            outbound: Vec::new(),
            outbound_done: false,
            vad: Vad::new(),
            events: Vec::new(),
            total_inbound_samples: 0,
            total_outbound_samples: 0,
            turn_count: 0,
        }
    }

    /// Append PCM samples to the inbound buffer. Returns the VAD verdict
    /// for *this* batch — caller consults it to decide whether to fire
    /// a brain turn.
    pub fn push_inbound_pcm(&mut self, samples: &[i16]) -> VadEvent {
        self.total_inbound_samples += samples.len() as u64;
        self.inbound.extend_from_slice(samples);
        if self.inbound.len() > MAX_INBOUND_SAMPLES {
            // Drop oldest; the in-progress utterance is what matters.
            let drop = self.inbound.len() - MAX_INBOUND_SAMPLES;
            self.inbound.drain(..drop);
        }
        self.vad.feed(samples)
    }

    /// Take ownership of the accumulated utterance for STT. The session
    /// keeps the VAD state — the next push_inbound_pcm starts a fresh
    /// utterance.
    pub fn drain_inbound(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.inbound)
    }

    /// Brain hands synthesized PCM back to the session. `done` flips
    /// `outbound_done` so the next poll surfaces "utterance over".
    pub fn enqueue_outbound_pcm(&mut self, samples: &[i16], done: bool) {
        self.total_outbound_samples += samples.len() as u64;
        self.outbound.extend_from_slice(samples);
        if self.outbound.len() > MAX_OUTBOUND_SAMPLES {
            let drop = self.outbound.len() - MAX_OUTBOUND_SAMPLES;
            self.outbound.drain(..drop);
        }
        if done {
            self.outbound_done = true;
        }
    }

    /// Drain everything currently queued for the shell. Returns
    /// `(pcm_base64, utterance_done)`.
    pub fn poll_outbound(&mut self) -> (String, bool) {
        if self.outbound.is_empty() {
            let done = std::mem::take(&mut self.outbound_done);
            return (String::new(), done);
        }
        let bytes: Vec<u8> = self
            .outbound
            .drain(..)
            .flat_map(|s| s.to_le_bytes())
            .collect();
        let done = std::mem::take(&mut self.outbound_done);
        (B64.encode(bytes), done)
    }

    pub fn record_event(&mut self, kind: SessionEventKind, text: String) {
        let timestamp_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        self.events.push(SessionEvent {
            kind,
            text,
            timestamp_ms,
        });
        if self.events.len() > MAX_EVENTS {
            let drop = self.events.len() - MAX_EVENTS;
            self.events.drain(..drop);
        }
    }

    pub fn events(&self) -> &[SessionEvent] {
        &self.events
    }

    pub fn listened_seconds(&self) -> f32 {
        self.total_inbound_samples as f32 / self.sample_rate_hz as f32
    }

    pub fn spoken_seconds(&self) -> f32 {
        self.total_outbound_samples as f32 / self.sample_rate_hz as f32
    }
}

/// Process-wide session registry. Sessions are keyed by `request_id`.
#[derive(Default)]
pub struct MeetAgentSessionRegistry {
    inner: Mutex<HashMap<String, MeetAgentSession>>,
}

impl MeetAgentSessionRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn start(&self, request_id: &str, sample_rate_hz: u32) -> Result<(), String> {
        let request_id = ops::sanitize_request_id(request_id)?;
        let sample_rate_hz = ops::validate_sample_rate(sample_rate_hz)?;
        let mut guard = self.inner.lock().unwrap();
        if guard.contains_key(&request_id) {
            // Idempotent restart: replace the old session so a shell
            // crash + reconnect doesn't wedge the registry.
            log::info!(
                "[meet-agent] replacing existing session request_id={request_id}"
            );
        }
        guard.insert(
            request_id.clone(),
            MeetAgentSession::new(request_id, sample_rate_hz),
        );
        Ok(())
    }

    pub fn stop(&self, request_id: &str) -> Result<MeetAgentSession, String> {
        let request_id = ops::sanitize_request_id(request_id)?;
        let mut guard = self.inner.lock().unwrap();
        guard
            .remove(&request_id)
            .ok_or_else(|| format!("[meet-agent] no session for request_id={request_id}"))
    }

    /// Run a closure with mutable access to the named session. Returns
    /// `Err` when the session is unknown.
    pub fn with_session<R>(
        &self,
        request_id: &str,
        f: impl FnOnce(&mut MeetAgentSession) -> R,
    ) -> Result<R, String> {
        let request_id = ops::sanitize_request_id(request_id)?;
        let mut guard = self.inner.lock().unwrap();
        let session = guard
            .get_mut(&request_id)
            .ok_or_else(|| format!("[meet-agent] no session for request_id={request_id}"))?;
        Ok(f(session))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }
}

/// Process-wide singleton. Lazy-initialized so tests can use a fresh
/// registry where they want to.
pub static SESSION_REGISTRY: OnceLock<MeetAgentSessionRegistry> = OnceLock::new();

pub fn registry() -> &'static MeetAgentSessionRegistry {
    SESSION_REGISTRY.get_or_init(MeetAgentSessionRegistry::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_and_stop_round_trip() {
        let reg = MeetAgentSessionRegistry::new();
        reg.start("abc-123", 16_000).unwrap();
        assert_eq!(reg.len(), 1);
        let session = reg.stop("abc-123").unwrap();
        assert_eq!(session.request_id, "abc-123");
        assert_eq!(reg.len(), 0);
    }

    #[test]
    fn start_rejects_bad_inputs() {
        let reg = MeetAgentSessionRegistry::new();
        assert!(reg.start("", 16_000).is_err());
        assert!(reg.start("abc", 1_000).is_err());
    }

    #[test]
    fn stop_unknown_session_errors() {
        let reg = MeetAgentSessionRegistry::new();
        assert!(reg.stop("never-started").is_err());
    }

    #[test]
    fn push_inbound_accumulates_samples() {
        let reg = MeetAgentSessionRegistry::new();
        reg.start("s1", 16_000).unwrap();
        reg.with_session("s1", |s| {
            s.push_inbound_pcm(&vec![1000; 320]);
            s.push_inbound_pcm(&vec![1000; 320]);
            assert_eq!(s.inbound.len(), 640);
        })
        .unwrap();
    }

    #[test]
    fn poll_outbound_returns_done_flag_once() {
        let reg = MeetAgentSessionRegistry::new();
        reg.start("s2", 16_000).unwrap();
        reg.with_session("s2", |s| {
            s.enqueue_outbound_pcm(&vec![0; 100], true);
            let (b64, done) = s.poll_outbound();
            assert!(!b64.is_empty());
            assert!(done);
            // Second poll: no audio, no `done` (we already consumed it).
            let (b64, done) = s.poll_outbound();
            assert!(b64.is_empty());
            assert!(!done);
        })
        .unwrap();
    }

    #[test]
    fn listened_seconds_tracks_total_inbound() {
        let reg = MeetAgentSessionRegistry::new();
        reg.start("s3", 16_000).unwrap();
        reg.with_session("s3", |s| {
            s.push_inbound_pcm(&vec![0; 16_000]); // 1.0s
            s.push_inbound_pcm(&vec![0; 8_000]); //  0.5s
            assert!((s.listened_seconds() - 1.5).abs() < 1e-3);
        })
        .unwrap();
    }
}
