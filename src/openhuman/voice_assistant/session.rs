//! Per-session state for the voice assistant.
//!
//! Each session holds inbound PCM, outbound PCM, VAD state, conversation
//! history, and provider configuration. Sessions are keyed by a UUID and
//! stored in a process-wide registry.

use std::collections::{HashMap, VecDeque};
use std::sync::{Mutex, OnceLock};

use base64::Engine as _;
use tracing::{debug, warn};

use crate::openhuman::meet_agent::ops::{Vad, VadEvent};
use crate::openhuman::util::now_epoch;

use super::types::SessionState;

const LOG_PREFIX: &str = "[voice-assistant-session]";

/// Maximum inbound PCM buffer: 30 seconds @ 16 kHz.
const MAX_INBOUND_SAMPLES: usize = 16_000 * 30;

/// Maximum outbound PCM buffer: 30 seconds @ 16 kHz.
const MAX_OUTBOUND_SAMPLES: usize = 16_000 * 30;

/// Maximum conversation history entries.
const MAX_HISTORY: usize = 50;

/// Maximum concurrent sessions before LRU eviction.
const MAX_SESSIONS: usize = 32;

/// Session idle timeout: 10 minutes without activity.
const SESSION_IDLE_TIMEOUT_SECS: u64 = 600;

/// A single voice assistant session.
pub struct VoiceAssistantSession {
    pub session_id: String,
    pub stt_provider: String,
    pub tts_provider: String,
    pub language: Option<String>,
    pub state: SessionState,
    pub turn_count: u32,
    pub inbound_samples: usize,
    pub outbound_samples: usize,

    /// Inbound PCM buffer (user speech, pre-STT).
    inbound_pcm: Vec<i16>,
    /// Outbound PCM buffer (assistant speech, post-TTS).
    outbound_pcm: Vec<i16>,
    /// VAD state machine.
    vad: Vad,
    /// Last transcript from STT.
    pub last_transcript: String,
    /// Last reply from LLM.
    pub last_reply: String,
    /// Conversation history for LLM context.
    pub history: VecDeque<ConversationTurn>,
    /// Last error from brain turn (if any). Cleared on next successful turn.
    pub last_error: Option<String>,
    /// Epoch seconds of last activity (push_audio, poll, etc.).
    pub last_activity: u64,
    /// True while a brain turn is in progress (prevents concurrent turns).
    pub processing_lock: bool,
    /// Detected language from last STT pass (auto-detection).
    pub detected_language: Option<String>,
    /// Detected emotion/sentiment from last utterance.
    pub detected_emotion: Option<String>,
    /// Whether barge-in (interruption) is enabled.
    pub barge_in_enabled: bool,
    /// Count of interruptions in this session.
    pub interrupt_count: u32,
    /// Wake word phrase (if wake-word mode is active).
    pub wake_word: Option<String>,
    /// Streaming partial transcript (updated during chunked STT).
    pub partial_transcript: String,
}

/// A single conversation turn (user said X, assistant replied Y).
#[derive(Debug, Clone)]
pub struct ConversationTurn {
    pub user_text: String,
    pub assistant_text: String,
}

impl VoiceAssistantSession {
    pub fn new(
        session_id: String,
        stt_provider: String,
        tts_provider: String,
        language: Option<String>,
    ) -> Self {
        Self {
            session_id,
            stt_provider,
            tts_provider,
            language,
            state: SessionState::Listening,
            turn_count: 0,
            inbound_samples: 0,
            outbound_samples: 0,
            inbound_pcm: Vec::with_capacity(16_000), // 1s initial
            outbound_pcm: Vec::new(),
            vad: Vad::new(),
            last_transcript: String::new(),
            last_reply: String::new(),
            history: VecDeque::new(),
            last_error: None,
            last_activity: now_epoch(),
            processing_lock: false,
            detected_language: None,
            detected_emotion: None,
            barge_in_enabled: true,
            interrupt_count: 0,
            wake_word: None,
            partial_transcript: String::new(),
        }
    }

    /// Push inbound PCM samples and run VAD. Returns the VAD event.
    /// If barge-in is enabled and session is Speaking, detects speech and interrupts.
    pub fn push_inbound_pcm(&mut self, samples: &[i16]) -> VadEvent {
        self.last_activity = now_epoch();
        if samples.is_empty() {
            return VadEvent::Idle;
        }

        // Barge-in: if we're speaking and detect user speech, interrupt immediately.
        if self.barge_in_enabled && self.state == SessionState::Speaking {
            let energy: f64 = samples
                .iter()
                .map(|&s| (s as f64) * (s as f64))
                .sum::<f64>()
                / samples.len() as f64;
            // Threshold: ~-40dBFS for 16-bit audio (RMS ~100 = energy ~10000)
            if energy > 10_000.0 {
                debug!(
                    "{LOG_PREFIX} barge-in detected (energy={energy:.0}), interrupting session={}",
                    self.session_id
                );
                self.interrupt();
            }
        }

        // Enforce max buffer size.
        let remaining = MAX_INBOUND_SAMPLES.saturating_sub(self.inbound_pcm.len());
        let to_push = samples.len().min(remaining);
        self.inbound_pcm.extend_from_slice(&samples[..to_push]);
        self.inbound_samples += to_push;

        self.vad.feed(samples)
    }

