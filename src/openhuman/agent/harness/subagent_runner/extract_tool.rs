//! `extract_from_result` — a sub-agent-side tool that answers a targeted
//! query against a payload previously stashed by the handoff cache (see
//! [`super::handoff`]).
//!
//! This used to dispatch the `summarizer` archetype as a full sub-agent.
//! That dragged along system-prompt scaffolding, a tool-loop, and an
//! extra inference round for a workload that really only needs one
//! completion call. So the tool now drives `provider.chat_with_system`
//! directly. Both the provider AND the model id are resolved by the runner
//! through the `summarization` role (`create_chat_provider("summarization")`)
//! and handed in, so this extraction follows the user's `memory_provider`
//! routing — managed (`summarization-v1`), BYOK, or local — exactly like every
//! other summarization path, instead of borrowing the parent agent's provider
//! with a hardcoded tier string (which 400'd on BYOK/local providers that don't
//! know the literal `summarization-v1`).
//!
//! Transcript discipline: the LLM call still costs tokens, so every
//! extraction round-trip is persisted as its own `session_raw/` JSONL (+
//! companion `.md`) under the parent's session chain. Single-shot calls
//! produce one file; chunked calls produce one file per chunk sharing a
//! common `call_seq`. Transcript failures are warnings — they never
//! block the tool result.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex as StdMutex};

use async_trait::async_trait;
use futures::stream::StreamExt;
use serde_json::{json, Value};

use super::handoff::{chunk_content, ResultHandoffCache, HANDOFF_MAX_ENTRIES};
use crate::openhuman::agent::harness::session::transcript::{
    resolve_keyed_transcript_path, write_transcript, MessageUsage, TranscriptMeta, TurnUsage,
};
use crate::openhuman::inference::provider::{ChatMessage, Provider};
use crate::openhuman::tools::{Tool, ToolCategory, ToolResult};

// ── Tunables ──────────────────────────────────────────────────────────

/// Temperature for extraction calls. Low but non-zero so the model can
/// pick reasonable phrasings when rewriting identifiers into a compact
/// answer, without straying into creative territory.
const EXTRACT_TEMPERATURE: f64 = 0.2;

/// Convert a context window (tokens) into the per-chunk char budget. A payload
/// at or under this budget is extracted in a single shot over its **entire**
/// content — higher quality than the chunk+concat fallback, which has no reduce
/// stage and can miss facts that span a chunk boundary. Headroom is reserved for
/// the extraction contract, the query, and the response.
///
/// `window_tokens = None` means neither the provider nor the static registry
/// could size the model — only reached for **cloud** models the registry doesn't
/// know (an unknown *local* model resolves to its small provider-profile window
/// via [`ExtractFromResultTool::extract_chunk_char_budget`], not here), so a
/// large window is a safe assumption.
fn chunk_char_budget_for_window(window_tokens: Option<u64>) -> usize {
    /// Last-resort window (tokens) when the model is unsizable — see above.
    const FALLBACK_WINDOW_TOKENS: u64 = 128_000;
    /// Approximate chars per token used for budgeting.
    const CHARS_PER_TOKEN: u64 = 4;
    /// Fraction of the window spent on the payload slice; the remainder covers
    /// the prompt scaffolding, query, and model response.
    const USABLE_PCT: u64 = 70;

    let window = window_tokens.unwrap_or(FALLBACK_WINDOW_TOKENS);
    (window * USABLE_PCT / 100 * CHARS_PER_TOKEN) as usize
}

/// System prompt fed to the provider on every `extract_from_result`
/// call. Lifted in spirit from the old `summarizer` agent's prompt but
/// trimmed to the core extraction contract — no fluff about iteration
/// budgets or sub-agent roles because this is a pure tool call.
const EXTRACT_SYSTEM_PROMPT: &str = "\
You are an extraction assistant. A larger tool output is provided below. \
Return ONLY the specific facts the user's query asks for. \
Preserve identifiers verbatim (ids, urls, emails, timestamps, prices). \
Be compact: no preamble, no commentary, no apologies, no meta-statements. \
If the payload contains nothing relevant to the query, reply with an \
empty string — do not invent information.";

// ── Tool impl ─────────────────────────────────────────────────────────

