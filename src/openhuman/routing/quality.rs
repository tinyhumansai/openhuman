//! Response quality assessment for routing fallback decisions.
//!
//! `is_low_quality` runs on the hot path after every local-model
//! inference response, before the routing layer commits to serving the
//! response or falling back to a remote model. False negatives (serving
//! a refusal / useless reply) are far more visible to the user than
//! false positives (an unnecessary remote call), so the heuristic is
//! intentionally conservative on the "low quality" side.
//!
//! ## Design
//!
//! - **Length floor** — anything shorter than [`MIN_CHARS`] after trim
//!   is low quality. Cheap structural gate.
//! - **Empty-noise tokens** — informationally-empty single-utterance
//!   responses that clear the length floor (`"Okay."`, `"Sure."`,
//!   `"Hmm."`, …). Small local models surprisingly often emit one of
//!   these as the entire response when they "give up" without an
//!   explicit refusal phrase, and the routing layer should fall back
//!   rather than serve them.
//! - **Refusal phrases** — a batched Aho-Corasick DFA over a curated
//!   list of refusal openings, scanned against the first
//!   [`REFUSAL_SCAN_WINDOW`] bytes of `trimmed`. The window is a
//!   compromise: position-tolerant enough to catch *"Hmm, I cannot
//!   help with that"* / *"Hello! Unfortunately I can't…"* (which the
//!   previous strict `starts_with` check missed), but tight enough
//!   that a long substantive answer that happens to use `"cannot"` in
//!   the middle isn't spuriously flagged.
//!
//! All patterns are ASCII; the DFA uses
//! [`AhoCorasickBuilder::ascii_case_insensitive`] so we get
//! case-insensitive matching without allocating a lowercased copy of
//! the input. Unicode bytes outside ASCII pass through transparently —
//! AC works on raw bytes, so non-ASCII input never spuriously matches
//! an ASCII pattern.
//!
//! ## Performance
//!
//! The previous implementation allocated a full lowercase `String`
//! copy of every response and then ran ten `starts_with` checks
//! against it. This implementation performs a single batched DFA pass
//! over the first ~200 bytes of the response with zero per-call heap
//! allocation. The DFA itself is compiled once on first use via
//! [`std::sync::LazyLock`].

use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use std::sync::LazyLock;

/// Minimum character count for a response to clear the "non-trivial"
/// length gate.
const MIN_CHARS: usize = 5;

/// Number of bytes from the start of the trimmed response to scan for
/// a refusal phrase. Catches refusals that follow a short polite
/// preamble (e.g. `"Hello! "`, `"Hmm, "`, `"Well, "`) without flagging
/// a long, substantive answer that happens to use `"cannot"` somewhere
/// in the middle. 200 bytes ≈ 30–40 English words, which covers every
/// realistic "lead-in then refuse" pattern observed on small local
/// models.
const REFUSAL_SCAN_WINDOW: usize = 200;

/// Refusal / inability phrases observed from small open-weight models
/// (Llama-3-8B, Phi-3-mini, Gemma-2-2B, Qwen2-7B, Mistral-7B). Order
/// is not significant — the DFA is built with [`MatchKind::LeftmostFirst`]
/// and we only care whether *any* phrase matched.
///
/// All entries are lowercase ASCII; the DFA is built case-insensitive.
const REFUSAL_PHRASES: &[&str] = &[
    // Direct inability
    "i cannot",
    "i can't",
    "i can not",
    "i won't",
    "i won't be able to",
    "i will not",
    "i'm unable to",
    "i am unable to",
    "i'm not able to",
    "i am not able to",
    // `"i'm afraid i can"` by itself is over-broad — it would also
    // match legitimately constrained-but-non-refusing responses such
    // as `"I'm afraid I can only give you three results."`. Pin to
    // the explicit-refusal continuations the surrounding patterns
    // target.
    "i'm afraid i can't",
    "i'm afraid i cannot",
    // Capability disclaimers
    "i don't have the ability",
    "i do not have the ability",
    "i don't have access",
    "i do not have access",
    // Self-identification disclaimers (Llama-family classic)
    "as an ai,",
    "as an ai language",
    "as a language model",
    "i'm just an ai",
    "i'm just a language model",
    // Formal decline
    "i must decline",
    "i have to decline",
    // Apologetic refusals
    "i'm sorry, but i cannot",
    "i'm sorry, but i can't",
    "i apologize, but i cannot",
    "i apologize, but i can't",
    "my apologies, but i cannot",
    "sorry, i cannot",
    "sorry, i can't",
    "unfortunately, i cannot",
    "unfortunately, i can't",
    // Policy framing
    "it's not appropriate for me",
    "it is not appropriate for me",
    "it would not be appropriate",
    "i'm not comfortable",
    "i am not comfortable",
];

