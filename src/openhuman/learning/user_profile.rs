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
mod tests {
    use super::*;
    use crate::openhuman::agent::hooks::TurnContext;
    use crate::openhuman::memory::{Memory, MemoryCategory, MemoryEntry};
    use async_trait::async_trait;
    use parking_lot::Mutex;
    use std::collections::HashMap;
    use std::sync::Arc;

    #[derive(Default)]
    struct MockMemory {
        entries: Mutex<HashMap<String, MemoryEntry>>,
    }

    #[async_trait]
    impl Memory for MockMemory {
        fn name(&self) -> &str {
            "mock"
        }

        async fn store(
            &self,
            namespace: &str,
            key: &str,
            content: &str,
            category: MemoryCategory,
            session_id: Option<&str>,
        ) -> anyhow::Result<()> {
            self.entries.lock().insert(
                key.to_string(),
                MemoryEntry {
                    id: key.to_string(),
                    key: key.to_string(),
                    content: content.to_string(),
                    namespace: Some(namespace.to_string()),
                    category,
                    timestamp: "now".into(),
                    session_id: session_id.map(str::to_string),
                    score: None,
                },
            );
            Ok(())
        }

        async fn recall(
            &self,
            _query: &str,
            _limit: usize,
            _opts: crate::openhuman::memory::RecallOpts<'_>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(Vec::new())
        }

        async fn get(&self, _namespace: &str, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
            Ok(self.entries.lock().get(key).cloned())
        }

        async fn list(
            &self,
            _namespace: Option<&str>,
            _category: Option<&MemoryCategory>,
            _session_id: Option<&str>,
        ) -> anyhow::Result<Vec<MemoryEntry>> {
            Ok(self.entries.lock().values().cloned().collect())
        }

        async fn forget(&self, _namespace: &str, key: &str) -> anyhow::Result<bool> {
            Ok(self.entries.lock().remove(key).is_some())
        }

        async fn namespace_summaries(
            &self,
        ) -> anyhow::Result<Vec<crate::openhuman::memory::NamespaceSummary>> {
            Ok(Vec::new())
        }

        async fn count(&self) -> anyhow::Result<usize> {
            Ok(self.entries.lock().len())
        }

        async fn health_check(&self) -> bool {
            true
        }
    }

    #[test]
    fn extract_preferences_finds_patterns() {
        let msg = "I prefer Rust over Python. Always use snake_case for variables.";
        let prefs = UserProfileHook::extract_preferences(msg);
        assert_eq!(prefs.len(), 2);
        assert!(prefs[0].contains("prefer"));
        assert!(prefs[1].contains("snake_case"));
    }

    #[test]
    fn extract_preferences_ignores_short_sentences() {
        let msg = "I prefer. OK.";
        let prefs = UserProfileHook::extract_preferences(msg);
        assert!(prefs.is_empty());
    }

    #[test]
    fn extract_preferences_handles_no_matches() {
        let msg = "Can you help me debug this function?";
        let prefs = UserProfileHook::extract_preferences(msg);
        assert!(prefs.is_empty());
    }

    #[test]
    fn extract_preferences_handles_single_sentence_message() {
        // No sentence delimiter — the whole message is one sentence,
        // matched by the DFA. The previous implementation needed a
        // dedicated post-loop fallback for this case; with the
        // word-boundary check inside `sentence_has_preference` the
        // main path handles it directly.
        let prefs = UserProfileHook::extract_preferences("I prefer compact diffs in code reviews");
        assert_eq!(prefs, vec!["I prefer compact diffs in code reviews"]);
    }

    #[test]
    fn extract_preferences_caps_at_max_per_turn() {
        // Message contains seven preference statements; cap is
        // MAX_PREFERENCES_PER_TURN (5).
        let many = UserProfileHook::extract_preferences(
            "I prefer Rust. I always use tests. Please always explain failures. \
             My timezone is PST. My stack is Tauri. Going forward use concise output. \
             Never use nested bullets.",
        );
        assert_eq!(many.len(), MAX_PREFERENCES_PER_TURN);
    }

    // ---------- word-boundary correctness ----------