/// The `extract_from_result` tool registered into the sub-agent's tool
/// surface when a handoff cache is active (currently: integrations_agent
/// with a toolkit scope).
pub(super) struct ExtractFromResultTool {
    cache: Arc<ResultHandoffCache>,
    provider: Arc<dyn Provider>,
    /// Model id for the extraction `chat_with_system` calls. Resolved by the
    /// runner through the `summarization` role (alongside `provider`), so it
    /// tracks the user's `memory_provider` routing + `cloud_llm_model` override
    /// instead of a hardcoded tier string.
    model: String,
    /// Workspace root for transcript writes.
    workspace_dir: PathBuf,
    /// Parent session chain joined with `__`, e.g.
    /// `"1700000000_orchestrator__1700000005_1234_integrations_agent_abc"`.
    /// Extract-call transcripts append a unique per-call suffix to this.
    parent_chain: String,
    /// Logical agent id that owns the calls (e.g. `"integrations_agent"`).
    /// Only used to compose a descriptive `agent_name` in transcript meta.
    owner_agent_id: String,
    /// Monotonic counter so repeated calls within the same millisecond
    /// still land on distinct transcript files.
    call_seq: StdMutex<u64>,
}

impl ExtractFromResultTool {
    pub(super) fn new(
        cache: Arc<ResultHandoffCache>,
        provider: Arc<dyn Provider>,
        model: String,
        workspace_dir: PathBuf,
        parent_chain: String,
        owner_agent_id: String,
    ) -> Self {
        Self {
            cache,
            provider,
            model,
            workspace_dir,
            parent_chain,
            owner_agent_id,
            call_seq: StdMutex::new(0),
        }
    }

    /// Resolve the per-chunk char budget for `self.model` against the chosen
    /// provider's context window.
    ///
    /// Asks the **provider** first: a local runtime (Ollama / LM Studio) reports
    /// its real loaded / profile window here (~8k tokens for Ollama), so an
    /// unknown *local* model is budgeted against its actual small context and the
    /// payload is chunked — instead of assuming a 128k window and sending an
    /// oversized single-shot prompt that overflows the local context (Codex P2).
    /// Falls back to the static registry, then the cloud-safe default in
    /// [`chunk_char_budget_for_window`].
    async fn extract_chunk_char_budget(&self) -> usize {
        let window = self
            .provider
            .effective_context_window(&self.model)
            .await
            .or_else(|| crate::openhuman::inference::context_window_for_model(&self.model));
        chunk_char_budget_for_window(window)
    }

    fn next_call_seq(&self) -> u64 {
        let mut guard = self
            .call_seq
            .lock()
            .expect("extract_from_result call_seq mutex poisoned");
        *guard = guard.saturating_add(1);
        *guard
    }
}

#[async_trait]
impl Tool for ExtractFromResultTool {
    fn name(&self) -> &str {
        "extract_from_result"
    }

