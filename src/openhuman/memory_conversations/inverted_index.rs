//! In-memory inverted index for `search_cross_thread_messages`.
//!
//! ## Architecture (v1)
//!
//! ```text
//!                    ┌───────────────┐
//!  query "cat"  ───▶ │  Phase 1:     │  posting-list intersection
//!                    │  ngram lookup │  on character n-grams
//!                    └──────┬────────┘
//!                           │ candidate doc ids per term
//!                    ┌──────▼────────┐
//!                    │  Phase 2:     │  exact substring verify on
//!                    │  verify+score │  normalized content, score
//!                    └──────┬────────┘  by `matched_terms / total_terms`
//!                           │
//!                           ▼
//!                       Vec<Hit>
//! ```
//!
//! ## Ownership choices for scale
//!
//! Conversation corpora can grow to hundreds of thousands of messages per
//! workspace. The data structures here are picked to keep the resident-set
//! size predictable at that scale:
//!
//! - **`thread_id` and `role` are interned `Arc<str>`** (see
//!   `intern_thread_id` / `intern_role`). A workspace with one thread of
//!   N messages would otherwise store N copies of the same thread id; a
//!   role string only ever takes two distinct values in practice. The
//!   interner amortises both to a single heap allocation per distinct
//!   value, plus one `Arc` clone per `DocEntry`.
//! - **Posting-map keys are `Box<str>`** (16 bytes) rather than `String`
//!   (24 bytes). Saves 8 bytes per distinct ngram in the corpus — at
//!   ~17k Latin trigrams plus CJK bigrams that adds up.
//! - **Posting lists are still `BTreeSet<u32>`** for ergonomic ordered
//!   iteration. The Phase 1 intersection is performed against a
//!   single-allocation `Vec<u32>` accumulator via a two-pointer
//!   sort-merge (no per-iteration `BTreeSet` rebuilds), so the BTreeSet
//!   shape only affects insertion and removal, not query latency.
//!   Roaring Bitmaps + FST + LSM segments are the long-term destination
//!   (Gemini Deep Research write-up); we defer that until corpus sizes
//!   justify the complexity.
//! - **Whole index lives in RAM**, rebuilt from JSONL on first access in
//!   the process. The JSONL files remain the source of truth.
//! - **Scoring matches the previous linear scan**:
//!   `score = matched_terms / total_terms` with a `created_at` tiebreaker.
//! - **Pathological query short-circuit**: if Phase 1 produces a
//!   candidate set larger than `LARGE_CANDIDATE_LIMIT` for any term, the
//!   index returns recency-ordered hits without running Phase 2. This
//!   genuinely caps tail latency — the check fires *before* the
//!   substring-verification loop, not after.

use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use super::tokenize::{ngrams, normalize};
use super::types::{ConversationMessage, CrossThreadHit};

/// Minimum byte length for a query term to be considered. Matches the
/// historical behaviour of `search_cross_thread_messages` so existing
/// callers (and tests) see no change. Single-byte ASCII tokens like "a"
/// or "is" are filtered out; a single CJK character (3 bytes in UTF-8)
/// passes through.
const MIN_TERM_BYTES: usize = 3;

/// When Phase 1 returns more than this many candidates we skip Phase 2
/// verification and fall back to a pure recency-ranked truncation. This
/// is the mitigation for the "user types `e`" pathological case.
const LARGE_CANDIDATE_LIMIT: usize = 10_000;

/// One indexed message. Carries enough state to (a) reconstruct a
/// `CrossThreadHit` without re-reading JSONL on the hot path and (b)
/// verify Phase 1 candidates by exact substring match on the normalized
/// form.
///
/// `thread_id` and `role` are `Arc<str>` because they repeat heavily
/// across messages (N messages per thread → N references to the same
/// thread id; only ~2 distinct role values across the entire corpus).
/// `message_id`, `content`, `content_normalized` and `created_at` are
/// per-message unique so they stay as `String`.
#[derive(Debug, Clone)]
struct DocEntry {
    thread_id: Arc<str>,
    message_id: String,
    role: Arc<str>,
    content: String,            // original, returned verbatim in hits
    content_normalized: String, // for Phase 2 substring verification
    created_at: String,
}

