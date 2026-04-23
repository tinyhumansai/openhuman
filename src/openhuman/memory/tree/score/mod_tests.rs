use super::*;
use crate::openhuman::memory::tree::types::{chunk_id, Chunk, Metadata, SourceKind};
use chrono::Utc;

fn test_chunk(content: &str) -> Chunk {
    let meta = Metadata::point_in_time(SourceKind::Email, "t1", "alice", Utc::now());
    Chunk {
        id: chunk_id(SourceKind::Email, "t1", 0),
        content: content.to_string(),
        token_count: crate::openhuman::memory::tree::types::approx_token_count(content),
        metadata: meta,
        seq_in_source: 0,
        created_at: Utc::now(),
    }
}

#[tokio::test]
async fn substantive_chunk_is_kept() {
    let c = test_chunk(
        "We decided to ship Phoenix on Friday after reviewing \
         alice@example.com and the migration plan carefully. \
         @bob will coordinate and we discussed #launch-q2 details.",
    );
    let cfg = ScoringConfig::default_regex_only();
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert!(r.kept, "expected kept, got total={}", r.total);
    assert!(r.drop_reason.is_none());
    assert!(!r.extracted.entities.is_empty());
    assert!(!r.canonical_entities.is_empty());
}

#[tokio::test]
async fn noise_chunk_is_dropped() {
    // Very short — below TOKEN_MIN — and no entities.
    let c = test_chunk("lol");
    let cfg = ScoringConfig::default_regex_only();
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert!(!r.kept);
    assert!(r.drop_reason.is_some());
}

#[tokio::test]
async fn threshold_override_respected() {
    let c = test_chunk("just ok content, mid-signal");
    let mut cfg = ScoringConfig::default_regex_only();
    cfg.drop_threshold = 0.99; // unreasonably high
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert!(!r.kept);
}

#[tokio::test]
async fn entities_are_canonicalised() {
    let c = test_chunk("ping Alice@Example.com — she @alice replied to thread");
    let cfg = ScoringConfig::default_regex_only();
    let r = score_chunk(&c, &cfg).await.unwrap();
    // Email (lowercased) and handle canonical ids should both appear
    let ids: Vec<_> = r
        .canonical_entities
        .iter()
        .map(|e| e.canonical_id.as_str())
        .collect();
    assert!(ids.iter().any(|id| *id == "email:alice@example.com"));
    assert!(ids.iter().any(|id| *id == "handle:alice"));
}

// ── Short-circuit / LLM-extractor tests ─────────────────────────────

/// Test extractor that returns a fixed importance value and records call count.
struct FakeLlm {
    importance: f32,
    call_count: std::sync::atomic::AtomicUsize,
}

impl FakeLlm {
    fn new(importance: f32) -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self {
            importance,
            call_count: std::sync::atomic::AtomicUsize::new(0),
        })
    }
    fn calls(&self) -> usize {
        self.call_count.load(std::sync::atomic::Ordering::Relaxed)
    }
}

#[async_trait::async_trait]
impl extract::EntityExtractor for FakeLlm {
    fn name(&self) -> &'static str {
        "fake-llm"
    }
    async fn extract(&self, _text: &str) -> Result<extract::ExtractedEntities> {
        self.call_count
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(extract::ExtractedEntities {
            entities: vec![],
            topics: vec![],
            llm_importance: Some(self.importance),
            llm_importance_reason: Some("fake".into()),
        })
    }
}

#[tokio::test]
async fn short_circuit_skips_llm_when_cheap_total_is_definite_keep() {
    // A substantive chunk with high cheap-total should bypass the LLM.
    let c = test_chunk(
        "We decided to ship Phoenix on Friday after reviewing alice@example.com and \
         the migration plan carefully. @bob will coordinate and we discussed \
         #launch-q2 details extensively in the email thread.",
    );
    let llm = FakeLlm::new(0.5);
    let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
    // Force the cheap total well above the keep threshold by lowering
    // the keep threshold so this test is robust to weight tuning.
    cfg.definite_keep_threshold = 0.10;
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert!(r.kept);
    assert_eq!(llm.calls(), 0, "LLM should not be consulted");
    // signals.llm_importance stays at 0 (no LLM call happened)
    assert_eq!(r.signals.llm_importance, 0.0);
}