    fn description(&self) -> &str {
        "Answer a targeted question against an oversized tool output that was \
         stashed under a `result_id` handle. Use this when a previous tool call \
         returned a placeholder like `result_id=\"res_1\"` because its raw output \
         was too large to show inline. Pass the handle plus a natural-language \
         `query` naming the exact facts/identifiers you need; returns only the \
         extracted answer, not the full payload. Multiple queries against the \
         same `result_id` are allowed — each one is independent."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "result_id": {
                    "type": "string",
                    "description": "The handle emitted in the oversized tool output placeholder (e.g. `res_1`)."
                },
                "query": {
                    "type": "string",
                    "description": "Natural-language question naming the exact facts or identifiers to extract. Be specific."
                }
            },
            "required": ["result_id", "query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let result_id = args.get("result_id").and_then(|v| v.as_str()).unwrap_or("");
        let query = args.get("query").and_then(|v| v.as_str()).unwrap_or("");

        if result_id.is_empty() || query.is_empty() {
            return Ok(ToolResult::error(
                "extract_from_result requires non-empty `result_id` and `query`.",
            ));
        }

        let cached = match self.cache.get(result_id) {
            Some(c) => c,
            None => {
                return Ok(ToolResult::error(format!(
                    "No cached result found for id '{result_id}'. The handle may have been evicted (cache holds the {HANDOFF_MAX_ENTRIES} most recent entries). Re-run the original tool to get a fresh handle."
                )));
            }
        };

        // Allow test harnesses to lower the chunk budget so multi-chunk
        // extraction can be exercised on compacted payloads. Never consulted
        // in production (env var absent).
        let effective_chunk_budget = match std::env::var("OPENHUMAN_TEST_EXTRACT_CHUNK_BUDGET")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
        {
            Some(budget) => budget,
            None => self.extract_chunk_char_budget().await,
        };

        // Fast path: payload fits in a single provider turn.
        if cached.content.len() <= effective_chunk_budget {
            tracing::debug!(
                tool = %cached.tool_name,
                bytes = cached.content.len(),
                "[extract_from_result] single-shot extraction"
            );
            return self
                .extract_single_shot(&cached.tool_name, &cached.content, query)
                .await;
        }

        // Slow path: chunk + parallel map. A single call on a payload
        // large enough to need the handoff (hundreds of KB common for
        // Gmail / Notion list operations) risks either (a) overflowing
        // the extraction model's context window, or (b) a low-quality
        // single-pass answer that misses facts near the tail. Splitting
        // into budgeted chunks and running them in parallel keeps each
        // call under its context budget and usually finishes faster
        // than a sequential single-shot call on the whole blob.
        //
        // No reduce stage: per-chunk extracts are concatenated in
        // original chunk order. A reduce LLM call adds latency (often
        // the slowest single turn) and becomes a single point of
        // failure when the upstream provider stalls. For
        // listing/extraction queries concatenation is equivalent; for
        // top-N / global-ordering queries the caller can post-process.
        let chunks = chunk_content(&cached.content, effective_chunk_budget);
        tracing::info!(
            tool = %cached.tool_name,
            total_bytes = cached.content.len(),
            chunk_count = chunks.len(),
            chunk_budget = effective_chunk_budget,
            "[extract_from_result] chunked extraction"
        );

        // Map stage: each chunk extracts items matching `query` from
        // ITS OWN slice only. Dispatched with bounded concurrency —
        // `buffer_unordered(MAP_CONCURRENCY)` keeps at most N calls in
        // flight at any time. Fully parallel `join_all` was generating
        // 504-gateway-timeout storms from the staging proxy when 7+
        // concurrent calls piled onto the upstream; batching at 3
        // trades some wall-clock time for reliability.
        const MAP_CONCURRENCY: usize = 3;
        let total_chunks = chunks.len();

        // Each chunk gets its own monotonic call_seq so sibling
        // transcripts written in parallel still land on distinct files.
        let call_seq_base = self.next_call_seq();
        let workspace_dir = self.workspace_dir.clone();
        let parent_chain = self.parent_chain.clone();
        let owner_agent_id = self.owner_agent_id.clone();
        let model = self.model.clone();

        // Consume `chunks` with `into_iter` so each async block owns
        // its `String` — `buffer_unordered` polls the stream lazily
        // and needs futures with no borrows into the enclosing scope.
        let map_futures = chunks.into_iter().enumerate().map(|(i, chunk)| {
            let provider = self.provider.clone();
            let tool_name = cached.tool_name.clone();
            let query = query.to_string();
            let workspace_dir = workspace_dir.clone();
            let parent_chain = parent_chain.clone();
            let owner_agent_id = owner_agent_id.clone();
            let model = model.clone();
            async move {
                let user_prompt = format!(
                    "Tool name: {tool_name}\nChunk {idx} of {total}\n\n\
                     Query: {query}\n\n\
                     This is one slice of a larger tool output. Extract ONLY \
                     items in THIS slice that match the query. Preserve \
                     identifiers verbatim. Return an empty string if nothing \
                     in this slice is relevant.\n\n\
                     --- BEGIN SLICE ---\n{chunk}\n--- END SLICE ---",
                    idx = i + 1,
                    total = total_chunks,
                );
                let result = provider
                    .chat_with_system(
                        Some(EXTRACT_SYSTEM_PROMPT),
                        &user_prompt,
                        &model,
                        EXTRACT_TEMPERATURE,
                    )
                    .await;

                // Persist this chunk's transcript before returning, so
                // a partial failure higher up the stream still leaves
                // an auditable record on disk.
                let transcript_input: Result<&str, String> = match &result {
                    Ok(text) => Ok(text.as_str()),
                    Err(e) => Err(e.to_string()),
                };
                let chunk_label = format!("chunk{:03}of{:03}", i + 1, total_chunks);
                write_extract_transcript(
                    &workspace_dir,
                    &parent_chain,
                    &owner_agent_id,
                    call_seq_base,
                    Some(&chunk_label),
                    EXTRACT_SYSTEM_PROMPT,
                    &user_prompt,
                    match &transcript_input {
                        Ok(s) => Ok(*s),
                        Err(s) => Err(s.as_str()),
                    },
                    &model,
                );

                (i, result)
            }
        });

        let mut map_results: Vec<(usize, _)> = futures::stream::iter(map_futures)
            .buffer_unordered(MAP_CONCURRENCY)
            .collect()
            .await;
        // `buffer_unordered` yields futures in completion order; restore
        // original chunk order so the concatenated output matches the
        // natural ordering of the underlying tool result (e.g. Notion's
        // reverse-chrono page list).
        map_results.sort_by_key(|(i, _)| *i);

        let partials: Vec<String> = map_results
            .into_iter()
            .filter_map(|(i, r)| match r {
                Ok(text) => {
                    let trimmed = text.trim();
                    if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    }
                }
                Err(e) => {
                    tracing::warn!(
                        chunk_idx = i,
                        error = %e,
                        "[extract_from_result] map-stage provider call failed; dropping partial"
                    );
                    None
                }
            })
            .collect();