    /// Interrupt the current TTS playback (barge-in).
    /// Clears outbound buffer and transitions back to Listening.
    pub fn interrupt(&mut self) -> usize {
        let discarded = self.outbound_pcm.len();
        self.outbound_pcm.clear();
        self.state = SessionState::Listening;
        self.interrupt_count += 1;
        debug!(
            "{LOG_PREFIX} interrupted session={} discarded={discarded} samples",
            self.session_id
        );
        discarded
    }

    /// Drain the inbound PCM buffer (called by brain after VAD fires).
    pub fn drain_inbound_pcm(&mut self) -> Vec<i16> {
        std::mem::take(&mut self.inbound_pcm)
    }

    /// Enqueue outbound PCM (TTS output for the user to hear).
    pub fn enqueue_outbound_pcm(&mut self, samples: &[i16]) {
        let remaining = MAX_OUTBOUND_SAMPLES.saturating_sub(self.outbound_pcm.len());
        let to_push = samples.len().min(remaining);
        self.outbound_pcm.extend_from_slice(&samples[..to_push]);
        self.outbound_samples += to_push;
    }

    /// Poll outbound PCM. Returns (base64_pcm, utterance_done).
    pub fn poll_outbound(&mut self) -> (String, bool) {
        self.last_activity = now_epoch();
        if self.outbound_pcm.is_empty() {
            return (String::new(), self.state != SessionState::Speaking);
        }
        let samples = std::mem::take(&mut self.outbound_pcm);
        let bytes: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
        // Mark utterance done when buffer is fully drained and not processing.
        let done = self.state != SessionState::Processing;
        (b64, done)
    }

    /// Record a completed turn.
    pub fn record_turn(&mut self, user_text: &str, assistant_text: &str) {
        self.last_transcript = user_text.to_string();
        self.last_reply = assistant_text.to_string();
        self.turn_count += 1;
        self.history.push_back(ConversationTurn {
            user_text: user_text.to_string(),
            assistant_text: assistant_text.to_string(),
        });
        if self.history.len() > MAX_HISTORY {
            self.history.pop_front();
        }
    }

    /// Total seconds of inbound audio processed.
    pub fn listened_seconds(&self) -> f64 {
        self.inbound_samples as f64 / 16_000.0
    }

    /// Total seconds of outbound audio synthesized.
    pub fn spoken_seconds(&self) -> f64 {
        self.outbound_samples as f64 / 16_000.0
    }
}

// ---------------------------------------------------------------------------
// Process-wide session registry
// ---------------------------------------------------------------------------

static REGISTRY: OnceLock<Mutex<HashMap<String, VoiceAssistantSession>>> = OnceLock::new();

fn registry_map() -> &'static Mutex<HashMap<String, VoiceAssistantSession>> {
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Public registry handle for RPC handlers.
pub struct SessionRegistry;

impl SessionRegistry {
    /// Start a new session. Evicts idle sessions if at capacity.
    pub fn start(
        session_id: &str,
        stt_provider: &str,
        tts_provider: &str,
        language: Option<&str>,
    ) -> Result<(), String> {
        // Validate session_id (same rules as meet_agent::ops::sanitize_request_id)
        let trimmed = session_id.trim();
        if trimmed.is_empty() {
            return Err("session_id must not be empty".into());
        }
        if trimmed.len() > 64 {
            return Err("session_id exceeds 64 characters".into());
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err("session_id contains forbidden characters".into());
        }

        let mut map = registry_map()
            .lock()
            .map_err(|e| format!("{LOG_PREFIX} lock poisoned: {e}"))?;
        if map.contains_key(session_id) {
            // Idempotent restart: close old, open new.
            debug!("{LOG_PREFIX} restarting existing session={session_id}");
            super::brain::evict_nc_state(session_id);
            map.remove(session_id);
        }
        // Evict expired sessions and enforce max capacity.
        evict_idle_sessions(&mut map);
        if map.len() >= MAX_SESSIONS {
            // Evict the least recently active session.
            if let Some(lru_id) = map
                .values()
                .min_by_key(|s| s.last_activity)
                .map(|s| s.session_id.clone())
            {
                warn!("{LOG_PREFIX} evicting LRU session={lru_id} (at capacity {MAX_SESSIONS})");
                super::brain::evict_nc_state(&lru_id);
                map.remove(&lru_id);
            }
        }
        let session = VoiceAssistantSession::new(
            session_id.to_string(),
            stt_provider.to_string(),
            tts_provider.to_string(),
            language.map(str::to_string),
        );
        map.insert(session_id.to_string(), session);
        debug!("{LOG_PREFIX} started session={session_id} stt={stt_provider} tts={tts_provider}");
        Ok(())
    }