/// Informationally-empty single-utterance responses. Flagged as low
/// quality even though they clear [`MIN_CHARS`], because they carry no
/// answer for the user. Matched against the *entire* trimmed response
/// via [`str::eq_ignore_ascii_case`] — no allocation.
///
/// Every entry here must be at least [`MIN_CHARS`] bytes long; shorter
/// tokens (e.g. `"ok."`, `"no."`, `"hmm."`) are already flagged as low
/// quality by the length gate in [`is_low_quality`] before this list is
/// even consulted, so listing them here would be dead config and is
/// guarded against by [`tests::empty_noise_tokens_all_clear_min_chars`].
const EMPTY_NOISE_TOKENS: &[&str] = &[
    "okay.",
    "okay!",
    "sure.",
    "sure!",
    "right.",
    "noted.",
    "got it.",
    "got it!",
    "understood.",
];

/// Compiled DFA over [`REFUSAL_PHRASES`]. Built lazily on first call
/// and reused for the lifetime of the process.
static REFUSAL_DFA: LazyLock<AhoCorasick> = LazyLock::new(|| {
    AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(MatchKind::LeftmostFirst)
        .build(REFUSAL_PHRASES)
        .expect("REFUSAL_PHRASES is a static, valid pattern list")
});

/// Returns `true` when `text` should be treated as low quality and a
/// remote fallback is warranted.
///
/// Cheap, allocation-free, no I/O. Safe to call on the hot path after
/// every local-model inference.
pub fn is_low_quality(text: &str) -> bool {
    let trimmed = text.trim();

    // 1. Structural length gate.
    if trimmed.len() < MIN_CHARS {
        return true;
    }

    // 2. Single-utterance "empty noise" — clears the length gate but
    //    carries no information.
    if is_empty_noise(trimmed) {
        return true;
    }

    // 3. Refusal phrase anywhere in the first REFUSAL_SCAN_WINDOW bytes.
    //    Single batched DFA pass, ASCII-case-insensitive, zero
    //    allocations.
    //
    //    Slicing on a UTF-8 byte boundary is safe here: even if
    //    REFUSAL_SCAN_WINDOW lands mid-codepoint, AC operates on raw
    //    bytes and our ASCII patterns can't match across the cut, so
    //    truncating a trailing multi-byte sequence is harmless.
    let window_end = trimmed.len().min(REFUSAL_SCAN_WINDOW);
    let window = &trimmed.as_bytes()[..window_end];
    REFUSAL_DFA.find(window).is_some()
}