        if partials.is_empty() {
            tracing::debug!(
                "[extract_from_result] no matching content found across any chunk; returning empty extraction"
            );
            return Ok(ToolResult::success(String::new()));
        }

        // Concatenate per-chunk summaries in original chunk order.
        // `join` with a single partial yields it unchanged (no trailing
        // separator), so no special-case is needed.
        Ok(ToolResult::success(partials.join("\n\n---\n\n")))
    }
}

impl ExtractFromResultTool {
    async fn extract_single_shot(
        &self,
        tool_name: &str,
        content: &str,
        query: &str,
    ) -> anyhow::Result<ToolResult> {
        let user_prompt = format!(
            "Tool name: {tool_name}\n\nQuery: {query}\n\n\
             Raw tool output follows. Extract ONLY the information the query \
             asks for.\n\n\
             --- BEGIN ---\n{content}\n--- END ---",
        );

        let call_seq = self.next_call_seq();
        let provider_result = self
            .provider
            .chat_with_system(
                Some(EXTRACT_SYSTEM_PROMPT),
                &user_prompt,
                &self.model,
                EXTRACT_TEMPERATURE,
            )
            .await;

        // Persist the transcript before returning — the LLM call cost
        // tokens regardless of whether we ultimately return success.
        let transcript_input: Result<&str, String> = match &provider_result {
            Ok(text) => Ok(text.as_str()),
            Err(e) => Err(e.to_string()),
        };
        write_extract_transcript(
            &self.workspace_dir,
            &self.parent_chain,
            &self.owner_agent_id,
            call_seq,
            None,
            EXTRACT_SYSTEM_PROMPT,
            &user_prompt,
            match &transcript_input {
                Ok(s) => Ok(*s),
                Err(s) => Err(s.as_str()),
            },
            &self.model,
        );

        match provider_result {
            Ok(text) => {
                let trimmed = text.trim();
                if trimmed.is_empty() {
                    tracing::debug!(
                        "[extract_from_result] provider returned an empty response; returning empty extraction"
                    );
                    Ok(ToolResult::success(String::new()))
                } else {
                    Ok(ToolResult::success(trimmed.to_string()))
                }
            }
            Err(e) => Ok(ToolResult::error(format!(
                "extract_from_result: provider call failed: {e}"
            ))),
        }
    }
}

// ── Transcript writer ─────────────────────────────────────────────────

