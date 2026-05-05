//! LLM-backed summariser — peer of
//! [`crate::openhuman::memory::tree::score::extract::llm::LlmEntityExtractor`].
//!
//! ## Responsibility
//!
//! When the source / topic / global tree's bucket-seal cascade decides to
//! fold N contributions (raw leaves at L0→L1, or lower-level summaries at
//! L_n→L_{n+1}), this summariser is asked to produce the parent node's
//! `content`. The seal machinery itself (bucket budgeting, level
//! promotion, `mem_tree_summaries` persistence) is unchanged — only the
//! text inside the summary row differs from [`super::inert::InertSummariser`].
//! Entities and topics on `SummaryOutput` are always emitted empty by
//! this summariser; canonical entity ids are populated separately by the
//! entity extractor.
//!
//! ## Soft-fallback contract
//!
//! A summariser that returns `Err` would abort the seal cascade and leave
//! the tree in an inconsistent state — a half-sealed buffer with no
//! parent row. We therefore promise **never** to return `Err`: every
//! failure (transport, HTTP status, JSON shape) falls back to the same
//! deterministic concat-and-truncate behaviour as `InertSummariser` and
//! logs a warn.
//!
//! ## Prompt shape
//!
//! The system prompt commits the model to returning JSON with the shape
//! `{ summary }`. We pass `temperature: 0.0` for maximum determinism —
//! same knob the entity extractor already uses with success.
//!
//! ## Backend transparency
//!
//! Originally this summariser owned its own `reqwest::Client` and talked
//! directly to Ollama. After the cloud-default refactor, it accepts an
//! `Arc<dyn ChatProvider>` instead — letting a single workspace pick
//! cloud (default) or local (opt-in) at runtime without changing this
//! file's prompt or parse logic.

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::sync::Arc;

use super::inert::InertSummariser;
use super::{Summariser, SummaryContext, SummaryInput, SummaryOutput};
use crate::openhuman::memory::tree::chat::{ChatPrompt, ChatProvider};
use crate::openhuman::memory::tree::types::approx_token_count;

/// Hard cap on summariser output length (in approximate tokens).
///
/// Two constraints set this:
///
/// 1. The downstream embedder (`nomic-embed-text-v1.5`) accepts up to
///    8192 tokens, and Phase 4 (`tree_source::bucket_seal`) embeds the
///    summary right after we produce it. An overshoot returns HTTP 500
///    and rolls back the whole seal transaction.
/// 2. Empirically, small instruction-tuned models running locally
///    degrade quickly past ~3500 tokens — they drift, hallucinate, or
///    produce repetitive boilerplate as they extend toward longer
///    targets. Keeping the cap below that breakeven keeps output
///    quality stable on local Ollama deployments.
///
/// 3500 sits comfortably under the embedder ceiling AND below the local
/// LLM quality cliff. The post-generation [`clamp_to_budget`] enforces
/// this regardless of what the model produces.
const MAX_SUMMARY_OUTPUT_TOKENS: u32 = 3_500;

/// Context window assumed for the model. Used as the divisor in the
/// per-input clamp so the joined prompt body stays under this even at
/// upper-level seals where SUMMARY_FANOUT children each near
/// MAX_SUMMARY_OUTPUT_TOKENS would otherwise overflow. Conservative —
/// real cloud models have larger contexts; smaller local models may
/// truncate, but the post-generation `clamp_to_budget` ensures output
/// fits the embedder regardless.
const NUM_CTX_TOKENS: u32 = 16_384;

/// Tokens reserved for the system prompt, JSON wrapper, and tokenizer
/// drift between our 4-chars/token heuristic and the model's tokenizer.
/// Trades a small loss of input capacity for a guarantee that the
/// prompt body + output budget never exceeds `num_ctx`.
const OVERHEAD_RESERVE_TOKENS: u32 = 512;

/// Configuration for [`LlmSummariser`]. Threaded down to the chat
/// provider for diagnostic logging — model selection at the wire level
/// happens inside the [`ChatProvider`].
#[derive(Clone, Debug)]
pub struct LlmSummariserConfig {
    /// Model identifier (e.g. `summarization-v1` for cloud, `qwen2.5:0.5b`
    /// or `llama3.1:8b` for local Ollama). Diagnostic / log only.
    pub model: String,
}

impl Default for LlmSummariserConfig {
    fn default() -> Self {
        Self {
            model: "qwen2.5:0.5b".to_string(),
        }
    }
}

