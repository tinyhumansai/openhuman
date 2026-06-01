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
use std::sync::Arc;

use super::inert::InertSummariser;
use super::{Summariser, SummaryContext, SummaryInput, SummaryOutput};
use crate::openhuman::learning::extract::summary_facets::{self, StructuredSummary};
use crate::openhuman::memory::tree::chat::{ChatPrompt, ChatProvider};
use crate::openhuman::memory::tree::types::approx_token_count;

/// Hard cap on summariser output length (in approximate tokens).
///
/// Sized to fit the downstream embedder (`nomic-embed-text-v1.5`,
/// 8192-token input ceiling) with headroom for tokenizer drift between
/// our 4-chars/token heuristic and the embedder's real tokenizer. The
/// post-generation [`clamp_to_budget`] enforces this regardless of what
/// the model produces.
const MAX_SUMMARY_OUTPUT_TOKENS: u32 = 5_000;

/// Context window assumed for the model. Sized for the cloud
/// summariser's 120k-token window with comfortable headroom — leaves
/// room for the joined L0 input batch (up to `INPUT_TOKEN_BUDGET = 50k`),
/// the requested output budget, the system prompt, and tokenizer drift.
/// Used as the divisor in the per-input clamp so the joined prompt body
/// stays under this even at upper-level seals where many children fold
/// together.
const NUM_CTX_TOKENS: u32 = 60_000;

/// Tokens reserved for the system prompt, message-envelope overhead,
/// and tokenizer drift between our 4-chars/token heuristic and the
/// model's tokenizer. Trades a small loss of input capacity for a
/// guarantee that the prompt body + output budget never exceeds
/// `num_ctx`.
const OVERHEAD_RESERVE_TOKENS: u32 = 2_048;

/// Configuration for [`LlmSummariser`]. Threaded down to the chat
/// provider for diagnostic logging — model selection at the wire level
/// happens inside the [`ChatProvider`].
#[derive(Clone, Debug)]
pub struct LlmSummariserConfig {
    /// Model identifier (e.g. `summarization-v1` for cloud, `qwen2.5:0.5b`
    /// or `llama3.1:8b` for local Ollama). Diagnostic / log only.
    pub model: String,
    /// When `true` (the default), the summariser appends a structured facet
    /// extraction request to the prompt and parses the resulting JSON block.
    /// Discovered facets are routed to the learning candidate buffer.
    /// Set to `false` to restore the plain-text-only behaviour for A/B testing
    /// or debugging.
    pub structured_facet_extraction: bool,
}

