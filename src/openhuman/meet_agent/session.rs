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

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

use super::ops::{self, Vad, VadEvent};
use super::types::{SessionEvent, SessionEventKind};

/// What `note_caption` decided to do with a caption. Replaces the
/// prior boolean return so the RPC layer can branch between the
/// "fire a normal LLM turn", "speak a polite refusal", and "do
/// nothing" paths without re-doing the gate logic out-of-band.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CaptionOutcome {
    /// Caption was dropped: not a wake, dedupped, cooled down, or
    /// during a turn-in-flight. No audible response.
    Ignored,
    /// Wake fired and the caller should kick `brain::run_caption_turn`.
    WakeFired,
    /// Wake phrase was detected from someone who is not the call
    /// owner (or on a session that hasn't had identities configured).
    /// The caller should speak a polite refusal — or a friendly hi
    /// when the tail is a greeting — via `brain::run_soft_deny_turn`
    /// rather than silently dropping. Carries the full caption text
    /// so the brain layer can classify intent (greeting vs task)
    /// and pick the appropriate canned reply.
    UnauthorizedWake { speaker: String, text: String },
}

/// How long after a denied wake the owner has to say "allow" before
/// the grant request expires. 2 minutes is enough for a back-and-forth
/// exchange ("hey openhuman" — refusal — owner: "go ahead, let them
/// ask") without leaving the gate softened indefinitely.
const PENDING_GRANT_WINDOW_MS: u64 = 120_000;

