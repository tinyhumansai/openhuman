//! User profile learning hook.
//!
//! Extracts user preferences from conversation turns using a curated
//! list of fixed-string opening phrases (e.g. *"I prefer…"*,
//! *"always use…"*, *"my timezone is…"*) compiled into a single
//! Aho-Corasick DFA, and stores matched sentences in the
//! `user_profile` memory category. The hook runs on every user turn
//! via [`PostTurnHook::on_turn_complete`], so the match path is
//! deliberately allocation-free.
//!
//! ## Why Aho-Corasick instead of `.contains()` per pattern
//!
//! The previous implementation lower-cased the entire user message
//! once, lower-cased each sentence again inside a loop, and then ran
//! every pattern through `str::contains` — for a 5-sentence message
//! that was 6 `String` allocations plus 5 × N substring scans per
//! turn. The current implementation builds one
//! [`AhoCorasick`] DFA at first use with
//! [`AhoCorasickBuilder::ascii_case_insensitive`] enabled, then runs a
//! single byte-level pass per sentence. Zero per-call allocation,
//! linear-time scan, and the same pattern source-of-truth.
//!
//! ## Word boundaries
//!
//! Each candidate match is accepted only if **both** of the following
//! hold:
//!
//! 1. The byte immediately after the match end is non-alphanumeric
//!    ASCII (whitespace, punctuation, or the leading byte of a
//!    multi-byte UTF-8 sequence) — so `"I preferred X"` does **not**
//!    match the `"i prefer"` phrase.
//! 2. There is at least one further byte of content past that boundary
//!    — so empty-tail fragments like `"I prefer"` (the residue of
//!    splitting `"I prefer."` on `.`) or dangling `"I prefer:"` are
//!    rejected. These carry no preference target and would otherwise
//!    pollute `user_profile` memory with useless slugs.
//!
//! Together this catches `"I prefer:X"`, `"I prefer-X"`,
//! `"I prefer X"` and `"I prefer\nX"` while filtering out the
//! degenerate empty-tail cases. As a consequence the previous
//! post-loop fallback (which only existed to rescue the
//! `"i prefer<punct>"` shape) is no longer needed and was removed.

use crate::openhuman::agent::hooks::{PostTurnHook, TurnContext};
use crate::openhuman::config::LearningConfig;
use crate::openhuman::memory::{Memory, MemoryCategory};
use aho_corasick::{AhoCorasick, AhoCorasickBuilder, MatchKind};
use async_trait::async_trait;
use std::sync::{Arc, LazyLock};

/// Sentence delimiters used to split a user message into candidate
/// preference statements. Includes `?` and `;` (which the previous
/// implementation missed) so that *"What's your view? I prefer Rust."*
/// and *"OK; I prefer Rust."* are both decomposed correctly.
///
/// `:` is intentionally **not** a delimiter: *"My role: engineer"* is
/// best treated as a single statement so the `"my role"` phrase can
/// match against it.
const SENTENCE_DELIMITERS: &[char] = &['.', '!', '?', ';', '\n'];

/// Minimum byte length of a sentence to be considered for preference
/// extraction. The shortest pattern (`"i like"`, `"i want"`, …) is six
/// bytes; anything below eight bytes can't carry a pattern plus a
/// trailing target token, so we'd just be matching noise.
const MIN_SENTENCE_BYTES: usize = 8;

/// Maximum number of preferences emitted from a single user message —
/// guards memory writes from a runaway "list of 50 prefs" prompt.
const MAX_PREFERENCES_PER_TURN: usize = 5;

/// Curated opening phrases that signal an explicit user preference.
///
/// All entries are lowercase ASCII; the DFA is built case-insensitive
/// so we never need to lowercase the input. Each phrase is matched
/// with a trailing word-boundary check (see [`sentence_has_preference`]),
/// so trailing whitespace is **not** part of the pattern itself.
///
/// Categories (informational; the DFA is unordered):
///
/// * **Direct preference / inclination** — `"i prefer"`, `"i'd prefer"`,
///   `"i would prefer"`, `"i'd rather"`, `"i like"`, `"i dislike"`,
///   `"i don't like"`, `"i want"`, `"i need"`.
/// * **Habit / instruction** — `"i always"`, `"always use"`,
///   `"never use"`, `"please always"`, `"please never"`, `"please use"`,
///   `"from now on"`, `"going forward"`.
/// * **Identity / context** — `"my name is"`, `"i am a"`, `"i'm a"`,
///   `"i work"`, `"my role"`, `"my stack"`, `"my timezone"`,
///   `"my language"`, `"my pronouns"`, `"my preferred"`, `"call me"`,
///   `"address me as"`.
const PREFERENCE_PATTERNS: &[&str] = &[
    // Direct preference / inclination
    "i prefer",
    "i'd prefer",
    "i would prefer",
    "i'd rather",
    "i like",
    "i dislike",
    "i don't like",
    "i want",
    "i need",
    // Habit / instruction
    "i always",
    "always use",
    "never use",
    "please always",
    "please never",
    "please use",
    "from now on",
    "going forward",
    // Identity / context
    "my name is",
    "i am a",
    "i'm a",
    "i work",
    "my role",
    "my stack",
    "my timezone",
    "my language",
    "my pronouns",
    "my preferred",
    "call me",
    "address me as",
];