    #[test]
    fn extract_preferences_word_boundary_rejects_alphanumeric_continuation() {
        // "I preferred" must NOT match `"i prefer"` — the byte after
        // the match end is alphanumeric, so it's a continuation of the
        // word, not a boundary. Previously this would have matched
        // via `str::contains` because the substring `"i prefer"` is
        // literally present in `"i preferred"`.
        let prefs =
            UserProfileHook::extract_preferences("I preferred to wait but it was ultimately fine.");
        assert!(prefs.is_empty(), "got: {prefs:?}");

        // Similarly for "I needed" against "i need", "I wanted"
        // against "i want".
        let prefs2 = UserProfileHook::extract_preferences("I needed coffee. I wanted snacks.");
        assert!(prefs2.is_empty(), "got: {prefs2:?}");
    }

    #[test]
    fn extract_preferences_word_boundary_accepts_non_alphanumeric_continuation() {
        // Punctuation directly after a pattern still counts as a
        // boundary, so `"I prefer:something"` matches. This is the
        // recovered capability from the previous implementation,
        // which only caught this case via the special-purpose
        // post-loop fallback that has now been removed.
        assert_eq!(
            UserProfileHook::extract_preferences("I prefer:Rust"),
            vec!["I prefer:Rust"]
        );
        assert_eq!(
            UserProfileHook::extract_preferences("I prefer-compact diffs"),
            vec!["I prefer-compact diffs"]
        );
    }

    #[test]
    fn extract_preferences_rejects_bare_pattern_with_no_content_after() {
        // Sentences where the pattern runs to the end with no target
        // word carry no useful preference signal and must be dropped.
        // `"I prefer."` after splitting on `.` becomes the sentence
        // `"I prefer"` — pattern match reaches end-of-sentence with
        // no content after it, so the boundary check returns false.
        for noise in [
            "I prefer.",
            "Sometimes I prefer.",
            "I always! Whatever.",
            "I want.",
        ] {
            let prefs = UserProfileHook::extract_preferences(noise);
            assert!(
                prefs.is_empty(),
                "noise {noise:?} unexpectedly produced {prefs:?}"
            );
        }
    }

    // ---------- expanded sentence-delimiter set ----------

    #[test]
    fn extract_preferences_splits_on_question_mark_and_semicolon() {
        // The previous splitter only split on `.`/`!`/`\n`. A leading
        // question or list-style preamble used to bleed into the
        // preference sentence and either swallow context or miss the
        // match entirely. `?` and `;` are now delimiters; `:` is
        // intentionally not (so `"My role: engineer"` stays as one
        // sentence the `"my role"` pattern can match).
        let q = UserProfileHook::extract_preferences(
            "What's the timezone situation? My timezone is PST.",
        );
        assert_eq!(q.len(), 1);
        assert!(q[0].contains("My timezone"));

        let s = UserProfileHook::extract_preferences("OK; I prefer Rust over Python.");
        assert_eq!(s.len(), 1);
        assert!(s[0].contains("I prefer Rust"));
    }

    // ---------- expanded pattern coverage ----------

    #[test]
    fn extract_preferences_catches_extended_patterns() {
        // Each new pattern category gets one minimal trigger so any
        // future drop is loud at CI time.
        let cases = [
            (
                "I'd prefer concise responses",
                "I'd prefer concise responses",
            ),
            (
                "I would prefer not to repeat myself",
                "I would prefer not to repeat myself",
            ),
            (
                "I'd rather skip the boilerplate",
                "I'd rather skip the boilerplate",
            ),
            (
                "I dislike verbose explanations",
                "I dislike verbose explanations",
            ),
            (
                "Please use snake_case in variables",
                "Please use snake_case in variables",
            ),
            ("Call me Alex from now on", "Call me Alex from now on"),
            ("Address me as Dr. Smith", "Address me as Dr"),
            ("My pronouns are they/them", "My pronouns are they/them"),
            (
                "My preferred editor is Helix",
                "My preferred editor is Helix",
            ),
        ];
        for (msg, expected_substr) in cases {
            let prefs = UserProfileHook::extract_preferences(msg);
            assert!(
                prefs.iter().any(|p| p.contains(expected_substr)),
                "input {msg:?} should yield {expected_substr:?}, got {prefs:?}"
            );
        }
    }