/// In-memory trigram/bigram inverted index over conversation messages.
///
/// Documents are addressed by a dense `u32` doc-id assigned in insertion
/// order. Deletes leave tombstones (`docs[i] = None`) rather than shifting
/// the array, so posting-list integers stay valid without rebuilding.
#[derive(Debug, Default)]
pub(crate) struct InvertedIndex {
    /// `ngram -> sorted set of doc-ids`. BTreeSet so per-doc removals
    /// are O(log n) and iteration is in sorted order (drives the
    /// sort-merge intersect in `candidates_for_term`). Keys are
    /// `Box<str>` to shave 8 bytes per entry vs `String`.
    postings: HashMap<Box<str>, BTreeSet<u32>>,
    /// Tombstoned: `docs[i] == None` means the message was deleted. We
    /// keep the slot so existing doc-ids in posting lists stay valid.
    docs: Vec<Option<DocEntry>>,
    /// Reverse lookup for incremental removal: `(thread_id, message_id)`
    /// → `doc_id`. Letting us drop a single message without re-walking
    /// the corpus.
    by_message: HashMap<(String, String), u32>,
    /// Interner pools. Keep a single `Arc<str>` per distinct thread id
    /// and role so every `DocEntry` referencing them can hold a cheap
    /// 16-byte `Arc` clone instead of a 24-byte `String`.
    thread_id_pool: HashMap<String, Arc<str>>,
    role_pool: HashMap<String, Arc<str>>,
}