/// LLM-backed summariser. Delegates to [`InertSummariser`] on any
/// failure so seal cascades never fail.
pub struct LlmSummariser {
    cfg: LlmSummariserConfig,
    provider: Arc<dyn ChatProvider>,
    fallback: InertSummariser,
}

impl LlmSummariser {
    /// Build a summariser with the supplied chat provider. Infallible —
    /// the caller is responsible for provider construction.
    pub fn new(cfg: LlmSummariserConfig, provider: Arc<dyn ChatProvider>) -> Self {
        Self {
            cfg,
            provider,
            fallback: InertSummariser::new(),
        }
    }

    /// Build the chat prompt sent to the provider for a given seal.
    fn build_prompt(&self, prompt_body: &str, budget: u32) -> ChatPrompt {
        ChatPrompt {
            system: system_prompt(budget),
            user: prompt_body.to_string(),
            temperature: 0.0,
            kind: "memory_tree::summarise",
        }
    }
}

#[async_trait]
impl Summariser for LlmSummariser {
    async fn summarise(
        &self,
        inputs: &[SummaryInput],
        ctx: &SummaryContext<'_>,
    ) -> Result<SummaryOutput> {
        // Clamp the model-side output budget so the summary fits the
        // downstream embedder. The seal-cascade hands us
        // `ctx.token_budget = 10k` by default but `nomic-embed-text`
        // only accepts ≤ 8k tokens of input. Producing a smaller
        // summary upfront avoids the embed-fails-after-summary
        // dead end.
        let effective_budget = ctx.token_budget.min(MAX_SUMMARY_OUTPUT_TOKENS);

        // Per-input clamp scaled by fanout. Without this, an upper-level
        // seal feeding `SUMMARY_FANOUT=4` children each near
        // `MAX_SUMMARY_OUTPUT_TOKENS` would push the prompt body alone
        // past `num_ctx` and Ollama would silently truncate (or error).
        // Divide the input budget evenly across contributors.
        let per_input_cap = if inputs.is_empty() {
            0
        } else {
            NUM_CTX_TOKENS
                .saturating_sub(effective_budget)
                .saturating_sub(OVERHEAD_RESERVE_TOKENS)
                / inputs.len() as u32
        };

        // Assemble the user-side prompt. We prefix each contribution with
        // its id so the model can weigh them and so log diffs are
        // traceable to source rows if anything looks odd.
        let body = build_user_prompt(inputs, per_input_cap);
        if body.trim().is_empty() {
            log::debug!(
                "[tree_source::summariser::llm] empty prompt body (no non-blank inputs) \
                 tree_id={} level={} — returning empty summary",
                ctx.tree_id,
                ctx.target_level
            );
            return Ok(SummaryOutput {
                content: String::new(),
                token_count: 0,
                entities: Vec::new(),
                topics: Vec::new(),
            });
        }

        let prompt = self.build_prompt(&body, effective_budget);

        log::debug!(
            "[tree_source::summariser::llm] chat provider={} model={} tree_id={} level={} \
             inputs={} budget={}",
            self.provider.name(),
            self.cfg.model,
            ctx.tree_id,
            ctx.target_level,
            inputs.len(),
            ctx.token_budget
        );

        let raw = match self.provider.chat_for_json(&prompt).await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "[tree_source::summariser::llm] chat provider={} failed: {e:#} — \
                     falling back to inert summariser for tree_id={} level={}",
                    self.provider.name(),
                    ctx.tree_id,
                    ctx.target_level
                );
                return self.fallback.summarise(inputs, ctx).await;
            }
        };

        let parsed: LlmSummaryOutput = match serde_json::from_str(&raw) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "[tree_source::summariser::llm] model returned non-JSON or wrong-shape \
                     body: {e}; content was: {} — falling back to inert",
                    truncate_for_log(&raw, 400)
                );
                return self.fallback.summarise(inputs, ctx).await;
            }
        };

        let (content, token_count) = clamp_to_budget(&parsed.summary, effective_budget);
        log::debug!(
            "[tree_source::summariser::llm] sealed tree_id={} level={} inputs={} tokens={}",
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

/// Build the user-message body that precedes the model call. Each
/// contribution is prefixed with a short id header and separated by a
/// blank line — matches the layout the model is instructed to
/// summarise. Each input's content is clamped to
/// `per_input_cap_tokens` so the joined body fits inside `num_ctx` even
/// at upper-level seals where many large summaries fold together. A
/// `0` cap means "don't include any content" (used when there are no
/// inputs); pass `u32::MAX` to disable clamping.
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

/// System prompt. Length isn't templated in — empirically, telling small
/// instruction-tuned models "stay under N tokens" makes them produce
/// curt, generic output even when the input has plenty of substance.
/// Output is clamped post-generation by [`clamp_to_budget`] in the
/// caller, so we don't need the model to self-police length.
fn system_prompt(_budget: u32) -> String {
    "You are a precise summariser. Summarise the user-provided contributions into a \
     single cohesive passage that preserves concrete facts, decisions, \
     and temporal ordering. Do not invent facts.\n\
     \n\
     Return JSON only — no prose, no markdown, no commentary. Schema:\n\
     {\n\
     \x20 \"summary\": \"<summary body>\"\n\
     }"
    .to_string()
}

/// Truncate to the caller's token budget using the same ~4 chars/token
/// heuristic as [`InertSummariser`].
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

fn truncate_for_log(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

// ── LLM JSON output ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LlmSummaryOutput {
    #[serde(default)]
    summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::tree_source::types::TreeKind;
    use chrono::Utc;

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

    fn test_ctx() -> SummaryContext<'static> {
        SummaryContext {
            tree_id: "tree-1",
            tree_kind: TreeKind::Source,
            target_level: 1,
            token_budget: 10_000,
        }
    }

    #[test]
    fn build_user_prompt_includes_ids_and_content() {
        let inputs = vec![
            sample_input("a", "hello world"),
            sample_input("b", "second contribution"),
        ];
        let out = build_user_prompt(&inputs, u32::MAX);
        assert!(out.contains("[a]"));
        assert!(out.contains("hello world"));
        assert!(out.contains("[b]"));
        assert!(out.contains("second contribution"));
    }

    #[test]
    fn build_user_prompt_skips_blank_contributions() {
        let inputs = vec![sample_input("a", "   "), sample_input("b", "kept")];
        let out = build_user_prompt(&inputs, u32::MAX);
        assert!(!out.contains("[a]"));
        assert!(out.contains("[b]"));
        assert!(out.contains("kept"));
    }

    #[test]
    fn build_user_prompt_clamps_each_input_to_per_input_cap() {
        // Regression guard for upper-level context overflow: at L2 with
        // SUMMARY_FANOUT=4 and large child summaries, the joined body
        // would otherwise blow past NUM_CTX_TOKENS. The clamp keeps
        // each contribution under per_input_cap_tokens regardless of
        // how big the original content is.
        let long = "x".repeat(2_000); // ~500 approx-tokens
        let inputs = vec![
            sample_input("a", &long),
            sample_input("b", &long),
            sample_input("c", &long),
            sample_input("d", &long),
        ];
        let cap_tokens: u32 = 50; // ~200 chars per input
        let out = build_user_prompt(&inputs, cap_tokens);

        // Each input contributes at most cap_tokens*4 chars of content,
        // plus a small id header. Total stays well under the unclamped
        // 4 * 2_000 = 8_000 chars baseline.
        let unclamped_baseline = 4 * 2_000;
        assert!(
            out.len() < unclamped_baseline / 2,
            "expected clamp to halve the body or better, got {} chars",
            out.len()
        );
        assert!(out.contains("[a]"));
        assert!(out.contains("[d]"));
    }

    #[test]
    fn system_prompt_describes_schema() {
        // Budget is no longer templated into the prompt — small models
        // produced overly curt output when told to "stay under N tokens".
        // The clamp in `clamp_to_budget` handles enforcement instead.
        let p = system_prompt(4096);
        assert!(!p.contains("4096"));
        assert!(!p.contains("Stay well under"));
        assert!(p.contains("\"summary\""));
        assert!(!p.contains("\"entities\""));
        assert!(!p.contains("\"topics\""));
    }

    #[test]
    fn clamp_to_budget_no_op_when_under() {
        let (out, t) = clamp_to_budget("short", 1000);
        assert_eq!(out, "short");
        assert_eq!(t, approx_token_count("short"));
    }

    #[test]
    fn clamp_to_budget_truncates_when_over() {
        let long = "a".repeat(1000);
        let (out, t) = clamp_to_budget(&long, 5);
        assert!(out.len() < long.len());
        assert!(t <= 6);
    }

    #[test]
    fn truncate_for_log_short_input_unchanged() {
        assert_eq!(truncate_for_log("hi", 10), "hi");
    }

    #[test]
    fn truncate_for_log_long_input_appends_ellipsis() {
        let long = "x".repeat(500);
        let out = truncate_for_log(&long, 10);
        assert_eq!(out.chars().count(), 11);
        assert!(out.ends_with('…'));
    }

    /// Mock chat provider that lets us assert prompt shape and stub responses
    /// in summariser unit tests without hitting the network.
    struct StubProvider {
        response: anyhow::Result<String>,
        calls: std::sync::atomic::AtomicUsize,
    }

    impl StubProvider {
        fn ok(text: impl Into<String>) -> Self {
            Self {
                response: Ok(text.into()),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
        fn err(msg: &'static str) -> Self {
            Self {
                response: Err(anyhow::anyhow!(msg)),
                calls: std::sync::atomic::AtomicUsize::new(0),
            }
        }
    }

    #[async_trait]
    impl ChatProvider for StubProvider {
        fn name(&self) -> &str {
            "test:stub"
        }
        async fn chat_for_json(&self, _p: &ChatPrompt) -> anyhow::Result<String> {
            self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.response
                .as_ref()
                .map(|s| s.clone())
                .map_err(|e| anyhow::anyhow!("{e}"))
        }
    }

    #[tokio::test]
    async fn empty_inputs_yield_empty_summary_without_provider_call() {
        // All inputs are blank → prompt body is empty → the summariser
        // short-circuits and returns an empty output without invoking the
        // chat provider.
        let provider = std::sync::Arc::new(StubProvider::ok("never returned"));
        let s = LlmSummariser::new(LlmSummariserConfig::default(), provider.clone());
        let inputs = vec![sample_input("a", "   "), sample_input("b", "")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.is_empty());
        assert_eq!(out.token_count, 0);
        assert_eq!(
            provider.calls.load(std::sync::atomic::Ordering::SeqCst),
            0,
            "blank inputs must not call the chat provider"
        );
    }

    #[tokio::test]
    async fn provider_failure_falls_back_to_inert() {
        // Provider errors → must NOT return Err; must fall through to
        // InertSummariser's concatenate+truncate behaviour (content
        // present, entities empty).
        let provider = std::sync::Arc::new(StubProvider::err("simulated"));
        let s = LlmSummariser::new(LlmSummariserConfig::default(), provider);
        let inputs = vec![sample_input("a", "alice decided to ship friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.contains("alice decided to ship"));
        assert!(out.entities.is_empty());
        assert!(out.topics.is_empty());
    }

    #[tokio::test]
    async fn malformed_response_falls_back_to_inert() {
        // Provider returns garbage → parse fails → fallback to inert.
        let provider = std::sync::Arc::new(StubProvider::ok("not json"));
        let s = LlmSummariser::new(LlmSummariserConfig::default(), provider);
        let inputs = vec![sample_input("a", "alice ships friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        // Inert fallback content includes the original input.
        assert!(out.content.contains("alice"));
    }

    #[tokio::test]
    async fn provider_summary_response_is_used_and_clamped() {
        // Provider returns valid JSON; summariser parses it and clamps to
        // the budget.
        let provider = std::sync::Arc::new(StubProvider::ok(
            r#"{"summary":"alice decided to ship friday"}"#,
        ));
        let s = LlmSummariser::new(LlmSummariserConfig::default(), provider.clone());
        let inputs = vec![sample_input("a", "alice ships friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.contains("alice decided to ship"));
        assert!(out.token_count > 0);
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[test]
    fn build_prompt_carries_body_and_kind_tag() {
        let provider = std::sync::Arc::new(StubProvider::ok("{}"));
        let s = LlmSummariser::new(
            LlmSummariserConfig {
                model: "llama3.1:8b".into(),
            },
            provider,
        );
        let prompt = s.build_prompt("body", 2048);
        assert!(prompt.system.contains("\"summary\""));
        assert!(!prompt.system.contains("\"entities\""));
        assert_eq!(prompt.user, "body");
        assert_eq!(prompt.temperature, 0.0);
        assert_eq!(prompt.kind, "memory_tree::summarise");
    }

    #[test]
    fn llm_output_deserialises_with_only_summary() {
        let v: LlmSummaryOutput = serde_json::from_str(r#"{"summary":"hi"}"#).unwrap();
        assert_eq!(v.summary, "hi");
    }

    #[test]
    fn llm_output_ignores_extraneous_fields() {
        // Prompt no longer asks for entities/topics, but if the model
        // emits them anyway we should still parse `summary` cleanly.
        let v: LlmSummaryOutput =
            serde_json::from_str(r#"{"summary":"hi","entities":["Alice"],"topics":["x"]}"#)
                .unwrap();
        assert_eq!(v.summary, "hi");
    }
}
