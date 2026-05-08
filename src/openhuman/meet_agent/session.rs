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
    /// Buffer of post-wake-word caption text waiting for the brain
    /// turn to fire. Populated by `note_caption` once a wake word is
    /// observed; flushed by `take_pending_prompt`.
    pending_prompt: String,
    /// True between "wake word matched" and "brain turn dispatched".
    /// Used to avoid firing a second turn on every subsequent caption
    /// line while the prompt is still being assembled.
    pub wake_active: bool,
    /// `ts_ms` of the last caption that contributed to
    /// `pending_prompt`. The brain uses this + the current time to
    /// decide whether the user has stopped talking.
    pub last_caption_ts_ms: u64,
    /// Page-side `Date.now()` of the most recent caption that fired
    /// the wake word. Suppresses re-firing while Meet's caption
    /// region keeps the same utterance visible (Meet shows captions
    /// for ~5–8 s after speaking ends, and our dedupe is per-exact-
    /// text — a single character growth re-queues the line). Without
    /// this gate the brain spam-fires on every caption growth.
    wake_cooldown_until_ts_ms: u64,
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
            pending_prompt: String::new(),
            wake_active: false,
            last_caption_ts_ms: 0,
            wake_cooldown_until_ts_ms: 0,
        }
    }

    /// Caption-driven listen path. Returns `true` when this caption
    /// just tripped the wake word (caller should kick a turn).
    ///
    /// The wake-word match is intentionally permissive: case-folded
    /// substring on `"hey openhuman"` (and `"hey open human"` to
    /// tolerate Meet's STT splitting the brand name). Any text after
    /// the match in the same caption is treated as the start of the
    /// prompt; subsequent captions append until `take_pending_prompt`
    /// drains.
    pub fn note_caption(&mut self, speaker: &str, text: &str, ts_ms: u64) -> bool {
        if text.trim().is_empty() {
            return false;
        }
        self.last_caption_ts_ms = ts_ms;
        // Already collecting after a previous wake word: just append
        // the new caption. No second fire — the brain is already
        // scheduled and will drain the prompt in ~1.5 s. Without this
        // gate, a slowly-growing caption fires the wake word on
        // every dedupe-then-grow cycle.
        if self.wake_active {
            if !self.pending_prompt.is_empty() {
                self.pending_prompt.push(' ');
            }
            self.pending_prompt.push_str(text.trim());
            return false;
        }
        // In cooldown after a recent turn — Meet keeps the same
        // utterance visible for several seconds, so without this
        // gate the brain re-fires on every caption growth. Continue
        // recording the caption to the transcript log (below) but
        // skip wake-word matching.
        if ts_ms != 0 && ts_ms < self.wake_cooldown_until_ts_ms {
            self.record_event(
                SessionEventKind::Heard,
                if speaker.is_empty() {
                    text.to_string()
                } else {
                    format!("{speaker}: {text}")
                },
            );
            return false;
        }
        // Normalize before matching: Meet's STT punctuates the wake
        // phrase ("hey, openhuman"), capitalizes mid-sentence, and
        // sometimes collapses the brand to two words. Folding to
        // lowercase + replacing punctuation with spaces + collapsing
        // whitespace gives us a single canonical form to substring
        // against. The tail (the dictation after the wake phrase) is
        // returned in normalized form too — that's fine for the LLM
        // and the transcript log; the user's punctuation isn't load-
        // bearing for note-taking.
        let normalized = normalize_for_wake(text);
        let wake_idx = normalized
            .find("hey openhuman")
            .or_else(|| normalized.find("hey open human"));
        if let Some(idx) = wake_idx {
            let after = if normalized[idx..].starts_with("hey openhuman") {
                idx + "hey openhuman".len()
            } else {
                idx + "hey open human".len()
            };
            let tail = normalized.get(after..).unwrap_or("").trim().to_string();
            self.pending_prompt = tail;
            self.wake_active = true;
            self.record_event(
                SessionEventKind::Note,
                format!("wake word from speaker={speaker}"),
            );
            return true;
        }
        // Outside a wake context, just record the line for the
        // transcript log. Useful for debugging "why didn't the agent
        // respond". (The wake-active branch is handled by the
        // early-return above.)
        self.record_event(
            SessionEventKind::Heard,
            if speaker.is_empty() {
                text.to_string()
            } else {
                format!("{speaker}: {text}")
            },
        );
        false
    }

    /// Drain the assembled wake-word prompt and clear the active
    /// flag. The brain calls this once it's ready to dispatch the
    /// turn so subsequent captions start a fresh wake-word cycle.
    ///
    /// Sets a cooldown window keyed off `last_caption_ts_ms` so any
    /// subsequent caption push for the same lingering utterance
    /// doesn't re-fire the wake-word state machine. 8s is a comfortable
    /// upper bound on how long Meet keeps a finalised caption visible.
    pub fn take_pending_prompt(&mut self) -> Option<String> {
        if !self.wake_active {
            return None;
        }
        self.wake_active = false;
        // 8s grace beyond the most recent caption's page timestamp.
        // `last_caption_ts_ms` is whatever Date.now() was page-side
        // when the line landed — same clock as future caption pushes.
        const COOLDOWN_MS: u64 = 8_000;
        self.wake_cooldown_until_ts_ms = self.last_caption_ts_ms.saturating_add(COOLDOWN_MS);
        let prompt = std::mem::take(&mut self.pending_prompt);
        let trimmed = prompt.trim().to_string();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed)
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