#[tokio::test]
async fn short_circuit_skips_llm_when_cheap_total_is_definite_drop() {
    // A noisy chunk with very low cheap total should bypass the LLM
    // and be dropped.
    let c = test_chunk("ok");
    let llm = FakeLlm::new(0.99);
    let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
    // Force the cheap total to look like definite_drop.
    cfg.definite_drop_threshold = 0.99;
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert!(!r.kept);
    assert_eq!(
        llm.calls(),
        0,
        "LLM should not be consulted on definite_drop"
    );
}

#[tokio::test]
async fn borderline_chunk_consults_llm() {
    // Pick content that will land in the borderline band and verify the LLM
    // gets called. Use generous band edges so the test isn't sensitive
    // to weight nudges.
    let c = test_chunk("This is a moderately interesting note about a project.");
    let llm = FakeLlm::new(0.9);
    let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
    cfg.definite_drop_threshold = 0.0;
    cfg.definite_keep_threshold = 1.0;
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert_eq!(llm.calls(), 1, "LLM should be consulted exactly once");
    assert!(r.signals.llm_importance > 0.0);
    assert_eq!(r.extracted.llm_importance_reason.as_deref(), Some("fake"));
}

#[tokio::test]
async fn llm_failure_falls_back_gracefully() {
    struct FailingLlm;
    #[async_trait::async_trait]
    impl extract::EntityExtractor for FailingLlm {
        fn name(&self) -> &'static str {
            "failing-llm"
        }
        async fn extract(&self, _text: &str) -> Result<extract::ExtractedEntities> {
            Err(anyhow::anyhow!("simulated failure"))
        }
    }
    let c = test_chunk("This is a moderately interesting note about a project.");
    let mut cfg = ScoringConfig::with_llm_extractor(std::sync::Arc::new(FailingLlm));
    cfg.definite_drop_threshold = 0.0;
    cfg.definite_keep_threshold = 1.0;
    // Should not error out; should produce a result based on cheap signals only.
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert_eq!(r.signals.llm_importance, 0.0);
}

/// When LLM is skipped (short-circuit or failure), the reported `total`
/// must equal `combine_cheap_only(signals, weights)` — not the
/// LLM-weighted `combine` (which would drag `llm_importance=0` through
/// a 2.0 weight and artificially lower the total).
#[tokio::test]
async fn short_circuit_reports_cheap_only_total() {
    let c = test_chunk(
        "We decided to ship Phoenix on Friday after reviewing alice@example.com and \
         the migration plan carefully. @bob will coordinate and we discussed \
         #launch-q2 details extensively in the email thread.",
    );
    let llm = FakeLlm::new(0.99);
    let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
    cfg.definite_keep_threshold = 0.10; // force short-circuit keep
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert_eq!(llm.calls(), 0);
    let expected = self::signals::combine_cheap_only(&r.signals, &cfg.weights);
    assert!(
        (r.total - expected).abs() < 1e-6,
        "total={} expected(cheap_only)={}",
        r.total,
        expected
    );
    // And explicitly NOT the full combine (which would include a 0-value
    // llm_importance term in a 0..1-clamped weighted average, dragging
    // the total down).
    let with_llm = self::signals::combine(&r.signals, &cfg.weights);
    assert!(
        r.total > with_llm,
        "cheap-only total ({}) should exceed LLM-weighted total \
         ({}) when llm_importance is zero",
        r.total,
        with_llm
    );
}

/// When the LLM *does* run, the reported total uses the full combine —
/// the llm_importance contribution is actually in the sum.
#[tokio::test]
async fn llm_consulted_reports_full_total() {
    let c = test_chunk("This is a moderately interesting note about a project.");
    let llm = FakeLlm::new(0.9);
    let mut cfg = ScoringConfig::with_llm_extractor(llm.clone());
    cfg.definite_drop_threshold = 0.0;
    cfg.definite_keep_threshold = 1.0;
    let r = score_chunk(&c, &cfg).await.unwrap();
    assert_eq!(llm.calls(), 1);
    let expected = self::signals::combine(&r.signals, &cfg.weights);
    assert!(
        (r.total - expected).abs() < 1e-6,
        "total={} expected(full combine)={}",
        r.total,
        expected
    );
}
