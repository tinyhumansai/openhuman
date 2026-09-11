//! Deterministic heuristic detectors for Style and Veto candidates.
//!
//! Three detectors feed [`crate::openhuman::agent::learning::candidate::global()`]:
//!
//! - **`LengthRatioDetector`** — emits `Style/verbosity` when the rolling
//!   ratio of user-to-agent message lengths shifts significantly over ≥ 30 turns.
//! - **`EditWindowDetector`** — emits `Style/*` candidates when the user sends
//!   a correction within 30 s of the previous agent reply.
//! - **`CorrectionRepeatDetector`** — promotes repeated (≥ 3×) correction cues to
//!   `Veto/*` candidates.
//!
//! All three detectors advance via a single call to [`record_turn`], which is
//! intended to be called from [`crate::openhuman::agent::learning::ReflectionHook`] or a
//! sibling post-turn hook.
//!
//! ## State lifetime
//!
//! State is per-session and bounded to 100 turns per session. A global `RwLock`-
//! guarded map keeps per-session [`RollingState`]. Sessions that exceed
//! `MAX_SESSION_TURNS` have their oldest turn evicted (FIFO) to stay bounded.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::OnceLock;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::RwLock;

use crate::openhuman::agent::learning::candidate::{
    self, CueFamily, EvidenceRef, FacetClass, LearningCandidate,
};

// ── Constants ────────────────────────────────────────────────────────────────

/// Per-session turn capacity before FIFO eviction.
const MAX_SESSION_TURNS: usize = 100;

/// Minimum number of turns before the length-ratio detector fires.
const LENGTH_RATIO_MIN_TURNS: usize = 30;

/// Number of turns in each of the two comparison windows.
const LENGTH_RATIO_WINDOW: usize = 15;

/// User-to-agent message length ratio below which we classify as "compressed".
const RATIO_COMPRESSED: f64 = 0.3;

/// User-to-agent message length ratio above which we classify as "balanced".
const RATIO_BALANCED: f64 = 0.8;

/// Ratio of the "earlier" window required to cross before we treat the shift as meaningful.
const RATIO_EARLIER_THRESHOLD: f64 = 0.5;

/// Edit-window threshold: correction sent within 30 s of agent reply is "quick".
const EDIT_WINDOW_SECS: f64 = 30.0;

/// Cooldown for EditWindowDetector per (key, value): 5 minutes.
const EDIT_COOLDOWN_SECS: f64 = 300.0;

/// Number of repeated corrections before promoting to Veto.
const VETO_PROMOTION_THRESHOLD: usize = 3;

// ── Per-turn state ────────────────────────────────────────────────────────────

/// A single turn entry in the rolling window.
#[derive(Clone, Debug)]
pub struct TurnEntry {
    pub turn_id: String,
    pub user_msg_len: usize,
    pub agent_msg_len: usize,
    /// Wall-clock seconds (epoch) when the user message arrived.
    pub user_timestamp: f64,
    /// Wall-clock seconds (epoch) when the agent finished replying.
    pub agent_timestamp: f64,
}

/// Per-session state for all three detectors.
#[derive(Default)]
pub struct RollingState {
    pub turns: VecDeque<TurnEntry>,
    /// Last emission per (key, value) to enforce cooldown / dedupe for length-ratio.
    pub length_ratio_emitted: HashSet<(String, String)>,
    /// Per (key, value) last emission timestamp for edit-window cooldown.
    pub edit_cooldown: HashMap<(String, String), f64>,
    /// Per correction-cue counter across the session.
    pub correction_counts: HashMap<String, usize>,
    /// Cues that have already been promoted to Veto (prevent double-emit).
    pub veto_promoted: HashSet<String>,
}

// ── Global state map ─────────────────────────────────────────────────────────

static SESSION_STATE: OnceLock<RwLock<HashMap<String, RollingState>>> = OnceLock::new();

