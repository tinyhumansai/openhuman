//! LLM-backed summariser — Ollama `/api/chat` peer of
//! [`crate::openhuman::memory::tree::score::extract::llm::LlmEntityExtractor`].
//!
//! ## Responsibility
//!
//! When the source / topic / global tree's bucket-seal cascade decides to
//! fold N contributions (raw leaves at L0→L1, or lower-level summaries at
//! L_n→L_{n+1}), this summariser is asked to produce the parent node's
//! `content` + derived `entities` + `topics`. The seal machinery itself
//! (bucket budgeting, level promotion, `mem_tree_summaries` persistence)
//! is unchanged — only the text inside the summary row differs from
//! [`super::inert::InertSummariser`].
//!
//! ## Soft-fallback contract
//!
//! A summariser that returns `Err` would abort the seal cascade and leave
//! the tree in an inconsistent state — a half-sealed buffer with no
//! parent row. We therefore promise **never** to return `Err`: every
//! failure (transport, HTTP status, JSON shape) falls back to the same
//! deterministic concat-and-truncate behaviour as `InertSummariser` and
//! logs a warn. Callers distinguish "LLM ran fine" from "we fell back"
//! only by observing whether the returned `entities`/`topics` are
//! populated — the inert branch emits empty vecs.
//!
//! ## Prompt shape
//!
//! The system prompt commits the model to returning JSON with the shape
//! `{ summary, entities, topics }`. We use Ollama's `format: "json"` +
//! `temperature: 0.0` to maximise determinism — same knobs the entity
//! extractor already uses with success.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};

use super::inert::InertSummariser;
use super::{Summariser, SummaryContext, SummaryInput, SummaryOutput};
use crate::openhuman::memory::tree::types::approx_token_count;

/// Hard cap on summary OUTPUT tokens, regardless of the seal's
/// `ctx.token_budget`. Driven by the embedder's context window —
/// `nomic-embed-text-v1.5` accepts up to 8192 tokens and Phase 4
/// (`source_tree::bucket_seal`) embeds the summary right after we
/// produce it. If our summary overshoots, the embedder returns 500
/// and the whole seal transaction rolls back → no summary persists.
/// 6000 leaves a safety margin for tokenizer differences and the
/// JSON wrapper.
const MAX_SUMMARY_OUTPUT_TOKENS: u32 = 6_000;

/// Context window we ask Ollama for. Must match the value below in
/// [`OllamaOptions::num_ctx`] so the per-input clamp computed in
/// [`LlmSummariser::summarise`] sizes inputs against the same window
/// the model actually sees.
const NUM_CTX_TOKENS: u32 = 16_384;

/// Tokens reserved for the system prompt, JSON wrapper, and tokenizer
/// drift between our 4-chars/token heuristic and the model's tokenizer.
/// Trades a small loss of input capacity for a guarantee that the
/// prompt body + output budget never exceeds `num_ctx`.
const OVERHEAD_RESERVE_TOKENS: u32 = 512;

/// Configuration for [`LlmSummariser`]. Endpoint + model defaults match
/// [`crate::openhuman::memory::tree::score::extract::llm::LlmExtractorConfig`]
/// so a workspace configured for one LLM path also satisfies the other
/// by default.
#[derive(Clone, Debug)]
pub struct LlmSummariserConfig {
    /// Base URL of the Ollama-compatible endpoint (e.g.
    /// `http://localhost:11434`). Do NOT include `/api/chat` — the
    /// summariser appends it.
    pub endpoint: String,
    /// Model identifier (e.g. `qwen2.5:0.5b` or `llama3.1:8b`).
    pub model: String,
    /// Per-request timeout. Generous default because first-call weight
    /// loading can be slow.
    pub timeout: Duration,
}

impl Default for LlmSummariserConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://localhost:11434".to_string(),
            model: "qwen2.5:0.5b".to_string(),
            // 120s — generous enough for small/medium models (1B-8B
            // params) summarising the seal cascade's full token
            // budget on first invocation, when Ollama may also be
            // loading model weights into VRAM. Large models on CPU
            // can still time out; bump via config for those.
            timeout: Duration::from_secs(120),
        }
    }
}

/// LLM-backed summariser. Delegates to [`InertSummariser`] on any
/// failure so seal cascades never fail.
pub struct LlmSummariser {
    cfg: LlmSummariserConfig,
    http: Client,
    fallback: InertSummariser,
}

impl LlmSummariser {
    pub fn new(cfg: LlmSummariserConfig) -> Result<Self> {
        let http = Client::builder()
            .timeout(cfg.timeout)
            .build()
            .map_err(anyhow::Error::from)?;
        Ok(Self {
            cfg,
            http,
            fallback: InertSummariser::new(),
        })
    }