    // ---------- Unicode / non-ASCII safety ----------

    #[test]
    fn extract_preferences_non_ascii_does_not_panic_or_falsely_match() {
        // Cyrillic / Polish diacritics / emoji must not match any
        // ASCII pattern, must not panic the DFA, and must not break
        // the byte-level word-boundary check (the leading byte of a
        // multi-byte UTF-8 sequence is >= 0x80 and therefore not
        // ASCII-alphanumeric, so it correctly counts as a boundary).
        assert!(
            UserProfileHook::extract_preferences("Это нормальное сообщение без предпочтений.")
                .is_empty()
        );
        assert!(UserProfileHook::extract_preferences(
            "Oczywiście — żadnej preferencji tutaj nie ma."
        )
        .is_empty());

        // Multi-byte prefix followed by a real preference must still match.
        let mixed =
            UserProfileHook::extract_preferences("🤔 I prefer compact diffs in code reviews.");
        assert_eq!(mixed.len(), 1);
        assert!(mixed[0].contains("I prefer compact diffs"));
    }

    // ---------- DFA construction smoke test ----------

    #[test]
    fn preference_dfa_compiles_and_has_expected_pattern_count() {
        // Force LazyLock initialization. If PREFERENCE_PATTERNS ever
        // contains a malformed entry, this is where it surfaces — not
        // in production at the first call site. Also catches a typo
        // that silently swallows an entry from the patterns slice.
        let dfa = &*PREFERENCE_DFA;
        assert_eq!(dfa.patterns_len(), PREFERENCE_PATTERNS.len());
    }

    #[tokio::test]
    async fn store_preferences_skips_duplicates_and_empty_slugs() {
        let memory_impl = Arc::new(MockMemory::default());
        memory_impl
            .store(
                "user_profile",
                "pref/i_prefer_rust",
                "I prefer Rust",
                MemoryCategory::Custom("user_profile".into()),
                None,
            )
            .await
            .unwrap();
        let memory: Arc<dyn Memory> = memory_impl.clone();
        let hook = UserProfileHook::new(
            LearningConfig {
                enabled: true,
                user_profile_enabled: true,
                ..LearningConfig::default()
            },
            memory,
        );

        hook.store_preferences(&[
            "I prefer Rust".into(),
            "!!!".into(),
            "My timezone is PST".into(),
        ])
        .await
        .unwrap();

        let keys: Vec<String> = memory_impl.entries.lock().keys().cloned().collect();
        assert_eq!(keys.len(), 2);
        assert!(keys.contains(&"pref/i_prefer_rust".into()));
        assert!(keys.contains(&"pref/my_timezone_is_pst".into()));
    }

    #[tokio::test]
    async fn on_turn_complete_respects_feature_flags_and_stores_preferences() {
        let memory_impl = Arc::new(MockMemory::default());
        let memory: Arc<dyn Memory> = memory_impl.clone();
        let ctx = TurnContext {
            user_message: "My language is English. Please always use concise output.".into(),
            assistant_response: "Noted".into(),
            tool_calls: Vec::new(),
            turn_duration_ms: 10,
            session_id: None,
            agent_id: None,
            entrypoint: None,
            iteration_count: 1,
        };

        let disabled = UserProfileHook::new(LearningConfig::default(), memory.clone());
        disabled.on_turn_complete(&ctx).await.unwrap();
        assert!(memory_impl.entries.lock().is_empty());

        let enabled = UserProfileHook::new(
            LearningConfig {
                enabled: true,
                user_profile_enabled: true,
                ..LearningConfig::default()
            },
            memory,
        );
        enabled.on_turn_complete(&ctx).await.unwrap();

        let values: Vec<String> = memory_impl
            .entries
            .lock()
            .values()
            .map(|entry| entry.content.clone())
            .collect();
        assert!(values
            .iter()
            .any(|value| value.contains("My language is English")));
        assert!(values
            .iter()
            .any(|value| value.contains("Please always use concise output")));
    }
}
