use async_trait::async_trait;

use super::regex;
use super::types::ExtractedEntities;

/// Interface for anything that can read a chunk's text and emit entities.
#[async_trait]
pub trait EntityExtractor: Send + Sync {
    /// Human-readable name for logs and diagnostics.
    fn name(&self) -> &'static str;

    /// Run extraction. Implementations should be idempotent per input.
    async fn extract(&self, text: &str) -> anyhow::Result<ExtractedEntities>;
}

/// Synchronous regex extractor adapted to the async [`EntityExtractor`] trait.
pub struct RegexEntityExtractor;

#[async_trait]
impl EntityExtractor for RegexEntityExtractor {
    fn name(&self) -> &'static str {
        "regex"
    }

    async fn extract(&self, text: &str) -> anyhow::Result<ExtractedEntities> {
        Ok(regex::extract(text))
    }
}

/// Runs a sequence of extractors and merges their results.
///
/// An extractor returning an error is logged and skipped — one bad extractor
/// does not abort ingestion.
pub struct CompositeExtractor {
    inner: Vec<Box<dyn EntityExtractor>>,
}

impl CompositeExtractor {
    pub fn new(inner: Vec<Box<dyn EntityExtractor>>) -> Self {
        Self { inner }
    }

    /// Convenience constructor: regex-only (the Phase 2 default).
    pub fn regex_only() -> Self {
        Self::new(vec![Box::new(RegexEntityExtractor)])
    }
}

#[async_trait]
impl EntityExtractor for CompositeExtractor {
    fn name(&self) -> &'static str {
        "composite"
    }

    async fn extract(&self, text: &str) -> anyhow::Result<ExtractedEntities> {
        let mut out = ExtractedEntities::default();
        for ex in &self.inner {
            match ex.extract(text).await {
                Ok(batch) => out.merge(batch),
                Err(e) => {
                    log::warn!(
                        "[memory_tree::extract] extractor `{}` failed: {e} — continuing",
                        ex.name()
                    );
                }
            }
        }
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::score::extract::EntityKind;

    #[tokio::test]
    async fn regex_only_extractor_works() {
        let c = CompositeExtractor::regex_only();
        let out = c.extract("hi @alice a@b.com #launch").await.unwrap();
        assert!(out.entities.iter().any(|e| e.kind == EntityKind::Handle));
        assert!(out.entities.iter().any(|e| e.kind == EntityKind::Email));
        assert!(out.entities.iter().any(|e| e.kind == EntityKind::Hashtag));
    }

    struct FailingExtractor;
    #[async_trait]
    impl EntityExtractor for FailingExtractor {
        fn name(&self) -> &'static str {
            "failing"
        }
        async fn extract(&self, _: &str) -> anyhow::Result<ExtractedEntities> {
            Err(anyhow::anyhow!("boom"))
        }
    }

    #[tokio::test]
    async fn composite_survives_one_failing_extractor() {
        let c = CompositeExtractor::new(vec![
            Box::new(FailingExtractor),
            Box::new(RegexEntityExtractor),
        ]);
        let out = c.extract("@alice").await.unwrap();
        assert!(out.entities.iter().any(|e| e.kind == EntityKind::Handle));
    }
}
