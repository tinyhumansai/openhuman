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
                taint: Default::default(),
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
    let q =
        UserProfileHook::extract_preferences("What's the timezone situation? My timezone is PST.");
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
    assert!(
        UserProfileHook::extract_preferences("Oczywiście — żadnej preferencji tutaj nie ma.")
            .is_empty()
    );

    // Multi-byte prefix followed by a real preference must still match.
    let mixed = UserProfileHook::extract_preferences("🤔 I prefer compact diffs in code reviews.");
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