impl InvertedIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert one message. Takes the message by value so the caller's
    /// owned strings can be moved into the index without an internal
    /// clone of each field. If the (thread, message_id) pair is already
    /// in the index this is a no-op (messages are append-only in the
    /// store, so duplicate IDs indicate a corrupt JSONL — silently
    /// ignore rather than panic).
    pub fn insert(&mut self, thread_id: &str, msg: ConversationMessage) {
        let ConversationMessage {
            id,
            content,
            sender,
            created_at,
            message_type: _,
            extra_metadata: _,
        } = msg;

        let key = (thread_id.to_string(), id.clone());
        if self.by_message.contains_key(&key) {
            return;
        }
        let normalized = normalize(&content);
        let doc_id = self.docs.len() as u32;
        for ngram in ngrams(&normalized) {
            if let Some(posting) = self.postings.get_mut(ngram) {
                posting.insert(doc_id);
            } else {
                let mut set = BTreeSet::new();
                set.insert(doc_id);
                self.postings.insert(ngram.into(), set);
            }
        }
        let thread_arc = self.intern_thread_id(thread_id);
        let role_arc = self.intern_role(&sender);
        self.docs.push(Some(DocEntry {
            thread_id: thread_arc,
            message_id: id,
            role: role_arc,
            content,
            content_normalized: normalized,
            created_at,
        }));
        self.by_message.insert(key, doc_id);
    }

    /// Drop every document belonging to a thread. Used by
    /// `delete_thread` and during full purge.
    pub fn remove_thread(&mut self, thread_id: &str) {
        let to_remove: Vec<u32> = self
            .by_message
            .iter()
            .filter(|((t, _), _)| t == thread_id)
            .map(|(_, id)| *id)
            .collect();
        for doc_id in to_remove {
            self.remove_doc(doc_id);
        }
        self.thread_id_pool.remove(thread_id);
    }

    /// Reset the index to its empty state. Cheaper than dropping and
    /// re-allocating when a workspace is being rebuilt.
    pub fn clear(&mut self) {
        self.postings.clear();
        self.docs.clear();
        self.by_message.clear();
        self.thread_id_pool.clear();
        self.role_pool.clear();
    }

    fn remove_doc(&mut self, doc_id: u32) {
        let idx = doc_id as usize;
        let Some(entry) = self.docs.get_mut(idx).and_then(|slot| slot.take()) else {
            return;
        };
        self.by_message
            .remove(&(entry.thread_id.to_string(), entry.message_id.clone()));
        // Remove doc_id from every posting list referencing it. We re-
        // tokenize the normalized content rather than tracking the
        // per-doc ngram set; tokenization is allocation-free now that
        // `ngrams` returns borrowed slices, so this stays cheap.
        for ngram in ngrams(&entry.content_normalized) {
            if let Some(posting) = self.postings.get_mut(ngram) {
                posting.remove(&doc_id);
                if posting.is_empty() {
                    self.postings.remove(ngram);
                }
            }
        }
    }

    /// The Phase 1 + Phase 2 query pipeline. Mirrors the contract of
    /// `ConversationStore::search_cross_thread_messages` so the store
    /// method can be a thin shim.
    pub fn search(
        &self,
        query: &str,
        limit: usize,
        exclude_thread_id: Option<&str>,
    ) -> Vec<CrossThreadHit> {
        if limit == 0 {
            return Vec::new();
        }
        let query_lower = normalize(query);
        // Filter terms by raw byte length (matches the historical
        // 3-byte threshold; single CJK chars are 3 bytes and pass).
        let terms: Vec<String> = query_lower
            .split_whitespace()
            .filter(|t| t.len() >= MIN_TERM_BYTES)
            .map(|s| s.to_string())
            .collect();
        if terms.is_empty() {
            return Vec::new();
        }

        // Phase 1: collect candidate doc-ids per term. Short-circuit to
        // recency-only ordering if any single term's candidate set
        // already exceeds the pathological threshold — this is the cap
        // on tail latency, and it must fire BEFORE we run the substring
        // verification loop.
        let mut per_term: Vec<Vec<u32>> = Vec::with_capacity(terms.len());
        for term in &terms {
            let candidates = match self.candidates_for_term(term) {
                Some(v) => v,
                None => self
                    .docs
                    .iter()
                    .enumerate()
                    .filter_map(|(i, slot)| slot.as_ref().map(|_| i as u32))
                    .collect::<Vec<u32>>(),
            };
            if candidates.len() > LARGE_CANDIDATE_LIMIT {
                return self.recency_fallback(exclude_thread_id, limit);
            }
            per_term.push(candidates);
        }

        // Phase 2: verify each candidate by exact substring match.
        // Count distinct terms per doc for the score.
        let mut hit_counts: HashMap<u32, usize> = HashMap::new();
        for (term, candidates) in terms.iter().zip(per_term.into_iter()) {
            for doc_id in candidates {
                let Some(entry) = self.docs[doc_id as usize].as_ref() else {
                    continue;
                };
                if exclude_thread_id == Some(entry.thread_id.as_ref()) {
                    continue;
                }
                if entry.content_normalized.contains(term.as_str()) {
                    *hit_counts.entry(doc_id).or_insert(0) += 1;
                }
            }
        }

        let total_terms = terms.len() as f64;
        let mut hits: Vec<CrossThreadHit> = hit_counts
            .into_iter()
            .map(|(doc_id, matched)| {
                let entry = self.docs[doc_id as usize]
                    .as_ref()
                    .expect("doc_id from hit_counts must be live");
                CrossThreadHit {
                    thread_id: entry.thread_id.to_string(),
                    message_id: entry.message_id.clone(),
                    role: entry.role.to_string(),
                    content: entry.content.clone(),
                    created_at: entry.created_at.clone(),
                    score: matched as f64 / total_terms,
                }
            })
            .collect();

        hits.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| b.created_at.cmp(&a.created_at))
        });
        hits.truncate(limit);
        hits
    }

    /// Build the Phase 1 candidate set for one query term.
    ///
    /// Returns `Some(vec)` containing the sorted intersection of posting
    /// lists for every ngram of `term`. If `term` is too short to
    /// produce any ngram (e.g. a single CJK char of length 1) returns
    /// `None` so the caller can fall back to a linear scan.
    ///
    /// The intersect is a two-pointer sort-merge over the already-sorted
    /// posting lists: `acc` is rewritten in place once per remaining
    /// ngram, allocating zero intermediate sets.
    fn candidates_for_term(&self, term: &str) -> Option<Vec<u32>> {
        let term_ngrams = ngrams(term);
        if term_ngrams.is_empty() {
            return None;
        }
        let mut iter = term_ngrams.iter();
        let first = iter.next().expect("non-empty by check above");
        let mut acc: Vec<u32> = match self.postings.get(*first) {
            Some(p) => p.iter().copied().collect(),
            None => return Some(Vec::new()),
        };
        for ng in iter {
            if acc.is_empty() {
                return Some(acc);
            }
            match self.postings.get(*ng) {
                Some(p) => intersect_sorted_with_btreeset(&mut acc, p),
                None => return Some(Vec::new()),
            }
        }
        Some(acc)
    }

    fn intern_thread_id(&mut self, thread_id: &str) -> Arc<str> {
        if let Some(existing) = self.thread_id_pool.get(thread_id) {
            return Arc::clone(existing);
        }
        let arc: Arc<str> = Arc::from(thread_id);
        self.thread_id_pool
            .insert(thread_id.to_string(), Arc::clone(&arc));
        arc
    }

    fn intern_role(&mut self, role: &str) -> Arc<str> {
        if let Some(existing) = self.role_pool.get(role) {
            return Arc::clone(existing);
        }
        let arc: Arc<str> = Arc::from(role);
        self.role_pool.insert(role.to_string(), Arc::clone(&arc));
        arc
    }

    fn recency_fallback(
        &self,
        exclude_thread_id: Option<&str>,
        limit: usize,
    ) -> Vec<CrossThreadHit> {
        let mut hits: Vec<CrossThreadHit> = self
            .docs
            .iter()
            .filter_map(|slot| slot.as_ref())
            .filter(|entry| exclude_thread_id != Some(entry.thread_id.as_ref()))
            .map(|entry| CrossThreadHit {
                thread_id: entry.thread_id.to_string(),
                message_id: entry.message_id.clone(),
                role: entry.role.to_string(),
                content: entry.content.clone(),
                created_at: entry.created_at.clone(),
                // Score 0.0 signals "matched via recency fallback only" —
                // documented in the function rustdoc above. Callers
                // sorting by `(score desc, created_at desc)` still see
                // the newest entries first.
                score: 0.0,
            })
            .collect();
        hits.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        hits.truncate(limit);
        hits
    }
}

/// Two-pointer sort-merge intersect. `acc` and `other` are both sorted
/// ascending; on return `acc` contains only the elements present in
/// both, in the same sorted order. Runs in O(|acc| + |other|) with zero
/// allocations.
fn intersect_sorted_with_btreeset(acc: &mut Vec<u32>, other: &BTreeSet<u32>) {
    let mut other_iter = other.iter().copied().peekable();
    let mut write = 0usize;
    for read in 0..acc.len() {
        let target = acc[read];
        // Advance `other_iter` past everything strictly less than the
        // current `target`. After this loop the next peeked value is
        // either equal to `target` (keep) or strictly greater (drop).
        while let Some(&o) = other_iter.peek() {
            if o < target {
                other_iter.next();
            } else {
                break;
            }
        }
        if other_iter.peek().copied() == Some(target) {
            acc[write] = target;
            write += 1;
            other_iter.next();
        }
    }
    acc.truncate(write);
}

#[cfg(test)]
mod tests {
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
}
