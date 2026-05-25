//! Memory-tree summariser: fold N items into one parent summary via a
//! single LLM call.
//!
//! This module replaces the previous trait-based summariser ladder
//! (`Summariser` + `InertSummariser` + `LlmSummariser`) with one plain
//! `async fn`. Callers pass inputs + context + config and get back
//! either a [`SummaryOutput`] or an error. Resilience (retry, graceful
//! degradation) is the caller's responsibility — see
//! [`fallback_summary`] for the deterministic concat-and-truncate
//! helper used by seal cascades that must never abort.
//!
//! The structured-facet-extraction side-channel that the old summariser
//! carried has been removed from this layer; facet extraction is the
//! `learning` domain's job and runs independently.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};

use crate::openhuman::config::Config;
use crate::openhuman::memory::chat::{build_chat_provider, ChatPrompt};
use crate::openhuman::memory_store::chunks::types::approx_token_count;
use crate::openhuman::memory_store::trees::types::TreeKind;

/// Hard cap on summariser output length (in approximate tokens). Sized
/// to fit the downstream embedder (`nomic-embed-text-v1.5`, 8192-token
/// input ceiling) with headroom for tokenizer drift.
const MAX_SUMMARY_OUTPUT_TOKENS: u32 = 5_000;

/// Context window assumed for the model. Sized for the cloud
/// summariser's 120k-token window with headroom; used as the divisor
/// in the per-input clamp so the joined prompt body stays under it
/// at upper-level seals where many children fold together.
const NUM_CTX_TOKENS: u32 = 60_000;

/// Tokens reserved for system prompt + envelope overhead + tokenizer
/// drift between our 4-chars/token heuristic and the model's tokenizer.
const OVERHEAD_RESERVE_TOKENS: u32 = 2_048;

/// One contribution being folded — a raw leaf at L0→L1, or a
/// lower-level summary at L_n→L_{n+1}.
#[derive(Clone, Debug)]
pub struct SummaryInput {
    pub id: String,
    pub content: String,
    pub token_count: u32,
    pub entities: Vec<String>,
    pub topics: Vec<String>,
    pub time_range_start: DateTime<Utc>,
    pub time_range_end: DateTime<Utc>,
    pub score: f32,
}

/// Per-seal context — lets logs identify which tree is being sealed
/// without threading config globally.
#[derive(Clone, Debug)]
pub struct SummaryContext<'a> {
    pub tree_id: &'a str,
    pub tree_kind: TreeKind,
    pub target_level: u32,
    pub token_budget: u32,
}

/// Output of a summarise call.
#[derive(Clone, Debug)]
pub struct SummaryOutput {
    pub content: String,
    pub token_count: u32,
    /// Always emitted empty by [`summarise`]. Canonical entity ids are
    /// populated separately by the entity extractor; rolling up children's
    /// labels mechanically is anti-pattern (see prior `InertSummariser`
    /// design note).
    pub entities: Vec<String>,
    pub topics: Vec<String>,
}

/// Fold `inputs` into a single summary by making one chat-provider call.
///
/// Returns `Err` on provider build failure, network failure, or empty
/// upstream response. Callers that must not abort (e.g. seal cascades)
/// should match on the error and fall back to [`fallback_summary`].
pub async fn summarise(
    config: &Config,
    inputs: &[SummaryInput],
    ctx: &SummaryContext<'_>,
) -> Result<SummaryOutput> {
    let effective_budget = ctx.token_budget.min(MAX_SUMMARY_OUTPUT_TOKENS);
    let per_input_cap = if inputs.is_empty() {
        0
    } else {
        NUM_CTX_TOKENS
            .saturating_sub(effective_budget)
            .saturating_sub(OVERHEAD_RESERVE_TOKENS)
            / inputs.len() as u32
    };

    let body = build_user_prompt(inputs, per_input_cap);
    if body.trim().is_empty() {
        return Ok(SummaryOutput {
            content: String::new(),
            token_count: 0,
            entities: Vec::new(),
            topics: Vec::new(),
        });
    }

    let provider =
        build_chat_provider(config).context("memory_tree::summarise: build chat provider")?;

    let prompt = ChatPrompt {
        system: system_prompt(effective_budget, config.output_language.as_deref()),
        user: body,
        temperature: 0.0,
        kind: "memory_tree::summarise",
    };

    log::debug!(
        "[memory_tree::summarise] provider={} tree_id={} level={} inputs={} budget={}",
        provider.name(),
        ctx.tree_id,
        ctx.target_level,
        inputs.len(),
        ctx.token_budget,
    );

    let raw = provider
        .chat_for_text(&prompt)
        .await
        .with_context(|| format!("memory_tree::summarise: provider={}", provider.name()))?;

    let (content, token_count) = clamp_to_budget(raw.trim(), effective_budget);
    log::debug!(
        "[memory_tree::summarise] sealed tree_id={} level={} inputs={} tokens={}",
        ctx.tree_id,
        ctx.target_level,
        inputs.len(),
        token_count,
    );

    Ok(SummaryOutput {
        content,
        token_count,
        entities: Vec::new(),
        topics: Vec::new(),
    })
}

