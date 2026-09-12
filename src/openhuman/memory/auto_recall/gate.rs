//! The gate in front of Lane C: is this message asking about the user?
//!
//! Deciding that with words costs nothing — no embedding, no store — which is
//! what lets the lane run on every turn without paying on every turn. The rule
//! is deliberately narrow: a message must **own** something (`my`, `me`, `I`,
//! …) **and** ask or request (`who … ?`, `what's …`, `tell me …`, `remind me
//! …`). "Fix my code" owns but does not ask; "what's the weather" asks but owns
//! nothing; neither reaches the store. A false positive costs one bounded tree
//! lookup that injects nothing; a false negative is covered by the standing
//! memory-access instruction, which tells the model to retrieve before it
//! claims something is not stored.
//!
//! The word lists are English. A message in another language passes only if
//! it happens to contain one of these tokens, so non-English users fall back
//! to the instruction-driven path. A fingerprint-similarity gate over a few
//! stock "about me" prototypes would lift that limit at the cost of one embed
//! per candidate turn; it is deferred until real gate logs say the lexical
//! rule misses enough to be worth it.

/// Messages longer than this are pastes, transcripts or briefs, not questions
/// about the user. They never reach the store.
const MAX_MESSAGE_CHARS: usize = 600;

/// First-person ownership or self-reference. Apostrophes are stripped before
/// matching, so `i've` and `ive` both land on the same entry.
const FIRST_PERSON: &[&str] = &["my", "me", "mine", "myself", "i", "im", "ive", "id", "ill"];

/// Words a message may open with before it gets to the point — politeness,
/// greetings, fillers. Skipped before the lead word is read, so "please remind
/// me …" and "hey, what's my …" are seen as the requests they are.
const PREAMBLE: &[&str] = &[
    "please", "pls", "plz", "hey", "hi", "hello", "yo", "ok", "okay", "so", "and", "also", "now",
    "then", "just", "quick", "quickly", "question", "btw", "bro", "buddy", "mate", "dude", "man",
    "hmm", "um", "uh", "well", "actually",
];

/// A leading word that makes the message a question or a request. Terminal
/// `?` is the other way in.
const LEADS: &[&str] = &[
    "who", "whos", "what", "whats", "which", "why", "when", "where", "how", "hows", "do", "does",
    "did", "am", "is", "are", "was", "were", "can", "could", "would", "should", "have", "has",
    "tell", "remind", "recall", "remember", "describe", "list", "name",
];

/// What the gate decided, and why. The reason is a stable slug for the
/// `[auto_recall]` log line and for tests.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum GateDecision {
    /// Run the lookup.
    Open(&'static str),
    /// Skip it.
    Closed(&'static str),
}

impl GateDecision {
    /// Whether the lookup runs.
    pub fn is_open(self) -> bool {
        matches!(self, Self::Open(_))
    }

    /// The slug behind the decision.
    pub fn reason(self) -> &'static str {
        match self {
            Self::Open(reason) | Self::Closed(reason) => reason,
        }
    }
}

/// Decide whether `message` asks about the user.
pub fn gate_decision(message: &str) -> GateDecision {
    let trimmed = message.trim();
    if trimmed.is_empty() {
        return GateDecision::Closed("empty");
    }
    if trimmed.contains("```") {
        return GateDecision::Closed("code_block");
    }
    if trimmed.chars().count() > MAX_MESSAGE_CHARS {
        return GateDecision::Closed("too_long");
    }

    let words = words_of(trimmed);
    if !words
        .iter()
        .any(|word| FIRST_PERSON.contains(&word.as_str()))
    {
        return GateDecision::Closed("no_first_person");
    }
    let lead = words.iter().find(|word| !PREAMBLE.contains(&word.as_str()));
    let asks = trimmed.ends_with('?') || lead.is_some_and(|word| LEADS.contains(&word.as_str()));
    if !asks {
        return GateDecision::Closed("not_a_question");
    }
    GateDecision::Open("lexical")
}

/// Lower-cased words with apostrophes (straight or curly) removed, so
/// `What’s` becomes `whats` and `I've` becomes `ive`.
fn words_of(text: &str) -> Vec<String> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '\u{2019}'))
        .filter(|word| !word.is_empty())
        .map(|word| {
            word.chars()
                .filter(|c| *c != '\'' && *c != '\u{2019}')
                .collect::<String>()
                .to_lowercase()
        })
        .filter(|word| !word.is_empty())
        .collect()
}

#[cfg(test)]
#[path = "gate_tests.rs"]
mod tests;
