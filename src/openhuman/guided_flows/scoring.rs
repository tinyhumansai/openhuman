//! Tag-based recommendation scoring engine.
//!
//! Replicates Octane AI's core mechanic: quiz answers accumulate weighted
//! tags into a user profile vector, then products are ranked by dot-product
//! similarity against that profile. Hard constraints filter before scoring.
//!
//! ## Log prefix
//!
//! `[guided-flows-scoring]`

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{debug, info};

/// A tag weight mapping: tag_name → weight (0.0–1.0).
pub type TagVector = HashMap<String, f64>;

/// Maps a choice answer to the tags it contributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChoiceTagMapping {
    pub choice: String,
    pub tags: TagVector,
}

/// A product/item in the catalog with feature tags.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogItem {
    pub id: String,
    pub name: String,
    pub description: String,
    pub tags: TagVector,
    /// Hard constraints: item is excluded if user profile has any of these tags.
    #[serde(default)]
    pub exclude_if: Vec<String>,
    /// Hard constraints: item requires ALL of these tags in user profile.
    #[serde(default)]
    pub require_tags: Vec<String>,
    #[serde(default)]
    pub metadata: HashMap<String, serde_json::Value>,
}

/// A scored recommendation result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredItem {
    pub item_id: String,
    pub item_name: String,
    pub score: f64,
    /// Normalized score in [0, 1].
    pub normalized_score: f64,
    /// Which tags contributed most to this score.
    pub top_contributing_tags: Vec<(String, f64)>,
    /// Why this item was recommended (human-readable).
    pub explanation: String,
}

/// Conversion event for tracking which recommendations led to actions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConversionEvent {
    pub session_id: String,
    pub item_id: String,
    pub action: ConversionAction,
    pub timestamp: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ConversionAction {
    Viewed,
    Clicked,
    Accepted,
    Dismissed,
}

/// Accumulate tags from a user's answer into their profile vector.
///
/// If the answer matches a choice in the mapping, the corresponding tags
/// are added (summed) into the profile. Weights accumulate across questions.
pub fn accumulate_tags(profile: &mut TagVector, answer: &str, mappings: &[ChoiceTagMapping]) {
    let lower = answer.to_lowercase();
    for mapping in mappings {
        if mapping.choice.to_lowercase() == lower {
            for (tag, weight) in &mapping.tags {
                *profile.entry(tag.clone()).or_insert(0.0) += weight;
            }
            debug!(
                choice = %answer,
                tags_added = mapping.tags.len(),
                "[guided-flows-scoring] tags accumulated"
            );
            return;
        }
    }
    debug!(choice = %answer, "[guided-flows-scoring] no tag mapping found for choice");
}

/// Score a catalog item against a user profile using dot product.
///
/// Returns the raw dot product score. Higher = better match.
pub fn dot_product_score(profile: &TagVector, item: &CatalogItem) -> f64 {
    let mut score = 0.0;
    for (tag, profile_weight) in profile {
        if let Some(item_weight) = item.tags.get(tag) {
            score += profile_weight * item_weight;
        }
    }
    score
}

/// Compute cosine similarity between user profile and item tag vector.
///
/// Returns value in [-1, 1] where 1 = perfect match.
pub fn cosine_similarity(profile: &TagVector, item_tags: &TagVector) -> f64 {
    let mut dot = 0.0;
    let mut norm_a = 0.0;
    let mut norm_b = 0.0;

    for (tag, w) in profile {
        norm_a += w * w;
        if let Some(iw) = item_tags.get(tag) {
            dot += w * iw;
        }
    }
    for (_, w) in item_tags {
        norm_b += w * w;
    }

    let denom = norm_a.sqrt() * norm_b.sqrt();
    if denom == 0.0 {
        return 0.0;
    }
    dot / denom
}

/// Check hard constraints: returns true if item passes all constraints.
fn passes_constraints(profile: &TagVector, item: &CatalogItem) -> bool {
    // Exclude if user has any excluded tag.
    for excluded_tag in &item.exclude_if {
        if profile.contains_key(excluded_tag) && profile[excluded_tag] > 0.0 {
            return false;
        }
    }
    // Require all required tags.
    for required_tag in &item.require_tags {
        if !profile.contains_key(required_tag) || profile[required_tag] <= 0.0 {
            return false;
        }
    }
    true
}