impl Default for LlmSummariserConfig {
    fn default() -> Self {
        Self {
            model: "qwen2.5:0.5b".to_string(),
            structured_facet_extraction: true,
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
    ///
    /// When `structured_facet_extraction` is enabled the system prompt includes
    /// an instruction to emit a fenced `json` block after the prose summary.
    fn build_prompt(&self, prompt_body: &str, budget: u32) -> ChatPrompt {
        ChatPrompt {
            system: system_prompt(budget, self.cfg.structured_facet_extraction),
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

        let raw = match self.provider.chat_for_text(&prompt).await {
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

        // When structured_facet_extraction is enabled, attempt to split the response
        // into a prose summary and an optional JSON block. On parse failure, the
        // prose is used as-is and zero facets are emitted (fail-soft).
        let summary_text: String;

        if self.cfg.structured_facet_extraction {
            let (prose, maybe_structured) = split_structured_response(raw.trim());
            let mut parsed_summary_text = String::new();
            match maybe_structured {
                Some(Ok(parsed)) => {
                    parsed_summary_text = parsed.summary.trim().to_string();
                    tracing::debug!(
                        "[learning::extract::summary] source_id={} facets_emitted={}",
                        ctx.tree_id,
                        parsed.facets.len()
                    );
                    summary_facets::route_facets_to_buffer(&parsed, ctx.tree_id);
                }
                Some(Err(e)) => {
                    log::warn!(
                        "[tree_source::summariser::llm] structured facet parse failed \
                         tree_id={} level={}: {e:#} — using raw prose, emitting 0 facets",
                        ctx.tree_id,
                        ctx.target_level
                    );
                }
                None => {
                    // No JSON block present — normal for content with no clear signals.
                    tracing::debug!(
                        "[tree_source::summariser::llm] no structured JSON block in response \
                         tree_id={} level={}",
                        ctx.tree_id,
                        ctx.target_level
                    );
                }
            }
            summary_text = if !prose.trim().is_empty() {
                prose.trim().to_string()
            } else {
                parsed_summary_text
            };
        } else {
            summary_text = raw.trim().to_string();
        }

        if summary_text.trim().is_empty() {
            log::warn!(
                "[tree_source::summariser::llm] provider returned no usable summary \
                 tree_id={} level={} — falling back to inert summariser",
                ctx.tree_id,
                ctx.target_level
            );
            return self.fallback.summarise(inputs, ctx).await;
        }

        let (content, token_count) = clamp_to_budget(&summary_text, effective_budget);
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

/// System prompt.
///
/// When `structured_facets` is `true`, appends instructions for the model to
/// emit a fenced `json` block after the prose summary containing any clearly
/// evidenced facets.
///
/// Length isn't templated in — empirically, telling instruction-tuned models
/// "stay under N tokens" makes them produce curt, generic output even when the
/// input has plenty of substance. Output is clamped post-generation by
/// [`clamp_to_budget`] in the caller.
fn system_prompt(_budget: u32, structured_facets: bool) -> String {
    let base = "You are a precise summariser. Summarise the user-provided contributions into a \
     single cohesive passage that preserves concrete facts, decisions, \
     and temporal ordering. Do not invent facts.\n\
     \n\
     Return the summary text first.";

    if !structured_facets {
        return format!(
            "{base} No commentary, no preamble, no headings, \
             no markdown wrappers, no JSON — just the prose summary."
        );
    }

    format!(
        "{base}\n\
         \n\
         After the summary, output a compact JSON object as the second part of your response, \
         fenced in a ```json block:\n\
         \n\
         ```json\n\
         {{\n\
           \"facets\": [\n\
             {{\n\
               \"class\": \"style|identity|tooling|veto|goal\",\n\
               \"key\": \"<canonical slug>\",\n\
               \"value\": \"<detected value>\",\n\
               \"evidence_chunks\": [\"<chunk_id>\", \"...\"],\n\
               \"confidence\": 0.0,\n\
               \"cue_family\": \"explicit|structural|behavioral\"\n\
             }}\n\
           ]\n\
         }}\n\
         ```\n\
         \n\
         Rules:\n\
         - Only include facets that are clearly evidenced in the content above.\n\
         - Do not repeat the prose summary inside JSON; the JSON block is only for facets.\n\
         - Keep each value under 80 characters; use a short label, not a sentence.\n\
         - Each facet must cite at least one chunk_id from this batch (the id in brackets \
           before each contribution, e.g. [chunk-abc]).\n\
         - Every facet object must include confidence as a number from 0.0 to 1.0. \
           If you cannot estimate confidence, omit that facet.\n\
         - Use canonical keys: verbosity, format, name, timezone, role, package_manager, \
           lang, framework, runtime, etc.\n\
         - Cap the facets array at 4 items per call. Skip the array entirely (emit \
           facets: []) if no clear evidence.\n\
         - No commentary outside the prose summary and the JSON block."
    )
}

/// Split a raw LLM response that may contain a trailing fenced `json` block.
///
/// Returns `(prose, Option<parse_result>)` where:
/// - `prose` is the text before the ` ```json ` fence (trimmed), or the full
///   raw text when no fence is present.
/// - The second element is `None` when no fence was found, or
///   `Some(Ok(StructuredSummary))` / `Some(Err(…))` on parse success/failure.
fn split_structured_response(raw: &str) -> (&str, Option<anyhow::Result<StructuredSummary>>) {
    // Look for the opening ` ```json ` fence.
    const OPEN_FENCE: &str = "```json";
    const CLOSE_FENCE: &str = "```";

    let Some(fence_start) = raw.find(OPEN_FENCE) else {
        return (raw, None);
    };

    let prose = raw[..fence_start].trim();
    let after_open = &raw[fence_start + OPEN_FENCE.len()..];

    // Find the closing fence.
    let json_str = if let Some(close_pos) = after_open.find(CLOSE_FENCE) {
        after_open[..close_pos].trim()
    } else {
        // No closing fence — treat everything after the open as JSON.
        after_open.trim()
    };

    let result = parse_structured_summary(json_str);

    (prose, Some(result))
}

fn parse_structured_summary(json_str: &str) -> anyhow::Result<StructuredSummary> {
    match parse_structured_summary_inner(json_str) {
        Ok(parsed) => Ok(parsed),
        Err(first_error) => {
            let trimmed = trim_to_first_json_object(json_str);
            if trimmed != json_str {
                if let Ok(parsed) = parse_structured_summary_inner(&trimmed) {
                    return Ok(parsed);
                }
            }
            let repaired = strip_json_trailing_commas(&trimmed);
            if repaired == json_str {
                return Err(anyhow::anyhow!(
                    "structured summary JSON parse error: {first_error}"
                ));
            }
            parse_structured_summary_inner(&repaired).map_err(|second_error| {
                anyhow::anyhow!(
                    "structured summary JSON parse error: {first_error}; repair failed: {second_error}"
                )
            })
        }
    }
}

fn trim_to_first_json_object(json_str: &str) -> String {
    let mut started = false;
    let mut depth = 0usize;
    let mut in_string = false;
    let mut escaped = false;
    let mut start_byte = 0usize;

    for (index, ch) in json_str.char_indices() {
        if !started {
            if ch.is_whitespace() {
                continue;
            }
            if ch != '{' {
                return json_str.to_string();
            }
            started = true;
            depth = 1;
            start_byte = index;
            continue;
        }

        if in_string {
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            continue;
        }

        if ch == '{' {
            depth = depth.saturating_add(1);
            continue;
        }

        if ch == '}' {
            depth = depth.saturating_sub(1);
            if depth == 0 {
                return json_str[start_byte..index + ch.len_utf8()]
                    .trim()
                    .to_string();
            }
        }
    }

    json_str.to_string()
}

fn parse_structured_summary_inner(json_str: &str) -> anyhow::Result<StructuredSummary> {
    let mut value = serde_json::from_str::<serde_json::Value>(json_str)?;
    if value.get("facets").is_none() {
        if let Some(nested) = value.get("summary").and_then(|summary| summary.as_object()) {
            if nested.get("facets").is_some() {
                value = serde_json::Value::Object(nested.clone());
            }
        }
    }
    if let Some(object) = value.as_object_mut() {
        if !object
            .get("summary")
            .is_some_and(|summary| summary.is_string())
        {
            object.insert(
                "summary".to_string(),
                serde_json::Value::String(String::new()),
            );
        }
    }
    serde_json::from_value::<StructuredSummary>(value).map_err(Into::into)
}

fn strip_json_trailing_commas(json_str: &str) -> String {
    let mut out = String::with_capacity(json_str.len());
    let mut chars = json_str.chars().peekable();
    let mut in_string = false;
    let mut escaped = false;

    while let Some(ch) = chars.next() {
        if in_string {
            out.push(ch);
            if escaped {
                escaped = false;
            } else if ch == '\\' {
                escaped = true;
            } else if ch == '"' {
                in_string = false;
            }
            continue;
        }

        if ch == '"' {
            in_string = true;
            out.push(ch);
            continue;
        }

        if ch == ',' {
            let mut lookahead = chars.clone();
            while matches!(lookahead.peek(), Some(next) if next.is_whitespace()) {
                lookahead.next();
            }
            if matches!(lookahead.peek(), Some('}' | ']')) {
                continue;
            }
        }

        out.push(ch);
    }

    out
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
    fn system_prompt_describes_plain_text_output() {
        // When structured_facets is disabled, the prompt asks for plain prose.
        let p = system_prompt(4096, false);
        assert!(!p.contains("4096"));
        assert!(!p.contains("Stay well under"));
        assert!(!p.contains("\"summary\""));
        assert!(p.to_lowercase().contains("no commentary"));
        assert!(p.to_lowercase().contains("no json"));
    }

    #[test]
    fn extends_prompt_when_flag_enabled() {
        let p = system_prompt(4096, true);
        // When structured_facets is true, the prompt should contain the JSON fence instruction.
        assert!(
            p.contains("```json"),
            "should contain JSON fence instruction"
        );
        assert!(p.contains("\"facets\""), "should mention the facets array");
        assert!(
            !p.contains("\"summary\""),
            "structured JSON should not duplicate the prose summary"
        );
        assert!(
            p.contains("evidence_chunks"),
            "should mention evidence_chunks"
        );
        assert!(
            p.contains("must include confidence"),
            "should require confidence for every facet"
        );
        assert!(
            p.contains("under 80 characters"),
            "should keep facet values compact"
        );
        assert!(
            p.contains("Cap the facets array at 4 items"),
            "should keep structured JSON compact"
        );
        assert!(
            p.contains("canonical keys"),
            "should specify canonical keys"
        );
    }

    #[test]
    fn parses_well_formed_response() {
        let raw = "The user prefers pnpm.\n\n\
                   ```json\n\
                   {\"summary\": \"The user prefers pnpm.\", \"facets\": [\
                   {\"class\": \"tooling\", \"key\": \"package_manager\", \
                   \"value\": \"pnpm\", \"evidence_chunks\": [\"c1\"], \
                   \"confidence\": 0.9, \"cue_family\": \"explicit\"}\
                   ]}\n\
                   ```";
        let (prose, maybe) = split_structured_response(raw);
        assert!(
            prose.contains("prefers pnpm"),
            "prose should precede the JSON block"
        );
        let parsed = maybe
            .expect("should find JSON block")
            .expect("should parse");
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
    }

    #[test]
    fn keeps_valid_facets_when_one_facet_is_malformed() {
        let raw = "The user prefers pnpm and terse replies.\n\n\
                   ```json\n\
                   {\"summary\": \"The user prefers pnpm and terse replies.\", \"facets\": [\
                   {\"class\": \"tooling\", \"key\": \"package_manager\", \
                   \"value\": \"pnpm\", \"evidence_chunks\": [\"c1\"], \
                   \"confidence\": 0.9, \"cue_family\": \"explicit\"},\
                   {\"class\": \"style\", \"key\": \"verbosity\", \
                   \"value\": \"terse\", \"evidence_chunks\": [\"c2\"], \
                   \"cue_family\": \"explicit\"}\
                   ]}\n\
                   ```";

        let (prose, maybe) = split_structured_response(raw);
        assert!(prose.contains("prefers pnpm"));
        let parsed = maybe
            .expect("should find JSON block")
            .expect("should parse with malformed facet dropped");
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
    }

    #[test]
    fn gracefully_falls_back_on_invalid_json() {
        let raw = "Summary text.\n\n```json\nnot valid json\n```";
        let (prose, maybe) = split_structured_response(raw);
        assert!(prose.contains("Summary"), "prose should be extracted");
        let result = maybe.expect("fence found");
        assert!(result.is_err(), "invalid JSON should produce Err");
    }

    #[test]
    fn repairs_common_trailing_commas_in_structured_response() {
        let raw = "The user prefers pnpm.\n\n\
                   ```json\n\
                   {\"facets\":[\
                   {\"class\":\"tooling\",\"key\":\"package_manager\",\
                   \"value\":\"pnpm\",\"evidence_chunks\":[\"c1\",],\
                   \"confidence\":0.9,\"cue_family\":\"explicit\",},\
                   ],}\n\
                   ```";
        let (_prose, maybe) = split_structured_response(raw);
        let parsed = maybe
            .expect("should find JSON block")
            .expect("trailing commas should be repaired");
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
    }

    #[test]
    fn parses_facets_only_structured_response() {
        let raw = "The user prefers pnpm.\n\n\
                   ```json\n\
                   {\"facets\":[{\"class\":\"tooling\",\"key\":\"package_manager\",\
                   \"value\":\"pnpm\",\"evidence_chunks\":[\"c1\"],\
                   \"confidence\":0.9,\"cue_family\":\"explicit\"}]}\n\
                   ```";
        let (prose, maybe) = split_structured_response(raw);
        assert!(prose.contains("prefers pnpm"));
        let parsed = maybe
            .expect("should find JSON block")
            .expect("facets-only JSON should parse");
        assert!(parsed.summary.is_empty());
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
    }

    #[test]
    fn ignores_non_string_summary_when_facets_are_present() {
        let raw = "The user prefers pnpm.\n\n\
                   ```json\n\
                   {\"summary\":{\"ignored\":\"object\"},\"facets\":[\
                   {\"class\":\"tooling\",\"key\":\"package_manager\",\
                   \"value\":\"pnpm\",\"evidence_chunks\":[\"c1\"],\
                   \"confidence\":0.9,\"cue_family\":\"explicit\"}]}\n\
                   ```";
        let (_prose, maybe) = split_structured_response(raw);
        let parsed = maybe
            .expect("should find JSON block")
            .expect("non-string summary should not reject facets");
        assert!(parsed.summary.is_empty());
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
    }

    #[test]
    fn accepts_facets_nested_under_summary_object() {
        let raw = "The user prefers pnpm.\n\n\
                   ```json\n\
                   {\"summary\":{\"facets\":[\
                   {\"class\":\"tooling\",\"key\":\"package_manager\",\
                   \"value\":\"pnpm\",\"evidence_chunks\":[\"c1\"],\
                   \"confidence\":0.9,\"cue_family\":\"explicit\"}]}}\n\
                   ```";
        let (_prose, maybe) = split_structured_response(raw);
        let parsed = maybe
            .expect("should find JSON block")
            .expect("nested facets should be promoted");
        assert!(parsed.summary.is_empty());
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
    }

    #[test]
    fn trailing_comma_repair_ignores_commas_inside_strings() {
        let raw = "```json\n\
                   {\"summary\":\"Keep commas, ] and } inside this string.\",\
                   \"facets\":[],}\n\
                   ```";
        let (_prose, maybe) = split_structured_response(raw);
        let parsed = maybe
            .expect("should find JSON block")
            .expect("trailing object comma should be repaired");
        assert_eq!(parsed.summary, "Keep commas, ] and } inside this string.");
    }

    #[test]
    fn ignores_text_after_first_json_object() {
        let raw = "The user prefers pnpm.\n\n\
                   ```json\n\
                   {\"facets\":[{\"class\":\"tooling\",\"key\":\"package_manager\",\
                   \"value\":\"pnpm\",\"evidence_chunks\":[\"c1\"],\
                   \"confidence\":0.9,\"cue_family\":\"explicit\"}]}\n\
                   Extra explanation that should not be parsed.\n\
                   ```";
        let (_prose, maybe) = split_structured_response(raw);
        let parsed = maybe
            .expect("should find JSON block")
            .expect("trailing text after JSON object should be ignored");
        assert_eq!(parsed.facets.len(), 1);
        assert_eq!(parsed.facets[0].key, "package_manager");
    }

    #[test]
    fn respects_disabled_flag() {
        let p = system_prompt(4096, false);
        assert!(
            !p.contains("```json"),
            "disabled flag must omit JSON instruction"
        );
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

    /// Helper config with structured facet extraction disabled for legacy tests.
    fn no_facets_cfg() -> LlmSummariserConfig {
        LlmSummariserConfig {
            model: "qwen2.5:0.5b".into(),
            structured_facet_extraction: false,
        }
    }

    #[tokio::test]
    async fn empty_inputs_yield_empty_summary_without_provider_call() {
        // All inputs are blank → prompt body is empty → the summariser
        // short-circuits and returns an empty output without invoking the
        // chat provider.
        let provider = std::sync::Arc::new(StubProvider::ok("never returned"));
        let s = LlmSummariser::new(no_facets_cfg(), provider.clone());
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
        let s = LlmSummariser::new(no_facets_cfg(), provider);
        let inputs = vec![sample_input("a", "alice decided to ship friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert!(out.content.contains("alice decided to ship"));
        assert!(out.entities.is_empty());
        assert!(out.topics.is_empty());
    }

    #[tokio::test]
    async fn provider_summary_response_is_used_and_clamped() {
        // Provider returns plain text; summariser uses it verbatim
        // (after trim) and clamps to the budget.
        let provider = std::sync::Arc::new(StubProvider::ok("alice decided to ship friday\n"));
        let s = LlmSummariser::new(no_facets_cfg(), provider.clone());
        let inputs = vec![sample_input("a", "alice ships friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();
        assert_eq!(out.content, "alice decided to ship friday");
        assert!(out.token_count > 0);
        assert_eq!(provider.calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn json_only_structured_response_uses_structured_summary_text() {
        let provider = std::sync::Arc::new(StubProvider::ok(
            "```json\n{\"summary\":\"alice decided to ship friday\",\"facets\":[]}\n```",
        ));
        let s = LlmSummariser::new(
            LlmSummariserConfig {
                model: "qwen2.5:0.5b".into(),
                structured_facet_extraction: true,
            },
            provider,
        );
        let inputs = vec![sample_input("a", "alice ships friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();

        assert_eq!(out.content, "alice decided to ship friday");
        assert!(out.token_count > 0);
    }

    #[tokio::test]
    async fn invalid_json_without_prose_falls_back_to_inert_summary() {
        let provider = std::sync::Arc::new(StubProvider::ok("```json\n{\"summary\":"));
        let s = LlmSummariser::new(
            LlmSummariserConfig {
                model: "qwen2.5:0.5b".into(),
                structured_facet_extraction: true,
            },
            provider,
        );
        let inputs = vec![sample_input("a", "alice ships friday")];
        let out = s.summarise(&inputs, &test_ctx()).await.unwrap();

        assert!(out.content.contains("alice ships friday"));
        assert!(out.token_count > 0);
    }

    #[test]
    fn build_prompt_carries_body_and_kind_tag() {
        let provider = std::sync::Arc::new(StubProvider::ok("hi"));
        // With structured_facet_extraction disabled, expect plain-text prompt.
        let s = LlmSummariser::new(
            LlmSummariserConfig {
                model: "llama3.1:8b".into(),
                structured_facet_extraction: false,
            },
            provider,
        );
        let prompt = s.build_prompt("body", 2048);
        assert!(prompt.system.to_lowercase().contains("no commentary"));
        assert!(!prompt.system.contains("\"summary\""));
        assert_eq!(prompt.user, "body");
        assert_eq!(prompt.temperature, 0.0);
        assert_eq!(prompt.kind, "memory_tree::summarise");
    }
}
