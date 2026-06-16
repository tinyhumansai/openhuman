//! Compile-time constants and the process-wide agent cache shared
//! across all sub-modules in `brain/`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use tokio::sync::Mutex as TokioMutex;

use crate::openhuman::agent::harness::session::Agent;

/// Process-wide cache of orchestrator Agents keyed by `request_id`.
/// Each meet session reuses the same Agent across all its turns so
/// the harness's in-memory `Agent.history` accumulates and the
/// orchestrator can recall prior dialogue ("did I tell you to
/// remember Friday?", "what did Alice say earlier?"). Without the
/// cache each turn builds a fresh Agent, loses the prior turn's
/// memory, and pays the 5-10s build cost every time.
///
/// Locked with `tokio::sync::Mutex` because we hold the inner
/// `Arc<TokioMutex<Agent>>` lock across `run_single().await` —
/// std::sync::Mutex cannot be held across await without breaking
/// Send + leaking the lock on cancel.
static AGENT_CACHE: OnceLock<TokioMutex<HashMap<String, Arc<TokioMutex<Agent>>>>> = OnceLock::new();

pub(super) fn agent_cache() -> &'static TokioMutex<HashMap<String, Arc<TokioMutex<Agent>>>> {
    AGENT_CACHE.get_or_init(|| TokioMutex::new(HashMap::new()))
}

/// Wall-clock ceiling on one agentic turn. Slack / Gmail fetches via
/// Composio + per-message filtering + iteration-2 synthesis can hit
/// 60-80s in the slow path. 90s gives the long integrations a chance
/// to land. The turn_in_progress gate blocks new wakes during the
/// wait, so the user cannot spawn parallel queries by re-asking.
pub(super) const AGENTIC_TURN_TIMEOUT_SECS: u64 = 90;

/// Spoken filler played immediately after wake-word fires, before the
/// (possibly slow) orchestrator+tool path runs. Bridges the 30-60s
/// silence on slow integration paths. Kept short (~1s synth) so it
/// doesn't intrude on fast greetings / time questions.
pub(super) const PREROLL_ACK_PHRASE: &str = "On it.";

/// How many of the most recent `Heard` / `Spoke` events we feed back
/// into the LLM as rolling conversation context. 12 ≈ a few minutes of
/// captioned dialogue — enough for the model to follow a thread without
/// blowing the prompt budget.
pub(super) const CONTEXT_EVENT_WINDOW: usize = 12;

/// Spoken-reply ceiling. Each token is roughly ¾ of a word, so 80
/// tokens ≈ ~60 spoken words ≈ ~12 seconds. The system prompt asks for
/// one short sentence, but reasoning-style backends ignore soft length
/// hints and emit 800+ char monologues. Hard token cap keeps the bot
/// interruptible regardless of model behaviour.
pub(super) const REPLY_MAX_TOKENS: u32 = 80;

/// ElevenLabs model. `eleven_turbo_v2_5` strikes the best
/// quality/latency balance; the older default the backend would pick
/// (`eleven_monolingual_v1`) sounds noticeably flatter.
pub(super) const TTS_MODEL_ID: &str = "eleven_turbo_v2_5";

/// Hard ceiling on reply characters fed to TTS. The LLM is asked to be
/// concise but reasoning models still emit 800+ char paragraphs. Cap
/// drops everything past the first sentence boundary at-or-before
/// this index, falling back to a raw char cut when no boundary fits.
/// ~25s of speech at average prosody — keeps the bot interruptible
/// and prevents the "60s monologue / can't talk over it" loop.
pub(super) const MAX_TTS_CHARS: usize = 400;

/// Minimum samples below which we skip the brain turn entirely.
/// 250 ms @ 16 kHz — under this, VAD almost certainly fired on a
/// transient (cough, click) rather than real speech.
pub(super) const MIN_TURN_SAMPLES: usize = 4_000;

/// Re-exported from `ops` so any drift (if we ever loosen the
/// boundary check) immediately breaks the WAV / duration math here
/// at compile time. Today the same constant is used in both places —
/// the ops boundary check rejects anything else outright.
pub(super) const SAMPLE_RATE_HZ: u32 = crate::openhuman::meet_agent::ops::REQUIRED_SAMPLE_RATE;

/// Delay between wake-word match and prompt drain. Long enough that
/// 2-3 caption fragments can join up; short enough that the user
/// doesn't experience awkward silence after they stop talking.
pub(super) const CAPTION_TURN_DELAY_MS: u64 = 1_500;

/// Prompt character threshold below which we skip the pre-roll ack.
/// Short prompts (greetings, trivial checks) are answered in 2-5s
/// without tools — they don't need an ack, and "On it. Yes, I can
/// hear you" sounds redundant.
pub(super) const PREROLL_SKIP_PROMPT_CHARS: usize = 50;