    fn build_request(&self, prompt_body: &str, budget: u32) -> OllamaChatRequest {
        OllamaChatRequest {
            model: self.cfg.model.clone(),
            messages: vec![
                OllamaMessage {
                    role: "system".to_string(),
                    content: system_prompt(budget),
                },
                OllamaMessage {
                    role: "user".to_string(),
                    content: prompt_body.to_string(),
                },
            ],
            format: "json".to_string(),
            stream: false,
            options: OllamaOptions {
                temperature: 0.0,
                // 16k context window. Sized so that the per-input
                // clamp in `summarise` (NUM_CTX - output_budget -
                // overhead, divided by `inputs.len()`) keeps the
                // joined prompt body inside this window even at
                // upper-level seals where SUMMARY_FANOUT children
                // each near MAX_SUMMARY_OUTPUT_TOKENS would otherwise
                // overflow. Keeping `num_ctx` modest also keeps the
                // kv-cache small enough to fit on consumer GPUs
                // alongside the model weights.
                num_ctx: NUM_CTX_TOKENS,
            },
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
                "[source_tree::summariser::llm] empty prompt body (no non-blank inputs) \
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

        let url = format!("{}/api/chat", self.cfg.endpoint.trim_end_matches('/'));
        let req = self.build_request(&body, effective_budget);

        log::debug!(
            "[source_tree::summariser::llm] POST {url} model={} tree_id={} level={} \
             inputs={} budget={}",
            self.cfg.model,
            ctx.tree_id,
            ctx.target_level,
            inputs.len(),
            ctx.token_budget
        );

        let resp = match self.http.post(&url).json(&req).send().await {
            Ok(r) => r,
            Err(e) => {
                log::warn!(
                    "[source_tree::summariser::llm] transport failure to {url}: {e} — \
                     falling back to inert summariser for tree_id={} level={}",
                    ctx.tree_id,
                    ctx.target_level
                );
                return self.fallback.summarise(inputs, ctx).await;
            }
        };

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            log::warn!(
                "[source_tree::summariser::llm] ollama non-success status {status} \
                 tree_id={} level={}: {} — falling back to inert",
                ctx.tree_id,
                ctx.target_level,
                truncate_for_log(&body, 200)
            );
            return self.fallback.summarise(inputs, ctx).await;
        }

        let envelope: OllamaChatResponse = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "[source_tree::summariser::llm] response not Ollama-shaped JSON: {e} — \
                     falling back to inert for tree_id={} level={}",
                    ctx.tree_id,
                    ctx.target_level
                );
                return self.fallback.summarise(inputs, ctx).await;
            }
        };

        let parsed: LlmSummaryOutput = match serde_json::from_str(&envelope.message.content) {
            Ok(v) => v,
            Err(e) => {
                log::warn!(
                    "[source_tree::summariser::llm] model returned non-JSON or wrong-shape \
                     body: {e}; content was: {} — falling back to inert",
                    truncate_for_log(&envelope.message.content, 400)
                );
                return self.fallback.summarise(inputs, ctx).await;
            }
        };

        let (content, token_count) = clamp_to_budget(&parsed.summary, effective_budget);
        log::debug!(
            "[source_tree::summariser::llm] sealed tree_id={} level={} inputs={} tokens={} \
             surface_entities_dropped={} topics={}",
            ctx.tree_id,
            ctx.target_level,
            inputs.len(),
            token_count,
            parsed.entities.len(),
            parsed.topics.len()
        );

        // Drop LLM-emitted entities. The model returns surface forms
        // ("Alice", "she"), but `SummaryNode.entities` is indexed via
        // `index_summary_entity_ids_tx` as canonical ids. Surface forms
        // would silently corrupt that index — searches by canonical id
        // would not find these summaries. Canonicalisation is the
        // entity extractor's job, not the summariser's. Topics stay
        // because they're free-form labels, not indexed as canonical
        // ids. The `entities` field stays in the prompt to nudge the
        // model toward entity-aware summarisation; we just don't
        // persist its output.
        Ok(SummaryOutput {
            content,
            token_count,
            entities: Vec::new(),
            topics: dedupe_sorted(parsed.topics),
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

/// System prompt. Token budget is templated in so the model aims under it.
fn system_prompt(budget: u32) -> String {
    format!(
        "You are a precise summariser. Summarise the user-provided contributions into a \
         single cohesive passage that preserves concrete facts, decisions, named entities, \
         and temporal ordering. Do not invent facts. Stay well under {budget} tokens.\n\
         \n\
         Return JSON only — no prose, no markdown, no commentary. Schema:\n\
         {{\n\
         \x20 \"summary\": \"<summary body>\",\n\
         \x20 \"entities\": [\"<named entity surface forms mentioned in the summary>\"],\n\
         \x20 \"topics\": [\"<short topic labels covered by the summary>\"]\n\
         }}"
    )
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

fn dedupe_sorted(mut items: Vec<String>) -> Vec<String> {
    for item in items.iter_mut() {
        *item = item.trim().to_string();
    }
    items.retain(|s| !s.is_empty());
    items.sort();
    items.dedup();
    items
}

fn truncate_for_log(s: &str, max_chars: usize) -> String {
    if s.chars().count() <= max_chars {
        return s.to_string();
    }
    let truncated: String = s.chars().take(max_chars).collect();
    format!("{truncated}…")
}

// ── Wire types (Ollama API) ──────────────────────────────────────────────

#[derive(Debug, Serialize)]
struct OllamaChatRequest {
    model: String,
    messages: Vec<OllamaMessage>,
    format: String,
    stream: bool,
    options: OllamaOptions,
}

#[derive(Debug, Serialize)]
struct OllamaMessage {
    role: String,
    content: String,
}

#[derive(Debug, Serialize)]
struct OllamaOptions {
    temperature: f32,
    /// Override Ollama's default 32k context window — large local
    /// models inflate kv-cache to >15 GiB at 32k which won't fit on
    /// consumer GPUs. 16k is plenty for one bucket summary (input is
    /// bounded by the source-tree's ~10k-token bucket budget) while
    /// keeping kv-cache reasonable.
    num_ctx: u32,
}

#[derive(Debug, Deserialize)]
struct OllamaChatResponse {
    message: OllamaResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OllamaResponseMessage {
    content: String,
}

// ── LLM JSON output ──────────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct LlmSummaryOutput {
    #[serde(default)]
    summary: String,
    #[serde(default)]
    entities: Vec<String>,
    #[serde(default)]
    topics: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::tree::source_tree::types::TreeKind;
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
    fn system_prompt_templates_budget() {
        let p = system_prompt(4096);
        assert!(p.contains("4096"));
        assert!(p.contains("\"summary\""));
        assert!(p.contains("\"entities\""));
        assert!(p.contains("\"topics\""));
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
    fn dedupe_sorted_trims_and_dedupes() {
        let out = dedupe_sorted(vec![
            "Bob".into(),
            "  Alice  ".into(),
            "Bob".into(),
            "".into(),
            "   ".into(),
            "Alice".into(),
        ]);
        assert_eq!(out, vec!["Alice", "Bob"]);
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

    #[tokio::test]
    async fn empty_inputs_yield_empty_summary_without_network_call() {
        // All inputs are blank → prompt body is empty → the summariser
        // short-circuits and returns an empty output. Importantly, this
        // path must work even if Ollama is unreachable.
        let cfg = LlmSummariserConfig {
            endpoint: "http://127.0.0.1:1".to_string(),
            timeout: Duration::from_millis(50),
            ..LlmSummariserConfig::default()
        };
        let s = LlmSummariser::new(cfg).unwrap();
        let inputs = vec![sample_input("a", "   "), sample_input("b", "")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.is_empty());
        assert_eq!(out.token_count, 0);
    }

    #[tokio::test]
    async fn transport_failure_falls_back_to_inert() {
        // Unreachable endpoint → transport error → must NOT return Err;
        // must fall through to InertSummariser's concatenate+truncate
        // behaviour (content present, entities empty).
        let cfg = LlmSummariserConfig {
            endpoint: "http://127.0.0.1:1".to_string(),
            timeout: Duration::from_millis(100),
            ..LlmSummariserConfig::default()
        };
        let s = LlmSummariser::new(cfg).unwrap();
        let inputs = vec![sample_input("a", "alice decided to ship friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.contains("alice decided to ship"));
        // Inert branch emits empty entities/topics — this is how callers
        // can distinguish fallback from a real LLM success with no entities.
        assert!(out.entities.is_empty());
        assert!(out.topics.is_empty());
    }

    #[test]
    fn build_request_uses_configured_model_and_json_format() {
        let cfg = LlmSummariserConfig {
            model: "llama3.1:8b".into(),
            ..LlmSummariserConfig::default()
        };
        let s = LlmSummariser::new(cfg).unwrap();
        let req = s.build_request("body", 2048);
        assert_eq!(req.model, "llama3.1:8b");
        assert_eq!(req.format, "json");
        assert!(!req.stream);
        assert_eq!(req.options.temperature, 0.0);
        assert_eq!(req.messages[0].role, "system");
        assert!(req.messages[0].content.contains("2048"));
        assert_eq!(req.messages[1].role, "user");
        assert_eq!(req.messages[1].content, "body");
    }

    #[test]
    fn llm_output_deserialises_with_missing_fields() {
        let v: LlmSummaryOutput = serde_json::from_str(r#"{"summary":"hi"}"#).unwrap();
        assert_eq!(v.summary, "hi");
        assert!(v.entities.is_empty());
        assert!(v.topics.is_empty());
    }
}