/// Compiled DFA over [`PREFERENCE_PATTERNS`]. Built lazily on first
/// call and reused for the lifetime of the process.
static PREFERENCE_DFA: LazyLock<AhoCorasick> = LazyLock::new(|| {
    AhoCorasickBuilder::new()
        .ascii_case_insensitive(true)
        .match_kind(MatchKind::LeftmostFirst)
        .build(PREFERENCE_PATTERNS)
        .expect("PREFERENCE_PATTERNS is a static, valid pattern list")
});

/// Returns `true` if `sentence` contains a preference opening phrase
/// followed by a word-boundary byte **and at least one byte of trailing
/// content**. Zero allocations.
///
/// End-of-sentence (`bytes.get(m.end()) == None`) is intentionally
/// **rejected**: a sentence that consists of nothing but the opening
/// phrase carries no preference target (e.g. `"I prefer"` after
/// splitting `"I prefer."` on `.`). Storing it would just pollute
/// `user_profile` memory with a slug that resolves to "I prefer". The
/// caller in [`UserProfileHook::extract_preferences`] depends on this
/// behaviour to filter the empty-tail case without a second pass.
fn sentence_has_preference(sentence: &str) -> bool {
    let bytes = sentence.as_bytes();
    PREFERENCE_DFA.find_iter(bytes).any(|m| {
        // End-of-sentence — no trailing content for the pattern to
        // qualify, so this is not a useful preference signal.
        let Some(b) = bytes.get(m.end()) else {
            return false;
        };
        // Any non-ASCII-alphanumeric byte is a valid boundary —
        // including the leading byte of a multi-byte UTF-8 sequence
        // (always >= 0x80 and therefore not alphanumeric). We then
        // require at least one further byte of content past the
        // boundary so we don't store fragments like `"I prefer:"`
        // either.
        !b.is_ascii_alphanumeric() && bytes.get(m.end() + 1).is_some()
    })
}

/// Post-turn hook that extracts user preferences from conversations.
pub struct UserProfileHook {
    config: LearningConfig,
    memory: Arc<dyn Memory>,
}

impl UserProfileHook {
    pub fn new(config: LearningConfig, memory: Arc<dyn Memory>) -> Self {
        Self { config, memory }
    }

    /// Extract preference statements from the user message.
    ///
    /// Splits on [`SENTENCE_DELIMITERS`], filters sentences below
    /// [`MIN_SENTENCE_BYTES`], and accepts any sentence where the
    /// Aho-Corasick DFA finds a preference phrase followed by a
    /// word boundary. Output is capped at [`MAX_PREFERENCES_PER_TURN`]
    /// entries. Allocation-free until a match is pushed onto `found`.
    fn extract_preferences(message: &str) -> Vec<String> {
        let mut found = Vec::new();

        for sentence in message.split(SENTENCE_DELIMITERS) {
            let trimmed = sentence.trim();
            if trimmed.len() < MIN_SENTENCE_BYTES {
                continue;
            }
            if sentence_has_preference(trimmed) {
                found.push(trimmed.to_string());
                if found.len() >= MAX_PREFERENCES_PER_TURN {
                    break;
                }
            }
        }

        found
    }

    /// Store extracted preferences in memory, deduplicating by slug.
    async fn store_preferences(&self, preferences: &[String]) -> anyhow::Result<()> {
        for pref in preferences {
            let slug = slugify(pref);
            if slug.is_empty() {
                continue;
            }
            let key = format!("pref/{slug}");

            // Check for existing entry to avoid duplicates
            if let Ok(Some(_)) = self.memory.get("user_profile", &key).await {
                log::debug!("[learning] user preference already stored: {key}");
                continue;
            }

            self.memory
                .store(
                    "user_profile",
                    &key,
                    pref,
                    MemoryCategory::Custom("user_profile".into()),
                    None,
                )
                .await?;
            log::info!("[learning] stored user preference: {key}");
        }
        Ok(())
    }
}

#[async_trait]
impl PostTurnHook for UserProfileHook {
    fn name(&self) -> &str {
        "user_profile"
    }

    async fn on_turn_complete(&self, ctx: &TurnContext) -> anyhow::Result<()> {
        if !self.config.enabled || !self.config.user_profile_enabled {
            return Ok(());
        }

        let preferences = Self::extract_preferences(&ctx.user_message);
        if preferences.is_empty() {
            return Ok(());
        }

        log::debug!(
            "[learning] extracted {} preference(s) from user message",
            preferences.len()
        );
        self.store_preferences(&preferences).await
    }
}

fn slugify(s: &str) -> String {
    s.chars()
        .filter_map(|c| {
            if c.is_alphanumeric() {
                Some(c.to_ascii_lowercase())
            } else if c == ' ' || c == '-' || c == '_' {
                Some('_')
            } else {
                None
            }
        })
        .take(40)
        .collect()
}

#[cfg(test)]
#[path = "user_profile_tests.rs"]
mod tests;