fn session_state() -> &'static RwLock<HashMap<String, RollingState>> {
    SESSION_STATE.get_or_init(|| RwLock::new(HashMap::new()))
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Advance all three detectors for a completed turn and push any emitted
/// candidates to [`candidate::global()`].
///
/// # Arguments
///
/// * `session_id` — stable session identifier used as the state map key.
/// * `turn_id` — stable identifier for the current turn (e.g. an episodic id
///   encoded as a string).
/// * `episodic_id` — numeric episodic log id used to build `EvidenceRef`.
/// * `user_message` — user's message text for edit-window scanning.
/// * `user_msg_len` — byte length of the user message.
/// * `agent_msg_len` — byte length of the assistant response.
/// * `user_timestamp` — epoch seconds when the user message arrived.
/// * `agent_timestamp` — epoch seconds when the agent replied.
/// * `prev_agent_at` — epoch seconds of the **previous** agent reply, used by
///   the edit-window detector to check whether the user responded within
///   `EDIT_WINDOW_SECS`. Pass `None` on the first turn of a session.
#[allow(clippy::too_many_arguments)]
pub fn record_turn(
    session_id: &str,
    turn_id: &str,
    episodic_id: i64,
    user_message: &str,
    user_msg_len: usize,
    agent_msg_len: usize,
    user_timestamp: f64,
    agent_timestamp: f64,
    prev_agent_at: Option<f64>,
) {
    let entry = TurnEntry {
        turn_id: turn_id.to_string(),
        user_msg_len,
        agent_msg_len,
        user_timestamp,
        agent_timestamp,
    };

    let mut candidates: Vec<LearningCandidate> = Vec::new();

    {
        let mut map = session_state().write();
        let state = map.entry(session_id.to_string()).or_default();

        // FIFO eviction.
        if state.turns.len() >= MAX_SESSION_TURNS {
            state.turns.pop_front();
        }
        state.turns.push_back(entry.clone());

        // ── A. Length-ratio detector ──────────────────────────────────────
        if state.turns.len() >= LENGTH_RATIO_MIN_TURNS {
            let turns_slice: Vec<&TurnEntry> = state.turns.iter().collect();
            let n = turns_slice.len();
            let recent: &[&TurnEntry] = &turns_slice[n - LENGTH_RATIO_WINDOW..];
            let earlier: &[&TurnEntry] = &turns_slice
                [n - LENGTH_RATIO_MIN_TURNS..n - LENGTH_RATIO_MIN_TURNS + LENGTH_RATIO_WINDOW];

            let recent_ratio = mean_ratio(recent);
            let earlier_ratio = mean_ratio(earlier);

            // Compressed: user messages shrunk relative to earlier.
            if recent_ratio < RATIO_COMPRESSED
                && earlier_ratio > RATIO_EARLIER_THRESHOLD
                && !state
                    .length_ratio_emitted
                    .contains(&("verbosity".to_string(), "compressed".to_string()))
            {
                let first_id = state.turns.front().map(|t| t.turn_id.clone());
                let last_id = state.turns.back().map(|t| t.turn_id.clone());
                let from_id = first_id
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(episodic_id);
                let to_id = last_id
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(episodic_id);

                tracing::debug!(
                    "[learning::extract::heuristics] length_ratio compressed session={} \
                     recent_ratio={:.2} earlier_ratio={:.2}",
                    session_id,
                    recent_ratio,
                    earlier_ratio
                );
                candidates.push(LearningCandidate {
                    class: FacetClass::Style,
                    key: "verbosity".to_string(),
                    value: "compressed".to_string(),
                    cue_family: CueFamily::Behavioral,
                    evidence: EvidenceRef::EpisodicWindow { from_id, to_id },
                    initial_confidence: 0.65,
                    observed_at: now_secs(),
                });
                state
                    .length_ratio_emitted
                    .insert(("verbosity".to_string(), "compressed".to_string()));
            }

            // Balanced: user message length roughly matches agent.
            if recent_ratio > RATIO_BALANCED
                && !state
                    .length_ratio_emitted
                    .contains(&("verbosity".to_string(), "balanced".to_string()))
            {
                let first_id = state.turns.front().map(|t| t.turn_id.clone());
                let last_id = state.turns.back().map(|t| t.turn_id.clone());
                let from_id = first_id
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(episodic_id);
                let to_id = last_id
                    .and_then(|s| s.parse::<i64>().ok())
                    .unwrap_or(episodic_id);

                tracing::debug!(
                    "[learning::extract::heuristics] length_ratio balanced session={} \
                     recent_ratio={:.2}",
                    session_id,
                    recent_ratio
                );
                candidates.push(LearningCandidate {
                    class: FacetClass::Style,
                    key: "verbosity".to_string(),
                    value: "balanced".to_string(),
                    cue_family: CueFamily::Behavioral,
                    evidence: EvidenceRef::EpisodicWindow { from_id, to_id },
                    initial_confidence: 0.60,
                    observed_at: now_secs(),
                });
                state
                    .length_ratio_emitted
                    .insert(("verbosity".to_string(), "balanced".to_string()));
            }
        }

        // ── B. Edit-window detector ───────────────────────────────────────
        if let Some(prev_at) = prev_agent_at {
            let gap = user_timestamp - prev_at;
            if (0.0..EDIT_WINDOW_SECS).contains(&gap) {
                let lower = user_message.to_ascii_lowercase();
                // Pattern → (key, value) pairs.
                let patterns: &[(&str, &str, &str)] = &[
                    ("shorter", "verbosity", "terse"),
                    ("too long", "verbosity", "terse"),
                    (" less ", "verbosity", "terse"),
                    ("just code", "format", "code-only"),
                    ("not bullets", "format", "prose"),
                    ("more detail", "verbosity", "detailed"),
                    ("more verbose", "verbosity", "detailed"),
                ];
                for (pattern, key, value) in patterns {
                    if lower.contains(pattern) {
                        let now = now_secs();

                        // ── C. Correction-repeat detector ─────────────────────
                        // The counter advances on EVERY in-window correction, regardless
                        // of the edit-window cooldown. This lets 3× corrections promote
                        // to a Veto even if the cooldown suppressed intermediate emissions.
                        let cue_key = format!("{key}={value}");
                        let count = state.correction_counts.entry(cue_key.clone()).or_insert(0);
                        *count += 1;
                        let current_count = *count;

                        if current_count >= VETO_PROMOTION_THRESHOLD
                            && !state.veto_promoted.contains(&cue_key)
                        {
                            tracing::debug!(
                                "[learning::extract::heuristics] correction_repeat veto \
                                 cue={:?} count={} session={}",
                                cue_key,
                                current_count,
                                session_id
                            );
                            let (veto_key, veto_value, veto_confidence) =
                                correction_to_veto(key, value);
                            candidates.push(LearningCandidate {
                                class: FacetClass::Veto,
                                key: veto_key,
                                value: veto_value,
                                cue_family: CueFamily::Behavioral,
                                evidence: EvidenceRef::Episodic { episodic_id },
                                initial_confidence: veto_confidence,
                                observed_at: now,
                            });
                            state.veto_promoted.insert(cue_key);
                        }

                        // Style candidate emission respects the cooldown.
                        let cooldown_key = (key.to_string(), value.to_string());
                        let last_emit = state
                            .edit_cooldown
                            .get(&cooldown_key)
                            .copied()
                            .unwrap_or(0.0);
                        if now - last_emit >= EDIT_COOLDOWN_SECS {
                            tracing::debug!(
                                "[learning::extract::heuristics] edit_window matched \
                                 pattern={:?} key={} value={} session={}",
                                pattern,
                                key,
                                value,
                                session_id
                            );
                            candidates.push(LearningCandidate {
                                class: FacetClass::Style,
                                key: key.to_string(),
                                value: value.to_string(),
                                cue_family: CueFamily::Behavioral,
                                evidence: EvidenceRef::Episodic { episodic_id },
                                initial_confidence: 0.70,
                                observed_at: now,
                            });
                            state.edit_cooldown.insert(cooldown_key, now);
                        }
                    }
                }
            }
        }
    }

    // Push outside the lock.
    let buf = candidate::global();
    let count = candidates.len();
    for c in candidates {
        buf.push(c);
    }
    if count > 0 {
        tracing::debug!(
            "[learning::extract::heuristics] record_turn session={} pushed {} candidate(s)",
            session_id,
            count
        );
    }
}

/// Map a (key, value) Style correction to a Veto triple: (veto_key, veto_value, confidence).
fn correction_to_veto(key: &str, value: &str) -> (String, String, f64) {
    match (key, value) {
        ("format", "prose") => ("format".to_string(), "nested-bullets".to_string(), 0.75),
        ("verbosity", "terse") => ("style".to_string(), "long-replies".to_string(), 0.70),
        _ => (format!("{key}-{value}"), "banned".to_string(), 0.65),
    }
}

/// Compute mean user/agent length ratio for a slice of turns.
/// Returns 0.0 for empty slices or all-zero agent lengths.
fn mean_ratio(turns: &[&TurnEntry]) -> f64 {
    if turns.is_empty() {
        return 0.0;
    }
    let sum: f64 = turns
        .iter()
        .map(|t| {
            if t.agent_msg_len == 0 {
                0.0
            } else {
                t.user_msg_len as f64 / t.agent_msg_len as f64
            }
        })
        .sum();
    sum / turns.len() as f64
}

fn now_secs() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "heuristics_heuristics_tests_tests.rs"]
mod heuristics_tests;