/// Canned acknowledgements the agent speaks out loud after capturing
/// a note. Short, varied so consecutive notes don't sound robotic.
/// Selected by hashing the prompt so the same dictation reliably
/// produces the same ack (helpful for tests + debugging) while still
/// rotating across the set in a normal conversation.
pub(super) const ACK_PHRASES: &[&str] =
    &["Got it.", "Noted.", "Adding that.", "On it.", "Captured."];

/// System prompt for the live meeting agent. Pushes the model toward
/// (a) recognising whether the latest utterance is genuinely directed
/// at it (intent classification — emit empty string when not), and
/// (b) responding conversationally and concisely when it is.
#[allow(dead_code)]
pub(super) const MEETING_SYSTEM_PROMPT: &str = "\
You are OpenHuman, joining a live Google Meet call by voice. Every word you \
produce will be spoken aloud over the call. The transcript shows `user` lines \
(humans on the call, sometimes prefixed with a name) and `assistant` lines \
(things you previously said out loud).\n\
\n\
STRICT OUTPUT RULES — these are non-negotiable. The output is fed DIRECTLY \
into TTS and spoken aloud verbatim. Any meta-text becomes audible bot \
gibberish on a live call.\n\
1. Output ONE sentence. Maximum 25 spoken words.\n\
2. Plain spoken English. No markdown. No bullets. No code. No emoji.\n\
3. NO chain-of-thought. NO reasoning. NO planning. NO <think> blocks. NO \
preamble. NEVER write phrases like \"We need to…\", \"I should…\", \"Let me…\", \
\"The user said…\", \"This is a greeting…\", \"So I should respond with…\", \
\"My response is…\". Output ONLY the final answer that the user should hear.\n\
4. Never repeat what the user said. Never narrate what you are about to do.\n\
5. If the latest user line is not directly addressed to you, output the empty \
string. Do not respond to side conversations or ambient speech.\n\
6. Examples — good vs bad:\n\
   User: \"hello\" → GOOD: \"Hey there.\"  BAD: \"The user said hello, so I should respond with a greeting.\"\n\
   User: \"what's the time\" → GOOD: \"I don't have a clock right now.\"  BAD: \"We need to generate a single sentence. The user is asking the time.\"\n\
\n\
Address-detection: respond when the user names you (\"OpenHuman\", \"hey \
openhuman\"), asks a direct question of you, or gives a direct command \
(remember, summarise, look up). Otherwise stay silent.\n\
\n\
For unanswerable questions: say so in one sentence (\"I don't know that off \
the top of my head\") instead of guessing or stalling.\n\
For dictation / note requests: a 2-3 word ack (\"Got it.\", \"Noted.\"). Don't \
read the note back.\n\
";

/// Voice-frontend system-prompt directive prepended to the user
/// utterance before it reaches the orchestrator. The orchestrator
/// already has its own persona, tool catalogue, memory loader and
/// connected integrations; this addendum just tells it the answer is
/// going to be spoken aloud verbatim so it should reply in one short
/// spoken sentence with no markdown / no chain-of-thought / no
/// preamble. Wrapped in a delimiter so the orchestrator can't confuse
/// the directive with the user's actual utterance.
pub(super) const MEET_VOICE_DIRECTIVE: &str = "\
MEETING VOICE MODE — this conversation is happening live over voice in a Google Meet call.\n\
\n\
LANGUAGE: Respond in ENGLISH ONLY. Do not switch languages even if a user's name, prior memory, or transcript hint suggests another locale. The TTS engine is English-only; non-English output produces garbled audio.\n\
\n\
TOOL USE (encouraged):\n\
- USE TOOLS whenever a tool can give a real answer. Calendar, email, slack, memory, integrations — \
call them. Tool calls are invisible to the user and DO NOT count toward your reply word budget.\n\
- If you need data from a tool to answer accurately, CALL THE TOOL. Do not guess from prior training. \
Do not claim something is not connected before attempting to call its tool — the tool surface above \
shows what is actually available right now.\n\
- delegate_to_integrations_agent is your gateway to all connected provider integrations (calendar, \
gmail, slack, etc.). Use it when the user asks about their schedule, mail, messages, or any other \
integration-backed data.\n\
\n\
FINAL SPOKEN REPLY (strict — this is the only part the user hears):\n\
- After tool work is done, output ONE short spoken sentence, max 25 words.\n\
- Plain spoken English only. No markdown. No bullets. No code. No URLs.\n\
- No meta-narration. Do not say \"Let me check…\", \"I will look…\", \"The user is asking…\", \
\"We need to…\", \"I should…\". Just give the answer.\n\
- If the user is not directly addressing you (chit-chat between humans, side conversation, your \
name appearing inside a longer thought aimed at someone else), output an empty string and stay silent.\n\
- For dictation / note requests (\"remember…\", \"action item…\", \"follow up on…\"), a 2-3 word \
ack is enough (\"Got it.\", \"Noted.\").\n\
- For genuinely unanswerable questions, say so in one short sentence rather than guessing.";