/// Minimum gap between consecutive non-owner dispatches. Meet's STT
/// re-transcribes the same utterance with slight wording jitter
/// ("Openhuman. I open." → "Openhuman. High openhum." →
/// "Openhuman. High Openhuman.") so per-text dedup misses the
/// duplicates. Without a session-wide rate limit each variant
/// would fire a fresh LLM + TTS round-trip.
///
/// Set at 20s (vs the prior 60s) so a non-owner can actually
/// engage in back-and-forth conversation — the toolless LLM
/// answers general questions now, so a 1-minute gate would feel
/// like the bot has gone deaf between asks. 20s is long enough
/// to cover Meet's STT replay window while letting real new
/// utterances through. 2026-05-25 smoke matrix.
const UNAUTHORIZED_COOLDOWN_MS: u64 = 20_000;

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
    /// Per-speaker last caption text. Drops verbatim repeats from the
    /// page-side observer. A single-slot Option<String> was broken
    /// because Meet's CC region renders two simultaneous rows (the
    /// user's caption AND the bot's TTS being captioned as
    /// speaker="You"). Polling walks both rows every 250ms; with a
    /// single-slot signature the value flips A → B → A → B every
    /// tick and dedup never matches. Per-speaker keying fixes it.
    last_caption_by_speaker: std::collections::HashMap<String, String>,
    /// True between brain-turn dispatch (run_caption_turn entry) and
    /// final-reply enqueue. While set, note_caption refuses to fire a
    /// fresh wake — without this gate, the model takes 5–15s to run
    /// tools but Meet keeps emitting new captions every 250ms, each
    /// firing a new turn that cancels the prior one. Tool calls never
    /// resolve. The gate is wider than `is_speaking()` (which only
    /// covers TTS playback) because the LLM + tool phase is the part
    /// the user can interrupt only by deliberately re-saying the wake
    /// word, which they shouldn't have to.
    pub turn_in_progress: bool,
    /// Set true by `cancel_outbound`; cleared by the next
    /// `poll_outbound`. Tells the shell side that the previous reply
    /// was interrupted and the JS audio bridge should flush any
    /// in-flight playback BEFORE feeding the next chunk. Without this
    /// distinct signal, a normal end-of-utterance would also flush,
    /// cutting the final 100ms of the last legitimate reply.
    flush_pending: bool,
    /// Wall-clock ms at the moment the previous brain turn finished.
    /// Used by note_caption to enforce a minimum gap between turns —
    /// even if the page-side caption cooldown expires (or Meet emits
    /// a fresh utterance just past it), the bot still refuses to
    /// fire a new wake within MIN_TURN_GAP_MS. Backstop against the
    /// "user asks once, bot answers 5 times" pattern when caption
    /// residue keeps re-matching the wake phrase.
    last_turn_done_at_ms: u64,
    /// Display name of the call owner — the user who launched the
    /// bot. Only captions from this speaker may trip the wake word.
    /// Empty until [`set_identities`] is called; while empty the
    /// gate fails closed (no wakes fire) so a misconfigured launch
    /// can never leak the user's tool surface to a remote
    /// participant. Normalisation (lowercase / parenthetical
    /// suffix strip) happens at compare time inside note_caption.
    owner_display_name: String,
    /// Display name the bot uses as its Meet participant tile.
    /// Used to drop the bot's own captions (Meet renders the bot's
    /// TTS in the same captions region as remote speakers; without
    /// an explicit bot-self filter the bot would re-wake on its
    /// own voice). Empty until set; while empty the bot-self filter
    /// is inert.
    bot_display_name: String,
    /// Normalised Meet URL the call joined. Snapshotted at start
    /// so the recent-calls log captures which meeting this was
    /// without forcing the frontend to keep an in-memory map.
    meet_url: String,
    /// Wall-clock ms when `start_session` ran. The session also
    /// keeps `started_at: Instant` for monotonic elapsed-seconds
    /// math, but the JSONL persistence layer needs an absolute
    /// timestamp that can be sorted across process restarts.
    started_at_ms: u64,
    /// Wall-clock ms of the most recent soft-deny dispatch. Used
    /// to enforce `UNAUTHORIZED_COOLDOWN_MS` so a non-owner whose
    /// caption Meet re-transcribes with text variations doesn't
    /// trigger a fresh soft-deny TTS on every variant. 0 = no
    /// soft-deny has dispatched yet this call.
    last_unauthorized_dispatch_at_ms: u64,
    /// Normalised name of the most recent non-owner speaker that
    /// tripped the wake word. Recorded so the owner can grant them
    /// access by saying "allow" / "let them" / "go ahead" within
    /// `PENDING_GRANT_WINDOW_MS` of the refusal. Cleared once a
    /// grant lands or the window elapses.
    pending_unauthorized_speaker: Option<String>,
    /// Wall-clock ms when `pending_unauthorized_speaker` was set.
    /// The owner has `PENDING_GRANT_WINDOW_MS` from this point to
    /// approve the asker.
    pending_unauthorized_at_ms: u64,
    /// Speakers (normalised display names) the owner has explicitly
    /// allowed to wake the bot during this call. Wake gate accepts
    /// captions whose speaker matches the owner OR appears here.
    /// Resets on `stop_session` (the registry drops the whole
    /// session). Empty by default — grants are opt-in per call.
    allowlist: HashSet<String>,
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
            last_caption_by_speaker: std::collections::HashMap::new(),
            turn_in_progress: false,
            flush_pending: false,
            last_turn_done_at_ms: 0,
            last_unauthorized_dispatch_at_ms: 0,
            owner_display_name: String::new(),
            bot_display_name: String::new(),
            meet_url: String::new(),
            started_at_ms: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            pending_unauthorized_speaker: None,
            pending_unauthorized_at_ms: 0,
            allowlist: HashSet::new(),
        }
    }

    /// Add a speaker to the per-call allowlist. The wake gate
    /// thereafter accepts captions from this speaker just like it
    /// would from the owner — single source of truth so the
    /// granted user can ask follow-up questions without saying
    /// "allow" each time. Stored using the normalised name so
    /// Meet's punctuation/case jitter doesn't reset the grant.
    pub fn allow_speaker(&mut self, speaker_display_name: &str) {
        let norm = normalise_participant_name(speaker_display_name);
        if !norm.is_empty() {
            self.allowlist.insert(norm);
        }
    }

    /// Consume the pending unauthorized speaker if still inside the
    /// grant window. Returns the display name (in its normalised
    /// form) so the brain layer can both grant them access and name
    /// them in the spoken confirmation ("Okay, <name> can ask me").
    /// Returns `None` when no pending grant exists or the window
    /// has already elapsed.
    pub fn take_pending_unauthorized(&mut self) -> Option<String> {
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let candidate = self.pending_unauthorized_speaker.take()?;
        if now_ms.saturating_sub(self.pending_unauthorized_at_ms) > PENDING_GRANT_WINDOW_MS {
            // Stale grant — drop without surfacing. The owner would
            // need to re-trigger the refusal flow to re-arm.
            self.pending_unauthorized_at_ms = 0;
            return None;
        }
        self.pending_unauthorized_at_ms = 0;
        Some(candidate)
    }

    /// Record the Meet URL the call joined. Stored alongside the
    /// session so `stop_session` can write it into the JSONL
    /// recent-calls log. Empty string acceptable (older shells that
    /// don't yet forward the URL will simply log calls with an
    /// empty `meet_url` field — the UI degrades gracefully).
    pub fn set_meet_url(&mut self, meet_url: &str) {
        self.meet_url = meet_url.trim().to_string();
    }

    /// Read accessors used when persisting the call record on
    /// `stop_session`. Kept at the session boundary so the store
    /// module doesn't have to reach into private fields.
    pub fn meet_url(&self) -> &str {
        &self.meet_url
    }
    pub fn bot_display_name(&self) -> &str {
        &self.bot_display_name
    }
    pub fn started_at_ms(&self) -> u64 {
        self.started_at_ms
    }

    /// Set the call-owner display name (the human who launched the
    /// bot) and the bot's own Meet participant name. The note_caption
    /// gate uses both: captions are accepted only when the speaker
    /// matches the owner, and the bot-self filter drops captions
    /// authored by the bot's own TTS feed.
    ///
    /// Either argument may be empty. Empty owner_display_name
    /// fails-closed (the gate refuses every wake) so a misconfigured
    /// launch can never expose the user's tool surface to a remote
    /// participant. Empty bot_display_name simply disables the
    /// bot-self filter — the dedup / cooldown layers still keep the
    /// loop in check, but it's a less-defended posture.
    pub fn set_identities(&mut self, owner_display_name: &str, bot_display_name: &str) {
        self.owner_display_name = owner_display_name.trim().to_string();
        self.bot_display_name = bot_display_name.trim().to_string();
    }

    /// Read accessor used by audit logging. Empty when set_identities
    /// has not been called for this session.
    pub fn owner_display_name(&self) -> &str {
        &self.owner_display_name
    }

    /// Stamp the current wall-clock time as "turn just finished". The
    /// brain calls this from the final with_session block of
    /// run_caption_turn (alongside clearing turn_in_progress) so the
    /// min-turn-gap backstop in note_caption can see it.
    pub fn mark_turn_done(&mut self) {
        self.last_turn_done_at_ms = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
    }

    /// True when the brain has TTS audio queued for playback. The
    /// note_caption gate uses this to refuse wake matches while the
    /// bot is actively speaking — otherwise Meet captions the bot's
    /// own voice (or the user keeps talking through the reply) and
    /// fires a fresh turn before the current one finishes, producing
    /// an unbreakable speak-listen-speak loop.
    pub fn is_speaking(&self) -> bool {
        !self.outbound.is_empty()
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
    pub fn note_caption(&mut self, speaker: &str, text: &str, ts_ms: u64) -> CaptionOutcome {
        if text.trim().is_empty() {
            return CaptionOutcome::Ignored;
        }
        // Drop noise captions from Meet's local-user / UI affordances.
        // `speaker=="You"` is Meet's label for the local participant
        // (the bot itself when its outbound is the user-facing tile),
        // plus a catch-all for placeholder / demo / accessibility
        // strings that some Meet variants surface inside the caption
        // region. Without this filter the bot's own TTS would loop
        // back as a "user spoke" prompt and re-fire the wake word,
        // eating the prompt budget and producing endless speech.
        let speaker_lower = speaker.trim().to_lowercase();
        if speaker_lower == "you" || speaker_lower.is_empty() {
            return CaptionOutcome::Ignored;
        }
        // Privacy gate — owner-only wake.
        //
        // Today the brain runs the user's full orchestrator agent with
        // their tool surface (calendar, Slack, Gmail, … 119 Composio
        // integrations) and the user's memory tree. A meeting is a
        // public room. Without an identity gate, *any* participant who
        // says the wake phrase (or whose audio Meet transcribes near
        // one) can issue tool calls in the user's name and have the
        // results spoken aloud to the whole room — a hard privacy
        // leak. So before any wake / dedup / cooldown work happens we
        // require: speaker == owner_display_name. Anyone else (and
        // the bot itself) is dropped without recording an event.
        //
        // Normalisation is intentionally light (lowercase + trim +
        // parenthetical suffix strip) so Meet's "(host)" / "(you)"
        // decorations don't break the match. Anything fancier
        // (NFKC, diacritic folding) waits for a real-name smoke
        // report — start tight, expand only on evidence.
        let speaker_norm = normalise_participant_name(speaker);
        let owner_norm = normalise_participant_name(&self.owner_display_name);
        let bot_norm = normalise_participant_name(&self.bot_display_name);
        // Bot-self filter first: a bot caption that happens to match
        // its own display name must never re-wake. Run before the
        // owner check so a (very contrived) bot_display_name ==
        // owner_display_name still doesn't let the bot wake itself.
        if !bot_norm.is_empty() && speaker_norm == bot_norm {
            return CaptionOutcome::Ignored;
        }
        // Fail-closed when no owner has been configured. A live
        // session without a known owner is by definition unsafe —
        // any participant could wake. Log once per such caption so
        // operators can spot the misconfiguration in the dev log.
        if owner_norm.is_empty() {
            log::warn!(
                "[meet-agent] wake refused: no owner_display_name configured \
                 request_id={} speaker={}",
                self.request_id,
                speaker
            );
            return CaptionOutcome::Ignored;
        }
        // Treat owner + previously-granted allowlist members as
        // authorised speakers for wake purposes. The allowlist is
        // populated when the owner says "allow them" / "go ahead"
        // / "let them ask" after a non-owner wake refusal — see
        // `brain::run_caption_turn`'s grant-intent branch.
        //
        // The actual authorised/unauthorised branch happens AFTER
        // all the rate-limit gates (dedup, turn-in-progress, min-
        // turn-gap, cooldown) below, so the same caption repeating
        // every 250 ms — which Meet does aggressively while a
        // participant is still visible in the CC region — cannot
        // spam the refusal path either. Without that ordering the
        // soft-deny TTS triggers a fresh refusal on every Meet
        // re-emit of the identical caption text. Smoke-tested as
        // the "sorry sorry sorry" loop on 2026-05-25.
        let speaker_is_authorised =
            speaker_norm == owner_norm || self.allowlist.contains(&speaker_norm);
        // Per-speaker dedup. Meet's CC region re-renders the same line
        // every 250 ms poll tick and emits BOTH speaker rows on each
        // walk (the user AND the bot TTS as speaker="You"). A single-
        // slot last-signature would flip A → B → A → B every tick and
        // never dedup. Keyed by speaker_lower so the user's repeating
        // utterance is dropped after the first hit regardless of bot
        // captions interleaving.
        //
        // Normalised match (lowercase + drop non-alphanumeric + collapse
        // whitespace) so Meet's punctuation/case jitter between emits
        // ("Hey, openhuman" → "hey openhuman.") doesn't slip through
        // the dedup. Without normalisation each capitalisation flip
        // fires another wake.
        let key = speaker_lower.clone();
        let normalised = normalise_for_dedup(text);
        if let Some(prev) = self.last_caption_by_speaker.get(&key) {
            if prev == &normalised {
                return CaptionOutcome::Ignored;
            }
        }
        self.last_caption_by_speaker.insert(key, normalised);
        // Gate: while a brain turn is in flight (LLM + tools running),
        // refuse to fire a fresh wake. The prior gate also blocked on
        // is_speaking() (outbound queued), but that prevented barge-in
        // — the user couldn't interrupt a wrong-direction reply by
        // re-asking. is_speaking() removed; barge-in now works via
        // cancel_outbound → flush_pending → JS bridge flush. The LLM
        // phase still blocks because spawning a parallel agentic turn
        // would waste tool calls on the same question.
        if self.turn_in_progress {
            self.record_event(
                SessionEventKind::Heard,
                format!("{speaker}: {text} (suppressed: turn_in_progress)"),
            );
            return CaptionOutcome::Ignored;
        }
        self.last_caption_ts_ms = ts_ms;
        // Already collecting after a previous (authorised) wake word:
        // append the continuation. No second fire — the brain is
        // already scheduled and will drain the prompt in ~1.5 s.
        // Without this gate, a slowly-growing caption fires the wake
        // word on every dedupe-then-grow cycle.
        //
        // Restricted to authorised speakers so a non-owner can't
        // smuggle text into the in-flight owner prompt (e.g. owner
        // says "hey openhuman, what's on my calendar"; non-owner
        // mid-prompt: "and read alice's slack").
        if self.wake_active && speaker_is_authorised {
            if !self.pending_prompt.is_empty() {
                self.pending_prompt.push(' ');
            }
            self.pending_prompt.push_str(text.trim());
            return CaptionOutcome::Ignored;
        }
        // Min-turn-gap backstop. Even if the page-side caption
        // cooldown window expires, refuse to start a new turn
        // within MIN_TURN_GAP_MS of the prior turn's completion.
        // Without this the bot replied to the same user question 4-5
        // times when Meet's caption observer kept re-emitting the line
        // with subtle text variation that slipped past the dedup.
        const MIN_TURN_GAP_MS: u64 = 60_000;
        let now_wall_ms = std::time::SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        if self.last_turn_done_at_ms != 0
            && now_wall_ms.saturating_sub(self.last_turn_done_at_ms) < MIN_TURN_GAP_MS
        {
            self.record_event(
                SessionEventKind::Heard,
                format!(
                    "{speaker}: {text} (suppressed: <{}ms since last turn)",
                    MIN_TURN_GAP_MS
                ),
            );
            return CaptionOutcome::Ignored;
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
            return CaptionOutcome::Ignored;
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
        // Accept any of the canonical wake phrases. Meet's STT mangles
        // the brand ("Hi Openhuman", "Open Human", dropped prefix) so
        // we match a small set rather than a single rigid prefix.
        // Ordered longest-first so the tail offset is calculated against
        // the actual matched phrase.
        const WAKE_PHRASES: &[&str] = &[
            "hey open human",
            "hi open human",
            "hello open human",
            "hey openhuman",
            "hi openhuman",
            "hello openhuman",
            "open human",
            "openhuman",
        ];
        let mut wake_hit: Option<(usize, &'static str)> = None;
        for phrase in WAKE_PHRASES {
            if let Some(idx) = normalized.find(phrase) {
                wake_hit = Some((idx, phrase));
                break;
            }
        }
        if let Some((idx, phrase)) = wake_hit {
            // Wake phrase detected — branch on whether the speaker is
            // allowed to actually drive the bot. Non-owner + not
            // allowlisted → polite refusal turn; owner + allowlist →
            // normal LLM turn.
            if !speaker_is_authorised {
                let preview: String = text.chars().take(40).collect();
                let now_ms = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map(|d| d.as_millis() as u64)
                    .unwrap_or(0);
                // Session-wide soft-deny cooldown. Meet's STT
                // re-transcribes the same utterance with wording
                // jitter, slipping past the per-text dedup. Cap the
                // refusal TTS to one dispatch per minute so the loop
                // can't compound itself (and so rate-limits from the
                // TTS backend don't fire either).
                if self.last_unauthorized_dispatch_at_ms != 0
                    && now_ms.saturating_sub(self.last_unauthorized_dispatch_at_ms)
                        < UNAUTHORIZED_COOLDOWN_MS
                {
                    log::debug!(
                        "[meet-agent] unauthorized_wake suppressed (cooldown) \
                         request_id={} speaker=\"{}\" preview=\"{}\"",
                        self.request_id,
                        speaker,
                        preview
                    );
                    return CaptionOutcome::Ignored;
                }
                log::info!(
                    "[meet-agent] unauthorized_wake_attempt request_id={} \
                     speaker=\"{}\" owner=\"{}\" preview=\"{}\"",
                    self.request_id,
                    speaker,
                    self.owner_display_name,
                    preview
                );
                self.last_unauthorized_dispatch_at_ms = now_ms;
                // Record the pending grant request. The owner has
                // PENDING_GRANT_WINDOW_MS to approve them via the
                // "allow" / "let them" / "go ahead" pattern; after
                // that the slot expires and the unauthorised speaker
                // has to re-trigger the refusal to re-arm.
                self.pending_unauthorized_speaker = Some(speaker.trim().to_string());
                self.pending_unauthorized_at_ms = now_ms;
                return CaptionOutcome::UnauthorizedWake {
                    speaker: speaker.trim().to_string(),
                    text: text.to_string(),
                };
            }
            let after = idx + phrase.len();
            let tail = normalized.get(after..).unwrap_or("").trim().to_string();
            self.pending_prompt = tail;
            self.wake_active = true;
            self.record_event(
                SessionEventKind::Note,
                format!("wake word from speaker={speaker}"),
            );
            return CaptionOutcome::WakeFired;
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
        CaptionOutcome::Ignored
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
        // 60s grace beyond the most recent caption's page timestamp.
        // The previous 8s window was too short: Meet's caption region
        // re-renders the just-finished utterance for 5-8s, the bot's
        // reply takes another 5-15s to synthesize + speak, then any
        // natural user follow-up ("wait, did you say X?") within the
        // same 60s window is treated as continuation rather than a
        // fresh wake. Under-prompted users especially repeat the wake
        // phrase 2-3 times before realising the bot already heard them
        // — without this, each repeat fires another tool call.
        const COOLDOWN_MS: u64 = 60_000;
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

    /// Drop everything queued for playback. The brain calls this at
    /// the start of a new caption turn so the bot stops mid-sentence
    /// instead of letting the previous reply play to completion while
    /// the user is already speaking again. Marks the outbound channel
    /// as 'done' so the speak_pump signals end-of-utterance on its
    /// next poll and the page bridge can reset its audio-bridge state
    /// cleanly.
    pub fn cancel_outbound(&mut self) {
        // Mark flush BEFORE the early-empty check — even if the Rust
        // queue happens to be empty right now, the JS bridge may have
        // already pulled the prior reply's tail and be playing it
        // standalone. The flush signal must still fire.
        self.flush_pending = true;
        if !self.outbound.is_empty() {
            self.outbound.clear();
        }
        self.outbound_done = true;
    }

    /// Take + clear the pending-flush flag. Called by the shell on
    /// every poll_outbound; when true, the shell will issue a JS
    /// bridge flush BEFORE feeding the next PCM chunk so the prior
    /// reply's in-flight playback stops cleanly.
    pub fn take_flush_pending(&mut self) -> bool {
        std::mem::take(&mut self.flush_pending)
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

/// Canonicalise a Meet participant display name for the owner-gate
/// comparison. Strips a single trailing parenthetical decorator
/// (Meet appends `" (host)"`, `" (you)"`, `" (presenter)"` to some
/// captions and labels), lowercases ASCII, and collapses internal
/// whitespace. NFKC folding is *not* applied — start tight and
/// expand on real-world miss reports rather than guessing at the
/// shape of names we haven't seen yet. Returns empty when the input
/// is empty / whitespace-only so the caller can fail-closed.
fn normalise_participant_name(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    // Strip a single trailing parenthetical (e.g. "Alice (host)").
    // We only strip when the parenthetical is at the end and the
    // preceding chunk is non-empty — guards against pathological
    // inputs like "()" or "(host)" alone.
    let stripped: &str = if let Some(open_idx) = trimmed.rfind(" (") {
        if trimmed.ends_with(')') && open_idx > 0 {
            &trimmed[..open_idx]
        } else {
            trimmed
        }
    } else {
        trimmed
    };
    // Lowercase + collapse internal whitespace.
    let mut out = String::with_capacity(stripped.len());
    let mut prev_space = true;
    for c in stripped.chars() {
        if c.is_whitespace() {
            if !prev_space {
                out.push(' ');
                prev_space = true;
            }
        } else {
            for lc in c.to_lowercase() {
                out.push(lc);
            }
            prev_space = false;
        }
    }
    out.trim_end().to_string()
}

/// Lowercase + drop non-alphanumeric + collapse whitespace. Used by
/// the per-speaker dedup so Meet's punctuation/case jitter between
/// caption emits doesn't bypass the dedup. Same shape as
/// `normalize_for_wake` but exposed under a distinct name to keep
/// the two intents (wake-word match vs. dedup key) separate at the
/// call site.
fn normalise_for_dedup(text: &str) -> String {
    normalize_for_wake(text)
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

    /// Build a session pre-configured for the wake-word tests: Alice
    /// is the call owner, "OpenHuman" is the bot's Meet tile. Every
    /// wake-path test goes through this helper so the owner gate
    /// (the privacy hard-lock around tool calls) is consistently
    /// in scope.
    fn session_with_owner_alice() -> MeetAgentSession {
        let mut s = MeetAgentSession::new("p".into(), 16_000);
        s.set_identities("Alice", "OpenHuman");
        s
    }

    #[test]
    fn note_caption_handles_punctuated_wake() {
        let mut s = session_with_owner_alice();
        // Meet often inserts a comma after "hey".
        let outcome = s.note_caption("Alice", "Hey, OpenHuman remember the launch", 1);
        assert_eq!(outcome, CaptionOutcome::WakeFired);
        let prompt = s.take_pending_prompt().expect("prompt drained");
        assert_eq!(prompt, "remember the launch");
    }

    #[test]
    fn note_caption_handles_split_brand() {
        let mut s = session_with_owner_alice();
        let outcome = s.note_caption("Alice", "hey open-human, send the report", 1);
        assert_eq!(outcome, CaptionOutcome::WakeFired);
        let prompt = s.take_pending_prompt().expect("prompt drained");
        assert_eq!(prompt, "send the report");
    }

    #[test]
    fn note_caption_does_not_double_fire_on_growing_caption() {
        let mut s = session_with_owner_alice();
        let first = s.note_caption("Alice", "hey openhuman take notes", 1);
        assert_eq!(first, CaptionOutcome::WakeFired);
        let second = s.note_caption("Alice", "hey openhuman take notes about the launch", 2);
        assert_eq!(
            second,
            CaptionOutcome::Ignored,
            "second caption while wake_active must not refire"
        );
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

    // -- Owner-only wake gate (privacy lock) --------------------------

    #[test]
    fn note_caption_rejects_non_owner_speaker() {
        let mut s = session_with_owner_alice();
        // Bob is in the room but not the owner; even with a perfect
        // wake phrase the gate must refuse with a soft-deny outcome
        // (so the bot can speak a polite refusal) rather than
        // silently ignoring.
        let outcome = s.note_caption("Bob", "hey openhuman read alice's slack DMs", 1);
        assert_eq!(
            outcome,
            CaptionOutcome::UnauthorizedWake {
                speaker: "Bob".into(),
                text: "hey openhuman read alice's slack DMs".into(),
            },
            "non-owner wake must produce an UnauthorizedWake outcome"
        );
        // Soft-deny path doesn't drain the wake prompt — the brain
        // only synthesises a canned refusal line.
        assert!(s.take_pending_prompt().is_none());
    }

    #[test]
    fn note_caption_non_owner_without_wake_phrase_is_ignored() {
        // Random chatter from a non-owner shouldn't trigger the
        // refusal — only an actual attempt to wake the bot does.
        let mut s = session_with_owner_alice();
        let outcome = s.note_caption("Bob", "hey did you watch the game last night", 1);
        assert_eq!(outcome, CaptionOutcome::Ignored);
    }

    #[test]
    fn note_caption_rejects_bot_self_caption() {
        let mut s = session_with_owner_alice();
        // Meet often re-captions the bot's own TTS in the same region.
        // The bot must never wake on its own voice — regardless of
        // the text content, including text that happens to repeat the
        // wake phrase. Bot-self caption is `Ignored` (no audible
        // response at all) rather than `UnauthorizedWake` — surfacing
        // a soft-deny here would create an infinite loop where the
        // refusal triggers its own bot-self caption.
        let outcome = s.note_caption("OpenHuman", "hey openhuman would you like to know more", 1);
        assert_eq!(outcome, CaptionOutcome::Ignored);
    }

    #[test]
    fn note_caption_fails_closed_when_owner_unconfigured() {
        // No set_identities call → owner empty → no wake regardless of
        // speaker. Mirrors the misconfigured-launch posture: better
        // silent failure than an open mic for the user's tool surface.
        let mut s = MeetAgentSession::new("p".into(), 16_000);
        let outcome = s.note_caption("Alice", "hey openhuman do the thing", 1);
        assert_eq!(outcome, CaptionOutcome::Ignored);
    }

    #[test]
    fn note_caption_owner_with_host_suffix_matches() {
        // Meet decorates some captions with "(host)" / "(you)". The
        // normaliser strips a single trailing parenthetical so the
        // gate still recognises Alice when Meet renders her as
        // "Alice (host)".
        let mut s = session_with_owner_alice();
        let outcome = s.note_caption("Alice (host)", "hey openhuman take a note", 1);
        assert_eq!(outcome, CaptionOutcome::WakeFired);
    }

    #[test]
    fn note_caption_owner_case_insensitive() {
        // Meet sometimes title-cases display names that the user
        // entered in lowercase, or vice versa. The comparison must
        // be case-insensitive.
        let mut s = session_with_owner_alice();
        let outcome = s.note_caption("ALICE", "hey openhuman summarise", 1);
        assert_eq!(outcome, CaptionOutcome::WakeFired);
    }

    #[test]
    fn allowlist_grants_subsequent_wakes() {
        // After the owner grants Bob via `allow_speaker`, Bob's
        // next wake-phrase caption should fire just like the
        // owner's — no soft-deny, no Ignored.
        let mut s = session_with_owner_alice();
        // First attempt without a grant is soft-deny:
        let denied = s.note_caption("Bob", "hey openhuman read slack", 1);
        assert!(matches!(denied, CaptionOutcome::UnauthorizedWake { .. }));
        // Owner grants Bob:
        s.allow_speaker("Bob");
        // Bob now wakes successfully. Use a different text so the
        // per-speaker dedup doesn't reject it.
        let granted = s.note_caption("Bob", "hey openhuman what's the weather", 2);
        assert_eq!(granted, CaptionOutcome::WakeFired);
    }

    #[test]
    fn note_caption_unauthorized_wake_cooldown_blocks_text_variants() {
        // Meet's STT re-transcribes the same utterance with text
        // jitter ("Openhuman. I open." → "Openhuman. High openhum.")
        // — the per-text dedup doesn't catch these because the
        // strings differ. The session-wide soft-deny cooldown must
        // gate subsequent variants from the same speaker so only
        // one refusal TTS dispatches per minute regardless of
        // STT churn.
        let mut s = session_with_owner_alice();
        let first = s.note_caption("Bob", "Openhuman. I open.", 1);
        assert!(matches!(first, CaptionOutcome::UnauthorizedWake { .. }));
        // Different text but same speaker → still cooled down.
        let second = s.note_caption("Bob", "Openhuman. High openhum.", 2);
        assert_eq!(second, CaptionOutcome::Ignored);
        let third = s.note_caption("Bob", "Openhuman. High Openhuman.", 3);
        assert_eq!(third, CaptionOutcome::Ignored);
        // Different speaker also gated — soft-deny TTS slot is
        // session-wide, not per-speaker.
        let charlie = s.note_caption("Charlie", "openhuman hello", 4);
        assert_eq!(charlie, CaptionOutcome::Ignored);
    }

    #[test]
    fn note_caption_unauthorized_wake_does_not_loop_on_identical_caption() {
        // Regression: Meet's caption observer re-emits the same row
        // every 250 ms while it's still visible. The first emission
        // produces an UnauthorizedWake; subsequent identical
        // emissions must be deduped to `Ignored` so the soft-deny
        // TTS doesn't fire on every tick ("sorry, sorry, sorry…"
        // loop seen in dev:app on 2026-05-25).
        let mut s = session_with_owner_alice();
        let first = s.note_caption("Bob", "hey openhuman read my dms", 1);
        assert!(matches!(first, CaptionOutcome::UnauthorizedWake { .. }));
        // Same text from same speaker — must dedup to Ignored.
        let second = s.note_caption("Bob", "hey openhuman read my dms", 2);
        assert_eq!(second, CaptionOutcome::Ignored);
        // Punctuation/case jitter on the same utterance still dedups
        // because the normaliser strips it before compare.
        let third = s.note_caption("Bob", "Hey, openhuman read my DMs.", 3);
        assert_eq!(third, CaptionOutcome::Ignored);
    }

    #[test]
    fn take_pending_unauthorized_returns_within_window() {
        // The soft-deny path records the speaker so the owner can
        // grant them shortly after. Inside the window we get the
        // name back; we'd need to fast-forward time to test the
        // expiry path, so just assert the in-window happy path here.
        let mut s = session_with_owner_alice();
        let _ = s.note_caption("Bob", "hey openhuman list my emails", 1);
        let pending = s.take_pending_unauthorized();
        assert_eq!(pending.as_deref(), Some("Bob"));
        // Consumed — second take returns None.
        assert!(s.take_pending_unauthorized().is_none());
    }

    #[test]
    fn normalise_participant_name_strips_trailing_paren() {
        assert_eq!(normalise_participant_name("Alice (host)"), "alice");
        assert_eq!(normalise_participant_name("Bob (you)"), "bob");
        // No paren — left as-is (modulo lowercase / trim).
        assert_eq!(normalise_participant_name("  Charlie  "), "charlie");
        // Internal whitespace collapsed.
        assert_eq!(normalise_participant_name("First  Last"), "first last");
        // Pathological standalone paren — preserved so the gate can
        // still treat it as a name distinct from the owner.
        assert_eq!(normalise_participant_name("(host)"), "(host)");
        // Empty stays empty so callers can fail-closed.
        assert_eq!(normalise_participant_name(""), "");
        assert_eq!(normalise_participant_name("   "), "");
    }

    #[test]
    fn set_identities_trims_whitespace() {
        let mut s = MeetAgentSession::new("p".into(), 16_000);
        s.set_identities("  Alice  ", "\tOpenHuman\n");
        assert_eq!(s.owner_display_name(), "Alice");
    }

    /// MIN_TURN_GAP_MS: immediately after a turn completes, the owner's
    /// next wake must be gated so fast re-fires ("did you hear me?")
    /// don't spawn duplicate tool calls within the 60s backstop window.
    #[test]
    fn note_caption_respects_min_turn_gap_after_mark_turn_done() {
        let mut s = session_with_owner_alice();

        // First wake fires normally.
        let first = s.note_caption("Alice", "hey openhuman what time is it", 1);
        assert!(matches!(first, CaptionOutcome::WakeFired));
        let _ = s.take_pending_prompt();

        // Simulate brain completing a turn — sets last_turn_done_at_ms to now.
        s.mark_turn_done();

        // Immediately re-firing the wake word (wall-clock gap = ~0ms)
        // must be suppressed by MIN_TURN_GAP_MS (60s).
        let too_soon = s.note_caption("Alice", "hey openhuman what time is it", 100);
        assert_eq!(
            too_soon,
            CaptionOutcome::Ignored,
            "wake fired within MIN_TURN_GAP_MS of prior turn must be suppressed"
        );
    }

    /// The wake-cooldown window (COOLDOWN_MS = 60s, keyed on caption ts_ms)
    /// must gate the same wake phrase from the owner within the cooldown
    /// window even if the prompt was already drained.
    #[test]
    fn note_caption_wake_cooldown_suppresses_repeat_within_window() {
        let mut s = session_with_owner_alice();

        // First wake fires and is drained.
        let first = s.note_caption("Alice", "openhuman help", 1_000);
        assert!(matches!(first, CaptionOutcome::WakeFired));
        let _ = s.take_pending_prompt();

        // A different wake phrase arriving at ts_ms = 2_000 is within
        // the 60s cooldown window (cooldown_until = 1_000 + 60_000 = 61_000).
        let during_cooldown = s.note_caption("Alice", "hey openhuman something else", 2_000);
        assert_eq!(
            during_cooldown,
            CaptionOutcome::Ignored,
            "wake within 60s caption-ts cooldown must be suppressed"
        );

        // A caption arriving after the cooldown window (ts_ms = 62_000) fires.
        let after_cooldown = s.note_caption("Alice", "hey openhuman one more thing", 62_000);
        assert!(
            matches!(after_cooldown, CaptionOutcome::WakeFired),
            "wake after cooldown window must be allowed through: got {after_cooldown:?}"
        );
    }
}
