//! Phase 1 + Phase 2 query pipeline tests for the in-memory inverted index.

use serde_json::json;

use super::*;

fn msg(id: &str, content: &str, created: &str) -> ConversationMessage {
    ConversationMessage {
        id: id.to_string(),
        content: content.to_string(),
        message_type: "text".to_string(),
        extra_metadata: json!({}),
        sender: "user".to_string(),
        created_at: created.to_string(),
    }
}

#[test]
fn substring_inside_word_matches() {
    // Canonical substring-inside-word case: querying "cat" must find
    // "concatenate" — a token-boundary tokenizer would miss this.
    let mut idx = InvertedIndex::new();
    idx.insert(
        "t1",
        msg("m1", "concatenate the strings", "2026-04-10T10:00:00Z"),
    );

    let hits = idx.search("cat", 10, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].message_id, "m1");
}

#[test]
fn polish_diacritics_normalized_both_sides() {
    let mut idx = InvertedIndex::new();
    idx.insert(
        "t1",
        msg("m1", "Lecę dziś do Krakowa", "2026-04-10T10:00:00Z"),
    );
    // Query with no diacritics finds content with diacritics.
    let hits = idx.search("krakow", 10, None);
    assert_eq!(hits.len(), 1, "krakow should match Krakowa");
}

#[test]
fn japanese_bigram_match() {
    let mut idx = InvertedIndex::new();
    idx.insert(
        "t1",
        msg("m1", "東京タワーが見える", "2026-04-10T10:00:00Z"),
    );
    let hits = idx.search("東京", 10, None);
    assert_eq!(hits.len(), 1, "two-char CJK query should match");
}

#[test]
fn arabic_harakat_stripped() {
    let mut idx = InvertedIndex::new();
    // "wrote" with full vocalization vs bare consonants.
    idx.insert("t1", msg("m1", "كَتَبَ الطالب", "2026-04-10T10:00:00Z"));
    // The bare-consonant form should still find the vocalized one.
    let hits = idx.search("كتب", 10, None);
    assert_eq!(hits.len(), 1, "harakat stripping should equalize forms");
}

#[test]
fn excludes_active_thread() {
    let mut idx = InvertedIndex::new();
    idx.insert(
        "active",
        msg("ma", "postgres deploy", "2026-04-10T10:00:00Z"),
    );
    idx.insert(
        "other",
        msg("mo", "postgres deploy", "2026-04-10T10:01:00Z"),
    );
    let hits = idx.search("postgres", 10, Some("active"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].thread_id, "other");
}

#[test]
fn empty_query_returns_empty() {
    let mut idx = InvertedIndex::new();
    idx.insert("t", msg("m", "anything here", "2026-04-10T10:00:00Z"));
    assert!(idx.search("", 10, None).is_empty());
}

#[test]
fn short_terms_only_returns_empty() {
    // Mirrors the legacy `search_cross_thread_messages_skips_short_terms`
    // behaviour — terms < 3 bytes are dropped.
    let mut idx = InvertedIndex::new();
    idx.insert("t", msg("m", "Postgres", "2026-04-10T10:00:00Z"));
    assert!(idx.search("a is on", 10, None).is_empty());
}

#[test]
fn limit_zero_returns_empty() {
    let mut idx = InvertedIndex::new();
    idx.insert("t", msg("m", "Postgres", "2026-04-10T10:00:00Z"));
    assert!(idx.search("postgres", 0, None).is_empty());
}

#[test]
fn score_matches_legacy_semantics() {
    // 5-term query, 2 substring matches → score = 0.4.
    let mut idx = InvertedIndex::new();
    idx.insert(
        "t1",
        msg(
            "m1",
            "Remember: my project is called Phoenix and uses Go and PostgreSQL.",
            "2026-04-10T10:00:00Z",
        ),
    );
    let hits = idx.search("What database does my project use", 10, None);
    assert_eq!(hits.len(), 1);
    // "project" + "use" (substring of "uses") → 2 of 5 terms.
    assert!(
        (hits[0].score - 0.4).abs() < 1e-9,
        "score = {}",
        hits[0].score
    );
}

#[test]
fn remove_thread_drops_all_messages() {
    let mut idx = InvertedIndex::new();
    idx.insert("t1", msg("m1", "postgres deploy", "2026-04-10T10:00:00Z"));
    idx.insert("t1", msg("m2", "postgres backup", "2026-04-10T10:01:00Z"));
    idx.insert("t2", msg("m3", "postgres replica", "2026-04-10T10:02:00Z"));
    assert_eq!(idx.search("postgres", 10, None).len(), 3);
    idx.remove_thread("t1");
    let hits = idx.search("postgres", 10, None);
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].thread_id, "t2");
}