/// Lowercase + drop punctuation + collapse whitespace, so the wake
/// phrase matches regardless of how Meet's STT punctuated or cased
/// it ("Hey, OpenHuman", "hey open-human", etc).
fn normalize_for_wake(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut prev_space = true;
    for c in text.chars() {
        let lc = c.to_ascii_lowercase();
        if lc.is_ascii_alphanumeric() {
            out.push(lc);
            prev_space = false;
        } else if !prev_space {
            out.push(' ');
            prev_space = true;
        }
    }
    out.trim_end().to_string()
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
            log::info!("[meet-agent] replacing existing session request_id={request_id}");
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
    fn note_caption_handles_punctuated_wake() {
        let mut s = MeetAgentSession::new("p".into(), 16_000);
        // Meet often inserts a comma after "hey".
        let fired = s.note_caption("Alice", "Hey, OpenHuman remember the launch", 1);
        assert!(fired, "punctuated wake phrase should still fire");
        let prompt = s.take_pending_prompt().expect("prompt drained");
        assert_eq!(prompt, "remember the launch");
    }

    #[test]
    fn note_caption_handles_split_brand() {
        let mut s = MeetAgentSession::new("p".into(), 16_000);
        let fired = s.note_caption("Alice", "hey open-human, send the report", 1);
        assert!(fired);
        let prompt = s.take_pending_prompt().expect("prompt drained");
        assert_eq!(prompt, "send the report");
    }

    #[test]
    fn note_caption_does_not_double_fire_on_growing_caption() {
        let mut s = MeetAgentSession::new("p".into(), 16_000);
        let first = s.note_caption("Alice", "hey openhuman take notes", 1);
        assert!(first);
        let second = s.note_caption("Alice", "hey openhuman take notes about the launch", 2);
        assert!(!second, "second caption while wake_active must not refire");
        let prompt = s.take_pending_prompt().expect("prompt drained");
        // First wake stripped "hey openhuman"; the continuation
        // appended the WHOLE growing caption (still containing "hey
        // openhuman" because we don't re-strip), separated by a
        // space. That's fine — the LLM ignores the prefix and the
        // transcript log still records the verbatim dictation.
        assert!(
            prompt.contains("take notes about the launch"),
            "got prompt: {prompt}"
        );
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