    /// Execute a closure with mutable access to a session.
    pub fn with_session<F, R>(session_id: &str, f: F) -> Result<R, String>
    where
        F: FnOnce(&mut VoiceAssistantSession) -> R,
    {
        let mut map = registry_map()
            .lock()
            .map_err(|e| format!("{LOG_PREFIX} lock poisoned: {e}"))?;
        let session = map
            .get_mut(session_id)
            .ok_or_else(|| format!("{LOG_PREFIX} session not found: {session_id}"))?;
        Ok(f(session))
    }

    /// Stop and remove a session. Returns the final session state.
    pub fn stop(session_id: &str) -> Result<VoiceAssistantSession, String> {
        super::brain::evict_nc_state(session_id);
        let mut map = registry_map()
            .lock()
            .map_err(|e| format!("{LOG_PREFIX} lock poisoned: {e}"))?;
        map.remove(session_id)
            .ok_or_else(|| format!("{LOG_PREFIX} session not found: {session_id}"))
    }

    /// Try to acquire the processing lock for a session.
    /// Returns false if a turn is already in progress.
    pub fn try_acquire_processing(session_id: &str) -> Result<bool, String> {
        Self::with_session(session_id, |s| {
            if s.processing_lock {
                false
            } else {
                s.processing_lock = true;
                true
            }
        })
    }

    /// Release the processing lock for a session.
    pub fn release_processing(session_id: &str) {
        let _ = Self::with_session(session_id, |s| {
            s.processing_lock = false;
        });
    }
}

/// Remove sessions that have been idle longer than the timeout.
fn evict_idle_sessions(map: &mut HashMap<String, VoiceAssistantSession>) {
    let now = now_epoch();
    let expired: Vec<String> = map
        .iter()
        .filter(|(_, s)| now.saturating_sub(s.last_activity) > SESSION_IDLE_TIMEOUT_SECS)
        .map(|(id, _)| id.clone())
        .collect();
    for id in &expired {
        debug!("{LOG_PREFIX} evicting idle session={id}");
        super::brain::evict_nc_state(id);
        map.remove(id);
    }
    if !expired.is_empty() {
        debug!("{LOG_PREFIX} evicted {} idle sessions", expired.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_lifecycle() {
        let id = format!("test-{}", uuid::Uuid::new_v4());
        SessionRegistry::start(&id, "whisper", "piper", Some("en")).unwrap();

        SessionRegistry::with_session(&id, |s| {
            assert_eq!(s.state, SessionState::Listening);
            assert_eq!(s.turn_count, 0);
        })
        .unwrap();

        let stopped = SessionRegistry::stop(&id).unwrap();
        assert_eq!(stopped.session_id, id);
    }

    #[test]
    fn push_pcm_triggers_vad() {
        let id = format!("test-vad-{}", uuid::Uuid::new_v4());
        SessionRegistry::start(&id, "whisper", "piper", None).unwrap();

        // Push silence — should get Idle or Silence.
        let event = SessionRegistry::with_session(&id, |s| {
            let silence = vec![0i16; 1600]; // 100ms silence
            s.push_inbound_pcm(&silence)
        })
        .unwrap();
        assert!(matches!(event, VadEvent::Idle | VadEvent::Silence));

        SessionRegistry::stop(&id).unwrap();
    }

    #[test]
    fn outbound_poll_returns_base64() {
        let id = format!("test-poll-{}", uuid::Uuid::new_v4());
        SessionRegistry::start(&id, "whisper", "piper", None).unwrap();

        SessionRegistry::with_session(&id, |s| {
            s.enqueue_outbound_pcm(&[100i16, 200, 300]);
            let (b64, _done) = s.poll_outbound();
            assert!(!b64.is_empty());
            // Second poll should be empty.
            let (b64_2, _) = s.poll_outbound();
            assert!(b64_2.is_empty());
        })
        .unwrap();

        SessionRegistry::stop(&id).unwrap();
    }

    #[test]
    fn record_turn_increments_counter() {
        let id = format!("test-turn-{}", uuid::Uuid::new_v4());
        SessionRegistry::start(&id, "whisper", "piper", None).unwrap();

        SessionRegistry::with_session(&id, |s| {
            s.record_turn("hello", "hi there");
            assert_eq!(s.turn_count, 1);
            assert_eq!(s.last_transcript, "hello");
            assert_eq!(s.last_reply, "hi there");
        })
        .unwrap();

        SessionRegistry::stop(&id).unwrap();
    }
}