#[test]
fn duplicate_message_id_is_idempotent() {
    let mut idx = InvertedIndex::new();
    let m = msg("m1", "postgres deploy", "2026-04-10T10:00:00Z");
    idx.insert("t1", m.clone());
    idx.insert("t1", m); // dup — should be ignored
    let hits = idx.search("postgres", 10, None);
    assert_eq!(hits.len(), 1, "duplicate insert must not duplicate hits");
}

#[test]
fn pathological_query_short_circuits_to_recency() {
    // Build a corpus big enough to trip LARGE_CANDIDATE_LIMIT.
    let mut idx = InvertedIndex::new();
    let n = LARGE_CANDIDATE_LIMIT + 50;
    for i in 0..n {
        // "common" appears in every doc, so the trigram "com" hits
        // all of them.
        let created = format!("2026-04-10T10:{:02}:{:02}Z", i / 60, i % 60);
        idx.insert("bulk", msg(&format!("m{i}"), "common payload", &created));
    }
    let hits = idx.search("common", 5, None);
    assert_eq!(hits.len(), 5, "fallback must still respect limit");
    // Score 0.0 is the recency-fallback marker.
    assert!(hits.iter().all(|h| h.score == 0.0));

    idx.insert("bulk", msg("cjk", "猫だけ", "2026-04-11T10:00:00Z"));
    let exact = idx.search("猫", 5, None);
    assert_eq!(exact.len(), 1);
    assert_eq!(exact[0].message_id, "cjk");
    assert_eq!(exact[0].score, 1.0);
}

#[test]
fn intern_pool_dedupes_repeated_thread_id_and_role() {
    // Smoke-test the Arc<str> interning: many messages on one
    // thread must share a single Arc backing the thread_id.
    let mut idx = InvertedIndex::new();
    for i in 0..5 {
        let ts = format!("2026-04-10T10:00:{:02}Z", i);
        idx.insert("shared-thread", msg(&format!("m{i}"), "payload", &ts));
    }
    assert_eq!(idx.thread_id_pool.len(), 1);
    assert_eq!(idx.role_pool.len(), 1);
    // After removing the thread, the pool entry is dropped too.
    idx.remove_thread("shared-thread");
    assert_eq!(idx.thread_id_pool.len(), 0);
}

#[test]
fn intersect_sorted_with_btreeset_basic() {
    let other: BTreeSet<u32> = [2, 4, 5, 9].into_iter().collect();
    let mut acc: Vec<u32> = vec![1, 2, 3, 5, 7, 9];
    intersect_sorted_with_btreeset(&mut acc, &other);
    assert_eq!(acc, vec![2, 5, 9]);
}

#[test]
fn intersect_sorted_with_btreeset_empty_other() {
    let other: BTreeSet<u32> = BTreeSet::new();
    let mut acc: Vec<u32> = vec![1, 2, 3];
    intersect_sorted_with_btreeset(&mut acc, &other);
    assert!(acc.is_empty());
}

#[test]
fn ranks_by_score_then_recency_before_truncating() {
    // More matches than `limit`, so truncation must keep the top-ranked hits by
    // (score desc, created_at desc) — this pins that ranking still happens
    // before the result set is cut, not after (the rank-before-materialize
    // refactor must stay order-equivalent to the old clone-then-rank path).
    let mut idx = InvertedIndex::new();
    // Both terms → score 1.0, but oldest.
    idx.insert(
        "t1",
        msg("both", "alpha beta gamma", "2026-04-10T10:00:00Z"),
    );
    // One term → score 0.5, newest of the 0.5 group.
    idx.insert("t1", msg("newest", "alpha delta", "2026-04-10T10:03:00Z"));
    // One term → score 0.5, middle.
    idx.insert("t1", msg("middle", "alpha epsilon", "2026-04-10T10:02:00Z"));
    // One term → score 0.5, oldest of the 0.5 group.
    idx.insert("t1", msg("oldest", "beta zeta", "2026-04-10T10:01:00Z"));

    let hits = idx.search("alpha beta", 2, None);
    assert_eq!(hits.len(), 2, "must respect the limit");
    // Highest score wins outright; the recency tiebreak then picks the newest of
    // the equal-score remainder. "middle"/"oldest" are dropped.
    assert_eq!(hits[0].message_id, "both");
    assert!(
        (hits[0].score - 1.0).abs() < 1e-9,
        "score = {}",
        hits[0].score
    );
    assert_eq!(hits[1].message_id, "newest");
    assert!(
        (hits[1].score - 0.5).abs() < 1e-9,
        "score = {}",
        hits[1].score
    );
}

#[test]
fn recency_tiebreak_compares_instants_across_rfc3339_offsets() {
    let mut idx = InvertedIndex::new();
    idx.insert(
        "t1",
        msg("older", "alpha match", "2026-04-10T12:30:00+02:00"),
    );
    idx.insert("t1", msg("newer", "alpha match", "2026-04-10T11:00:00Z"));

    let hits = idx.search("alpha", 2, None);
    assert_eq!(hits[0].message_id, "newer");
    assert_eq!(hits[1].message_id, "older");
}