/// Rank catalog items against a user profile.
///
/// 1. Filter by hard constraints (exclude_if, require_tags).
/// 2. Score remaining items by dot product.
/// 3. Normalize scores to [0, 1].
/// 4. Sort descending by score.
/// 5. Return top_n results with explanations.
pub fn rank_items(profile: &TagVector, catalog: &[CatalogItem], top_n: usize) -> Vec<ScoredItem> {
    let mut scored: Vec<(usize, f64)> = Vec::new();

    for (idx, item) in catalog.iter().enumerate() {
        if !passes_constraints(profile, item) {
            continue;
        }
        let score = dot_product_score(profile, item);
        scored.push((idx, score));
    }

    // Normalize scores.
    let max_score = scored
        .iter()
        .map(|(_, s)| *s)
        .fold(f64::NEG_INFINITY, f64::max);
    let min_score = scored.iter().map(|(_, s)| *s).fold(f64::INFINITY, f64::min);
    let range = max_score - min_score;

    // Sort descending.
    scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    scored.truncate(top_n);

    let results: Vec<ScoredItem> = scored
        .into_iter()
        .map(|(idx, raw_score)| {
            let item = &catalog[idx];
            let normalized = if range > 0.0 {
                (raw_score - min_score) / range
            } else if max_score > 0.0 {
                1.0
            } else {
                0.0
            };

            // Find top contributing tags.
            let mut contributions: Vec<(String, f64)> = profile
                .iter()
                .filter_map(|(tag, pw)| item.tags.get(tag).map(|iw| (tag.clone(), pw * iw)))
                .filter(|(_, c)| *c > 0.0)
                .collect();
            contributions
                .sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
            contributions.truncate(3);

            let explanation = if contributions.is_empty() {
                "General match based on profile".into()
            } else {
                let reasons: Vec<String> = contributions
                    .iter()
                    .map(|(tag, _)| tag.replace('_', " "))
                    .collect();
                format!("Matches your preferences: {}", reasons.join(", "))
            };

            ScoredItem {
                item_id: item.id.clone(),
                item_name: item.name.clone(),
                score: raw_score,
                normalized_score: normalized,
                top_contributing_tags: contributions,
                explanation,
            }
        })
        .collect();

    info!(
        candidates = catalog.len(),
        after_filter = results.len(),
        "[guided-flows-scoring] ranking complete"
    );
    results
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_catalog() -> Vec<CatalogItem> {
        vec![
            CatalogItem {
                id: "whisper-local".into(),
                name: "Local Whisper STT".into(),
                description: "On-device speech recognition".into(),
                tags: HashMap::from([
                    ("privacy".into(), 1.0),
                    ("local".into(), 1.0),
                    ("voice".into(), 0.8),
                    ("high_end".into(), 0.6),
                ]),
                exclude_if: vec![],
                require_tags: vec![],
                metadata: HashMap::new(),
            },
            CatalogItem {
                id: "cloud-stt".into(),
                name: "Cloud STT (Deepgram)".into(),
                description: "Cloud-based speech recognition".into(),
                tags: HashMap::from([
                    ("cloud".into(), 1.0),
                    ("voice".into(), 0.9),
                    ("low_latency".into(), 0.8),
                ]),
                exclude_if: vec!["privacy".into()],
                require_tags: vec![],
                metadata: HashMap::new(),
            },
            CatalogItem {
                id: "ollama-local".into(),
                name: "Ollama Local LLM".into(),
                description: "Run LLMs locally".into(),
                tags: HashMap::from([
                    ("privacy".into(), 1.0),
                    ("local".into(), 1.0),
                    ("high_end".into(), 0.9),
                    ("developer".into(), 0.7),
                ]),
                exclude_if: vec![],
                require_tags: vec!["high_end".into()],
                metadata: HashMap::new(),
            },
            CatalogItem {
                id: "piper-tts".into(),
                name: "Piper TTS".into(),
                description: "Fast local text-to-speech".into(),
                tags: HashMap::from([
                    ("voice".into(), 1.0),
                    ("local".into(), 0.9),
                    ("low_end".into(), 0.8),
                ]),
                exclude_if: vec![],
                require_tags: vec![],
                metadata: HashMap::new(),
            },
        ]
    }

    #[test]
    fn accumulate_tags_adds_weights() {
        let mut profile = TagVector::new();
        let mappings = vec![ChoiceTagMapping {
            choice: "Keep everything local".into(),
            tags: HashMap::from([("privacy".into(), 1.0), ("local".into(), 0.9)]),
        }];
        accumulate_tags(&mut profile, "Keep everything local", &mappings);
        assert_eq!(profile["privacy"], 1.0);
        assert_eq!(profile["local"], 0.9);
    }

    #[test]
    fn accumulate_tags_sums_across_questions() {
        let mut profile = TagVector::new();
        let m1 = vec![ChoiceTagMapping {
            choice: "Voice".into(),
            tags: HashMap::from([("voice".into(), 1.0)]),
        }];
        let m2 = vec![ChoiceTagMapping {
            choice: "Meetings".into(),
            tags: HashMap::from([("voice".into(), 0.5), ("meetings".into(), 1.0)]),
        }];
        accumulate_tags(&mut profile, "Voice", &m1);
        accumulate_tags(&mut profile, "Meetings", &m2);
        assert_eq!(profile["voice"], 1.5); // summed
        assert_eq!(profile["meetings"], 1.0);
    }

    #[test]
    fn accumulate_tags_case_insensitive() {
        let mut profile = TagVector::new();
        let mappings = vec![ChoiceTagMapping {
            choice: "High-end".into(),
            tags: HashMap::from([("high_end".into(), 1.0)]),
        }];
        accumulate_tags(&mut profile, "high-end", &mappings);
        assert_eq!(profile["high_end"], 1.0);
    }

    #[test]
    fn accumulate_tags_no_match_does_nothing() {
        let mut profile = TagVector::new();
        let mappings = vec![ChoiceTagMapping {
            choice: "X".into(),
            tags: HashMap::from([("x".into(), 1.0)]),
        }];
        accumulate_tags(&mut profile, "Y", &mappings);
        assert!(profile.is_empty());
    }

    #[test]
    fn dot_product_basic() {
        let profile = HashMap::from([("voice".into(), 1.0), ("privacy".into(), 0.8)]);
        let item = CatalogItem {
            id: "t".into(),
            name: "t".into(),
            description: "t".into(),
            tags: HashMap::from([("voice".into(), 0.9), ("privacy".into(), 1.0)]),
            exclude_if: vec![],
            require_tags: vec![],
            metadata: HashMap::new(),
        };
        let score = dot_product_score(&profile, &item);
        // 1.0*0.9 + 0.8*1.0 = 1.7
        assert!((score - 1.7).abs() < 1e-10);
    }

    #[test]
    fn cosine_similarity_identical_vectors() {
        let a = HashMap::from([("x".into(), 1.0), ("y".into(), 2.0)]);
        let b = HashMap::from([("x".into(), 1.0), ("y".into(), 2.0)]);
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 1.0).abs() < 1e-10);
    }

    #[test]
    fn cosine_similarity_orthogonal() {
        let a = HashMap::from([("x".into(), 1.0)]);
        let b = HashMap::from([("y".into(), 1.0)]);
        let sim = cosine_similarity(&a, &b);
        assert!((sim - 0.0).abs() < 1e-10);
    }

    #[test]
    fn cosine_similarity_empty_returns_zero() {
        let a = TagVector::new();
        let b = HashMap::from([("x".into(), 1.0)]);
        assert_eq!(cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn hard_constraint_exclude_if() {
        let profile = HashMap::from([("privacy".into(), 1.0)]);
        let catalog = sample_catalog();
        // "cloud-stt" has exclude_if: ["privacy"] — should be filtered out.
        let results = rank_items(&profile, &catalog, 10);
        assert!(!results.iter().any(|r| r.item_id == "cloud-stt"));
    }

    #[test]
    fn hard_constraint_require_tags() {
        let profile = HashMap::from([("privacy".into(), 1.0), ("local".into(), 1.0)]);
        // "ollama-local" requires "high_end" — should be filtered out.
        let catalog = sample_catalog();
        let results = rank_items(&profile, &catalog, 10);
        assert!(!results.iter().any(|r| r.item_id == "ollama-local"));
    }

    #[test]
    fn hard_constraint_require_tags_passes() {
        let profile = HashMap::from([
            ("privacy".into(), 1.0),
            ("local".into(), 1.0),
            ("high_end".into(), 1.0),
        ]);
        let catalog = sample_catalog();
        let results = rank_items(&profile, &catalog, 10);
        // Now ollama-local should appear (has high_end requirement met).
        assert!(results.iter().any(|r| r.item_id == "ollama-local"));
    }

    #[test]
    fn rank_items_sorted_descending() {
        let profile = HashMap::from([
            ("privacy".into(), 1.0),
            ("local".into(), 1.0),
            ("voice".into(), 0.5),
            ("high_end".into(), 1.0),
        ]);
        let catalog = sample_catalog();
        let results = rank_items(&profile, &catalog, 10);
        // Scores should be descending.
        for window in results.windows(2) {
            assert!(window[0].score >= window[1].score);
        }
    }

    #[test]
    fn rank_items_top_n_limits() {
        let profile = HashMap::from([("voice".into(), 1.0)]);
        let catalog = sample_catalog();
        let results = rank_items(&profile, &catalog, 2);
        assert!(results.len() <= 2);
    }

    #[test]
    fn rank_items_normalized_scores() {
        let profile = HashMap::from([("voice".into(), 1.0), ("local".into(), 0.5)]);
        let catalog = sample_catalog();
        let results = rank_items(&profile, &catalog, 10);
        // First item should have normalized_score = 1.0 (highest).
        if !results.is_empty() {
            assert!((results[0].normalized_score - 1.0).abs() < 1e-10);
        }
        // All normalized scores should be in [0, 1].
        for r in &results {
            assert!(r.normalized_score >= 0.0 && r.normalized_score <= 1.0);
        }
    }

    #[test]
    fn rank_items_empty_profile() {
        let profile = TagVector::new();
        let catalog = sample_catalog();
        let results = rank_items(&profile, &catalog, 10);
        // All scores should be 0.
        for r in &results {
            assert_eq!(r.score, 0.0);
        }
    }

    #[test]
    fn rank_items_empty_catalog() {
        let profile = HashMap::from([("voice".into(), 1.0)]);
        let results = rank_items(&profile, &[], 10);
        assert!(results.is_empty());
    }

    #[test]
    fn scored_item_has_explanation() {
        let profile = HashMap::from([("voice".into(), 1.0), ("privacy".into(), 0.8)]);
        let catalog = sample_catalog();
        let results = rank_items(&profile, &catalog, 10);
        for r in &results {
            assert!(!r.explanation.is_empty());
        }
    }

    #[test]
    fn top_contributing_tags_limited_to_3() {
        let profile = HashMap::from([
            ("a".into(), 1.0),
            ("b".into(), 1.0),
            ("c".into(), 1.0),
            ("d".into(), 1.0),
            ("e".into(), 1.0),
        ]);
        let item = CatalogItem {
            id: "t".into(),
            name: "t".into(),
            description: "t".into(),
            tags: HashMap::from([
                ("a".into(), 1.0),
                ("b".into(), 1.0),
                ("c".into(), 1.0),
                ("d".into(), 1.0),
                ("e".into(), 1.0),
            ]),
            exclude_if: vec![],
            require_tags: vec![],
            metadata: HashMap::new(),
        };
        let results = rank_items(&profile, &[item], 1);
        assert!(results[0].top_contributing_tags.len() <= 3);
    }

    #[test]
    fn conversion_event_serializes() {
        let event = ConversionEvent {
            session_id: "s1".into(),
            item_id: "whisper-local".into(),
            action: ConversionAction::Accepted,
            timestamp: 1700000000,
        };
        let json = serde_json::to_string(&event).unwrap();
        let back: ConversionEvent = serde_json::from_str(&json).unwrap();
        assert_eq!(back.action, ConversionAction::Accepted);
    }
}