/// Persist a single extract-from-result LLM round-trip as its own
/// transcript file under `session_raw/DDMMYYYY/{stem}.jsonl` (+ `.md`).
///
/// Best-effort: transcript failures are logged and swallowed so a
/// readable-log hiccup never blocks the extraction itself. Appends a
/// short suffix to the parent chain so every call lands on a distinct
/// file (sibling extract calls within the same tool invocation still
/// get unique stems).
fn write_extract_transcript(
    workspace_dir: &Path,
    parent_chain: &str,
    owner_agent_id: &str,
    call_seq: u64,
    chunk_label: Option<&str>,
    system_prompt: &str,
    user_prompt: &str,
    assistant_output: Result<&str, &str>,
    model: &str,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default();
    let unix_ts = now.as_secs();
    let nanos = now.subsec_nanos();
    let chunk_tag = match chunk_label {
        Some(label) => format!("_{label}"),
        None => String::new(),
    };
    let stem = format!("{parent_chain}__extract_{unix_ts}_{nanos:09}_{call_seq:04}{chunk_tag}");

    let path = match resolve_keyed_transcript_path(workspace_dir, &stem) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(
                error = %e,
                stem = %stem,
                "[extract_from_result] could not resolve transcript path; skipping transcript"
            );
            return;
        }
    };

    let (assistant_text, is_error) = match assistant_output {
        Ok(text) => (text.to_string(), false),
        Err(err) => (format!("[error] {err}"), true),
    };

    let messages = vec![
        ChatMessage {
            id: None,
            role: "system".into(),
            content: system_prompt.to_string(),
            extra_metadata: None,
        },
        ChatMessage {
            id: None,
            role: "user".into(),
            content: user_prompt.to_string(),
            extra_metadata: None,
        },
        ChatMessage {
            id: None,
            role: "assistant".into(),
            content: assistant_text,
            extra_metadata: None,
        },
    ];

    // Token counts aren't surfaced by `chat_with_system`; leave cost /
    // usage fields zeroed and let the backend's own telemetry fill in
    // the blanks when we wire richer accounting later.
    let ts_rfc3339 = chrono::Utc::now().to_rfc3339();
    let turn_usage = TurnUsage {
        provider: "extract_from_result".to_string(),
        model: model.to_string(),
        usage: MessageUsage {
            input: 0,
            output: 0,
            cached_input: 0,
            context_window: 0,
            cost_usd: 0.0,
        },
        ts: ts_rfc3339.clone(),
        reasoning_content: None,
        tool_calls: Vec::new(),
        iteration: 1,
    };

    let meta = TranscriptMeta {
        agent_name: format!("{owner_agent_id}::extract_from_result"),
        agent_id: Some(owner_agent_id.to_string()),
        agent_type: Some("extractor".to_string()),
        dispatcher: "native".into(),
        provider: Some(turn_usage.provider.clone()),
        model: Some(turn_usage.model.clone()),
        created: ts_rfc3339.clone(),
        updated: ts_rfc3339,
        turn_count: 1,
        input_tokens: 0,
        output_tokens: 0,
        cached_input_tokens: 0,
        charged_amount_usd: 0.0,
        thread_id: crate::openhuman::inference::provider::thread_context::current_thread_id(),
        task_id: None,
    };

    if let Err(e) = write_transcript(&path, &messages, &meta, Some(&turn_usage)) {
        tracing::warn!(
            error = %e,
            path = %path.display(),
            "[extract_from_result] transcript write failed"
        );
    } else {
        tracing::debug!(
            path = %path.display(),
            is_error,
            "[extract_from_result] transcript written"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The chunk budget tracks the resolved context window, so a small local
    // window yields a much smaller budget than a long-context cloud tier — this
    // is what forces chunking instead of an oversized single-shot prompt.
    #[test]
    fn chunk_budget_tracks_context_window() {
        let summarization_window =
            crate::openhuman::inference::context_window_for_model("summarization-v1");
        let big = chunk_char_budget_for_window(summarization_window);
        let small = chunk_char_budget_for_window(Some(8_192)); // Ollama local default
        assert!(
            big > small,
            "long-context tier budget {big} must exceed an 8k local window budget {small}"
        );
    }

    // Codex P2: an unknown LOCAL model resolves (via the provider) to its small
    // ~8k profile window, NOT the 128k cloud fallback. The resulting budget must
    // be well under a production handoff payload (~200k chars) so it chunks
    // instead of single-shotting into a local context overflow.
    #[test]
    fn chunk_budget_for_small_local_window_forces_chunking() {
        let budget = chunk_char_budget_for_window(Some(8_192));
        // 8192 * 70% * 4 = 22_937 chars.
        assert_eq!(budget, (8_192u64 * 70 / 100 * 4) as usize);
        assert!(
            budget < 200_000,
            "an 8k local window must budget below a typical handoff payload so it chunks"
        );
    }

    // When neither provider nor registry can size the model (cloud-unknown), the
    // cloud-safe 128k fallback applies.
    #[test]
    fn chunk_budget_uses_cloud_fallback_when_unsizable() {
        let expected = (128_000u64 * 70 / 100 * 4) as usize;
        assert_eq!(chunk_char_budget_for_window(None), expected);
    }
}