/// Deterministic, dependency-free summary — concatenate inputs with a
/// provenance prefix and truncate to budget. Used by seal cascades when
/// [`summarise`] returns an error and the cascade must still produce a
/// parent row (replaces the old `InertSummariser` soft-fallback role).
pub fn fallback_summary(inputs: &[SummaryInput], budget: u32) -> SummaryOutput {
    const PROVENANCE_PREFIX: &str = "— ";
    let mut parts: Vec<String> = Vec::with_capacity(inputs.len());
    for inp in inputs {
        let trimmed = inp.content.trim();
        if trimmed.is_empty() {
            continue;
        }
        parts.push(format!("{PROVENANCE_PREFIX}{trimmed}"));
    }
    let joined = parts.join("\n\n");
    let (content, token_count) = clamp_to_budget(&joined, budget);
    SummaryOutput {
        content,
        token_count,
        entities: Vec::new(),
        topics: Vec::new(),
    }
}

fn build_user_prompt(inputs: &[SummaryInput], per_input_cap_tokens: u32) -> String {
    let mut out = String::new();
    for inp in inputs {
        let trimmed = inp.content.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (clamped, _) = clamp_to_budget(trimmed, per_input_cap_tokens);
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&format!("[{}]\n{clamped}", inp.id));
    }
    out
}

fn system_prompt(budget: u32, output_language: Option<&str>) -> String {
    let lang_line = match output_language {
        Some(lang) if !lang.trim().is_empty() => {
            format!("\nWrite the summary in {lang}.")
        }
        _ => String::new(),
    };
    format!(
        "You are folding multiple notes into one compact summary.\n\
         Aim for ~{budget} tokens or fewer. Capture key facts, decisions, and entities.\n\
         Output only the summary prose — no preamble, no JSON, no markdown headings.{lang_line}"
    )
}

fn clamp_to_budget(text: &str, budget: u32) -> (String, u32) {
    let initial = approx_token_count(text);
    if initial <= budget {
        return (text.to_string(), initial);
    }
    let char_ceiling = (budget as usize).saturating_mul(4);
    let truncated: String = text.chars().take(char_ceiling).collect();
    let tokens = approx_token_count(&truncated);
    (truncated, tokens)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_input(id: &str, content: &str) -> SummaryInput {
        let ts = Utc::now();
        SummaryInput {
            id: id.to_string(),
            content: content.to_string(),
            token_count: approx_token_count(content),
            entities: Vec::new(),
            topics: Vec::new(),
            time_range_start: ts,
            time_range_end: ts,
            score: 0.5,
        }
    }

    #[test]
    fn fallback_concatenates_with_provenance_prefix() {
        let inputs = vec![sample_input("a", "hello"), sample_input("b", "world")];
        let out = fallback_summary(&inputs, 10_000);
        assert!(out.content.contains("hello"));
        assert!(out.content.contains("world"));
        assert!(out.content.contains("— "));
        assert!(out.entities.is_empty());
    }

    #[test]
    fn fallback_truncates_at_budget() {
        let inputs = vec![sample_input("a", &"x".repeat(1000))];
        let out = fallback_summary(&inputs, 5);
        assert!(out.token_count <= 6);
    }

    #[test]
    fn fallback_skips_blank_inputs() {
        let inputs = vec![sample_input("a", "   "), sample_input("b", "kept")];
        let out = fallback_summary(&inputs, 10_000);
        assert!(out.content.contains("kept"));
        assert_eq!(out.content.matches("— ").count(), 1);
    }
}