/// Case-insensitive ASCII compare of `trimmed` against the empty-noise
/// list. Uses [`str::eq_ignore_ascii_case`] directly on the borrowed
/// bytes — no allocation. Patterns are all ASCII; non-ASCII inputs
/// simply won't equal any pattern (different byte length / different
/// bytes).
fn is_empty_noise(trimmed: &str) -> bool {
    EMPTY_NOISE_TOKENS
        .iter()
        .any(|token| trimmed.eq_ignore_ascii_case(token))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- length gate ----------

    #[test]
    fn empty_is_low_quality() {
        assert!(is_low_quality(""));
        assert!(is_low_quality("   "));
    }

    #[test]
    fn too_short_is_low_quality() {
        assert!(is_low_quality("ok"));
        assert!(is_low_quality("yes"));
        assert!(is_low_quality("no"));
    }

    #[test]
    fn borderline_length_not_flagged_if_content_ok() {
        // Exactly 5 chars — clears length gate AND not in empty-noise list.
        assert!(!is_low_quality("Hello"));
        // 4 chars — below threshold.
        assert!(is_low_quality("Hi!"));
    }

    // ---------- substantive responses pass ----------

    #[test]
    fn normal_response_is_not_low_quality() {
        assert!(!is_low_quality("The answer is 42."));
        assert!(!is_low_quality("Here is a summary of the article."));
        assert!(!is_low_quality(
            "Sure thing — the meeting is scheduled for tomorrow at 3 PM."
        ));
    }

    // ---------- empty-noise: informationally empty single utterances ----------

    #[test]
    fn empty_noise_tokens_are_low_quality() {
        // These all clear MIN_CHARS=5 but carry no answer for the user.
        assert!(is_low_quality("Okay."));
        assert!(is_low_quality("Sure."));
        assert!(is_low_quality("Hmm."));
        assert!(is_low_quality("Right."));
        assert!(is_low_quality("Noted."));
        assert!(is_low_quality("Got it."));
        assert!(is_low_quality("Understood."));
        // Case variants and trailing/leading whitespace.
        assert!(is_low_quality("  okay.  "));
        assert!(is_low_quality("OKAY."));
        assert!(is_low_quality("Sure!"));
    }

    #[test]
    fn empty_noise_does_not_swallow_real_answers_that_start_similarly() {
        // "Okay, …" continued with content is a real answer.
        assert!(!is_low_quality("Okay, here's the answer: 42."));
        assert!(!is_low_quality("Sure thing, I scheduled it for 3 PM."));
        assert!(!is_low_quality(
            "Got it — sending the file to the team now."
        ));
    }

    // ---------- refusal phrases: existing coverage ----------

    #[test]
    fn legacy_refusal_prefixes_are_low_quality() {
        // Every phrase from the previous implementation must still match.
        assert!(is_low_quality("I cannot help with that."));
        assert!(is_low_quality("I can't do that."));
        assert!(is_low_quality("I'm unable to process this request."));
        assert!(is_low_quality("I am unable to assist."));
        assert!(is_low_quality("As an AI, I don't have opinions."));
        assert!(is_low_quality("As an AI language model, I cannot..."));
        assert!(is_low_quality(
            "I don't have the ability to browse the web."
        ));
        assert!(is_low_quality("I'm sorry, but I cannot comply."));
        assert!(is_low_quality("I apologize, but I cannot do that."));
        assert!(is_low_quality("Sorry, I cannot assist with that."));
    }

    #[test]
    fn refusal_check_is_case_insensitive() {
        // Builder uses ascii_case_insensitive(true) — no lowercase allocation needed.
        assert!(is_low_quality("I CANNOT help with that."));
        assert!(is_low_quality("I CAN'T do that."));
        assert!(is_low_quality("AS AN AI LANGUAGE MODEL, ..."));
        assert!(is_low_quality("Unfortunately, I CANNOT comply."));
    }

    // ---------- refusal phrases: new coverage (Llama-3 / Phi / Gemma / Qwen) ----------

    #[test]
    fn extended_refusal_phrases_are_caught() {
        // Direct inability variants the previous list missed.
        assert!(is_low_quality("I can not perform that task."));
        assert!(is_low_quality("I won't be able to help with this."));
        assert!(is_low_quality("I will not comply with that."));
        assert!(is_low_quality("I'm not able to access that file."));
        assert!(is_low_quality("I am not able to fetch live data."));
        assert!(is_low_quality("I'm afraid I can't disclose that."));
        // The tighter `"i'm afraid i can't"` / `"i'm afraid i cannot"`
        // form must still catch the explicit-refusal cases above
        // without flagging legitimately constrained-but-not-refusing
        // responses — see `i_am_afraid_pattern_does_not_flag_constrained_but_not_refusing`.
        assert!(is_low_quality("I'm afraid I cannot proceed."));
        // Capability disclaimers.
        assert!(is_low_quality("I don't have access to the internet."));
        assert!(is_low_quality("I do not have the ability to execute code."));
        // Llama-family self-identification.
        assert!(is_low_quality(
            "As a language model, I cannot predict the future."
        ));
        assert!(is_low_quality("I'm just an AI, I don't have feelings."));
        assert!(is_low_quality("I'm just a language model trained by ..."));
        // Formal decline.
        assert!(is_low_quality("I must decline that request."));
        assert!(is_low_quality("I have to decline this one."));
        // Additional apologetic openings.
        assert!(is_low_quality("My apologies, but I cannot answer that."));
        assert!(is_low_quality("Unfortunately, I can't help with that."));
        // Policy framing.
        assert!(is_low_quality(
            "It's not appropriate for me to comment on that."
        ));
        assert!(is_low_quality(
            "It would not be appropriate to share that information."
        ));
        assert!(is_low_quality(
            "I'm not comfortable answering that question."
        ));
    }

    // ---------- position-tolerance: refusals after a short preamble ----------

    #[test]
    fn refusal_after_short_preamble_is_caught() {
        // The previous `starts_with` check missed all of these because the
        // refusal phrase wasn't at byte 0. The DFA window catches them.
        assert!(is_low_quality("Hmm, I cannot help with that."));
        assert!(is_low_quality(
            "Hello! Unfortunately, I can't process this."
        ));
        assert!(is_low_quality("Well, I'm afraid I can't answer that one."));
        assert!(is_low_quality(
            "Thanks for asking. As an AI, I don't have personal opinions."
        ));
    }

    #[test]
    fn refusal_word_in_middle_of_long_answer_is_not_flagged() {
        // A genuine, substantive answer that uses "cannot" / "can't" deep
        // in the response (past REFUSAL_SCAN_WINDOW) must not be
        // misclassified as a refusal. This is the false-positive guard.
        let preamble = "The schedule for the conference is as follows: the keynote starts at \
            9 AM in the main hall, followed by three parallel tracks running through lunch. \
            After the afternoon break we have the closing panel, and a networking reception \
            in the lobby starting around 6 PM. ";
        // Sanity: preamble is longer than the scan window.
        assert!(preamble.len() > REFUSAL_SCAN_WINDOW);
        let response =
            format!("{preamble}I cannot recall the exact end time off the top of my head.");
        assert!(!is_low_quality(&response));
    }

    // ---------- Unicode / non-ASCII safety ----------

    #[test]
    fn non_ascii_input_does_not_panic_or_falsely_match() {
        // Cyrillic, Polish diacritics, emoji — none of these bytes overlap
        // with any ASCII refusal phrase, so they must not match. They also
        // must not panic the DFA, the slicing, or the trim path.
        assert!(!is_low_quality("Это нормальный ответ на ваш вопрос."));
        assert!(!is_low_quality(
            "Oczywiście — spotkanie jest jutro o 15:00."
        ));
        assert!(!is_low_quality("✅ Done! The deployment is live now."));
        // Multi-byte prefix followed by a refusal still inside the window:
        // the AC engine searches the raw byte slice, so the refusal phrase
        // (which is ASCII) is still found.
        assert!(is_low_quality("🤔 Hmm, I cannot help with that request."));
    }

    #[test]
    fn refusal_scan_window_truncation_does_not_panic_on_codepoint_boundary() {
        // Construct a response where byte REFUSAL_SCAN_WINDOW lands inside
        // a multi-byte UTF-8 sequence. We slice on bytes, not chars, so
        // this must remain safe.
        let mut padding = "x".repeat(REFUSAL_SCAN_WINDOW - 1);
        padding.push('ł'); // 'ł' is 2 bytes; cut lands inside it.
        let input = format!("{padding} I cannot help.");
        // Result is whatever it is — we just need to not panic.
        let _ = is_low_quality(&input);
    }

    // ---------- DFA construction smoke test ----------

    #[test]
    fn refusal_dfa_compiles_and_has_expected_pattern_count() {
        // Force LazyLock initialization. If REFUSAL_PHRASES ever contains
        // a malformed entry, this is where it'll surface — not in
        // production at the first call site.
        let dfa = &*REFUSAL_DFA;
        assert_eq!(dfa.patterns_len(), REFUSAL_PHRASES.len());
    }

    // ---------- false-positive guard for over-broad patterns ----------

    #[test]
    fn i_am_afraid_pattern_does_not_flag_constrained_but_not_refusing() {
        // `"I'm afraid I can only give you three results."` is a
        // legitimately constrained-but-not-refusing answer. The
        // earlier broader `"i'm afraid i can"` phrase would have
        // matched this and triggered a remote fallback unnecessarily.
        // The tightened `"i'm afraid i can't"` / `"i'm afraid i cannot"`
        // pair must catch only the explicit-refusal continuations.
        assert!(!is_low_quality(
            "I'm afraid I can only give you three results."
        ));
        assert!(!is_low_quality(
            "I'm afraid I can offer you a partial summary at best."
        ));

        // Sanity: the explicit-refusal continuations must still fire.
        assert!(is_low_quality("I'm afraid I can't help with that."));
        assert!(is_low_quality("I'm afraid I cannot share that."));
    }

    // ---------- dead-config guard ----------

    #[test]
    fn empty_noise_tokens_all_clear_min_chars() {
        // Any EMPTY_NOISE_TOKENS entry shorter than MIN_CHARS is dead
        // config: the length gate in `is_low_quality` returns `true`
        // before `is_empty_noise` is ever consulted, so the token is
        // unreachable. Removing or adding tokens is fine — adding a
        // short one without intending to is a bug, and this test
        // catches it at CI time instead of letting it ship as silent
        // dead config.
        for token in EMPTY_NOISE_TOKENS {
            assert!(
                token.len() >= MIN_CHARS,
                "EMPTY_NOISE_TOKENS entry {token:?} is shorter than MIN_CHARS={MIN_CHARS}; \
                 it would be unreachable behind the length gate. Either lengthen the token, \
                 lower MIN_CHARS, or drop the entry."
            );
        }
    }
}
