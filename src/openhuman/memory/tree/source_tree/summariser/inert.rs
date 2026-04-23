//! Deterministic fallback summariser (#709).
//!
//! `InertSummariser` concatenates each input's content, separated by a
//! blank line, and hard-truncates to `ctx.token_budget`. Entities and
//! topics are **intentionally empty**: per design, summary-level entity /
//! topic metadata is derived by the LLM summariser from the summary's own
//! synthesised content (not by mechanically unioning children's labels).
//! Until the networked summariser lands, inert-sealed summaries have no
//! entity index rows — an honest stub. The goal of this fallback is not
//! metadata fidelity; it's a stable, dependency-free baseline so tree
//! mechanics (sealing, cascade, roots) can be tested without an LLM.

use anyhow::Result;
use async_trait::async_trait;

use crate::openhuman::memory::tree::source_tree::summariser::{
    Summariser, SummaryContext, SummaryInput, SummaryOutput,
};
use crate::openhuman::memory::tree::types::approx_token_count;

/// Default prefix applied to each contribution in the joined body. Keeps
/// provenance visible to a human reading the raw summary.
const PROVENANCE_PREFIX: &str = "— ";

pub struct InertSummariser;

impl InertSummariser {
    pub fn new() -> Self {
        Self
    }
}

impl Default for InertSummariser {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Summariser for InertSummariser {
    async fn summarise(
        &self,
        inputs: &[SummaryInput],
        ctx: &SummaryContext<'_>,
    ) -> Result<SummaryOutput> {
        let mut parts: Vec<String> = Vec::with_capacity(inputs.len());
        for inp in inputs {
            let trimmed = inp.content.trim();
            if trimmed.is_empty() {
                continue;
            }
            parts.push(format!("{}{}", PROVENANCE_PREFIX, trimmed));
        }
        let joined = parts.join("\n\n");

        let (content, token_count) = truncate_to_budget(&joined, ctx.token_budget);

        log::debug!(
            "[source_tree::summariser::inert] sealed tree_id={} level={} inputs={} tokens={} \
             entities=0 topics=0 (honest stub — LLM summariser derives these)",
            ctx.tree_id,
            ctx.target_level,
            inputs.len(),
            token_count
        );

        Ok(SummaryOutput {
            content,
            token_count,
            entities: Vec::new(),
            topics: Vec::new(),
        })
    }
}

/// Truncate `text` to fit within `budget` approximate tokens. Returns the
/// (possibly truncated) body and its recomputed token count. Truncation is
/// done on character boundaries — `approx_token_count` assumes ~4 chars
/// per token so we clamp character length to `budget * 4`.
fn truncate_to_budget(text: &str, budget: u32) -> (String, u32) {
    let initial = approx_token_count(text);
    if initial <= budget {
        return (text.to_string(), initial);
    }
    // Character ceiling derived from the same ~4 chars/token heuristic.
    let char_ceiling = (budget as usize).saturating_mul(4);
    let truncated: String = text.chars().take(char_ceiling).collect();
    let tokens = approx_token_count(&truncated);
    (truncated, tokens)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::source_tree::types::TreeKind;
    use chrono::Utc;

    fn sample_input(id: &str, content: &str, entities: &[&str]) -> SummaryInput {
        let ts = Utc::now();
        SummaryInput {
            id: id.to_string(),
            content: content.to_string(),
            token_count: approx_token_count(content),
            entities: entities.iter().map(|s| s.to_string()).collect(),
            topics: Vec::new(),
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
        }
    }

    fn test_ctx() -> SummaryContext<'static> {
        SummaryContext {
            tree_id: "tree-1",
            tree_kind: TreeKind::Source,
            target_level: 1,
            token_budget: 10_000,
        }
    }

    #[tokio::test]
    async fn concats_inputs_with_provenance_prefix() {
        let s = InertSummariser::default();
        let inputs = vec![
            sample_input("a", "hello world", &[]),
            sample_input("b", "second contribution", &[]),
        ];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.contains(PROVENANCE_PREFIX));
        assert!(out.content.contains("hello world"));
        assert!(out.content.contains("second contribution"));
        assert_eq!(out.token_count, approx_token_count(&out.content));
    }

    #[tokio::test]
    async fn honest_stub_emits_no_entities_or_topics() {
        // Per design: summary-level entities/topics are LLM-derived from
        // the summary's own synthesised content. The inert fallback does
        // not propagate children's labels — it emits empty vecs. The
        // Ollama summariser (future) will fill them via real NER on its
        // own output.
        let s = InertSummariser::default();
        let inputs = vec![
            sample_input("a", "x", &["entity:alice", "entity:bob"]),
            sample_input("b", "y", &["entity:bob", "entity:carol"]),
        ];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.entities.is_empty());
        assert!(out.topics.is_empty());
    }

    #[tokio::test]
    async fn truncates_when_over_budget() {
        let s = InertSummariser::default();
        let long_text = "a".repeat(100);
        let inputs = vec![sample_input("a", &long_text, &[])];
        let mut ctx = test_ctx();
        ctx.token_budget = 5; // way under — should truncate hard
        let out = s.summarise(&inputs, &ctx).await.unwrap();
        assert!(out.token_count <= ctx.token_budget + 1);
        assert!(out.content.len() < long_text.len() + PROVENANCE_PREFIX.len());
    }

    #[tokio::test]
    async fn skips_empty_contributions() {
        let s = InertSummariser::default();
        let inputs = vec![
            sample_input("a", "   ", &[]),
            sample_input("b", "kept", &[]),
        ];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.contains("kept"));
        // exactly one provenance prefix should appear
        assert_eq!(out.content.matches(PROVENANCE_PREFIX).count(), 1);
    }
}
