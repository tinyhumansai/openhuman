use super::*;

/// Definition id of the pure-retrieval memory agent, reached from chat as the
/// `retrieve_memory` delegate and from other agents as `call_memory_agent`.
pub(super) const AGENT_MEMORY_ID: &str = "agent_memory";

/// How many deterministic hits the memory fast path returns (#4677).
pub(super) const MEMORY_FAST_PATH_LIMIT: usize = 8;

/// Whether the deterministic memory fast path (#4677) is enabled. Default on;
/// `OPENHUMAN_MEMORY_FAST_PATH=0` (or `false`/`no`/`off`) forces the full
/// model-driven walk, e.g. to A/B the two paths without a rebuild.
pub(super) fn memory_fast_path_enabled() -> bool {
    parse_memory_fast_path_enabled(std::env::var("OPENHUMAN_MEMORY_FAST_PATH").ok().as_deref())
}

/// Pure core of [`memory_fast_path_enabled`], kept env-free for deterministic
/// unit testing.
pub(super) fn parse_memory_fast_path_enabled(env_value: Option<&str>) -> bool {
    !matches!(
        env_value.map(|v| v.trim().to_ascii_lowercase()).as_deref(),
        Some("0") | Some("false") | Some("no") | Some("off")
    )
}

/// Render deterministic retrieval hits into a compact, citable memory-context
/// block for the parent turn. Returns `None` when there are no hits, so the
/// caller falls back to the model-driven walk (the empty/degraded case is
/// #4655's territory and still benefits from the model's judgement).
pub(super) fn format_deterministic_memory_hits(resp: &RetrievalResponse) -> Option<String> {
    use std::fmt::Write as _;
    if resp.hits.is_empty() {
        return None;
    }
    const PER_HIT_CHARS: usize = 600;
    let mut out = format!(
        "Retrieved {} relevant memor{} via deterministic memory search:\n",
        resp.hits.len(),
        if resp.hits.len() == 1 { "y" } else { "ies" }
    );
    for (i, hit) in resp.hits.iter().enumerate() {
        let content = hit.content.trim();
        let body: String = content.chars().take(PER_HIT_CHARS).collect();
        let ellipsis = if content.chars().count() > PER_HIT_CHARS {
            " …"
        } else {
            ""
        };
        let scope = if hit.tree_scope.trim().is_empty() {
            "memory"
        } else {
            hit.tree_scope.trim()
        };
        let _ = writeln!(
            out,
            "{}. [{scope}] {body}{ellipsis} (relevance {:.2})",
            i + 1,
            hit.score
        );
    }
    Some(out)
}

/// Truncate `output` in place to the definition's `max_result_chars` cap (when
/// set), appending a `[...truncated]` marker. Char-count based (not byte-length)
/// to avoid panicking on a multi-byte UTF-8 sequence at the boundary.
///
/// Shared by the normal sub-agent path and the deterministic memory fast path so
/// both honour a definition's cap. `agent_memory` sets no cap today (its output
/// is self-bounded at 8 hits × 600 chars), but routing the fast path through the
/// same helper keeps the two paths from silently diverging if one is ever added
/// (YellowSnnowmann review).
pub(super) fn apply_max_result_chars(output: &mut String, cap: Option<usize>, agent_id: &str) {
    let Some(cap) = cap else { return };
    let original_chars = output.chars().count();
    if original_chars <= cap {
        return;
    }
    tracing::debug!(
        agent_id = %agent_id,
        original_chars,
        cap,
        "[subagent_runner] truncating oversized result to max_result_chars cap"
    );
    let byte_offset = output
        .char_indices()
        .nth(cap)
        .map(|(i, _)| i)
        .unwrap_or(output.len());
    output.truncate(byte_offset);
    output.push_str("\n[...truncated]");
}

/// Deterministic fast path for the pure-retrieval [`AGENT_MEMORY_ID`] sub-agent
/// (#4677).
///
/// `agent_memory` otherwise runs a model-driven walk (≤ its `max_iterations`)
/// whose per-iteration LLM round-trips dominate turn latency at ~30–40s per call
/// *even when data is present*. [`fast_retrieve`] (E2GraphRAG: query-entity +
/// dense/semantic recall over the same memory tree, no LLM in the loop) returns
/// the same hits in a single deterministic pass. When it finds data we return
/// those hits directly; when the fast path is disabled, errors, or finds nothing
/// we return `None` so the caller runs the full sub-agent unchanged.
///
/// # Relevance guard (Codex review)
///
/// We only short-circuit for an **entity-grounded** query — one that yields at
/// least one canonical entity or salient topic. Without grounding, `fast_retrieve`
/// falls back to a pure global-dense pass that reranks/truncates whatever
/// summaries exist, so a vague query against a populated profile would surface
/// unrelated top-k memories as a "completed" retrieval instead of letting the
/// model-driven agent judge relevance (or emit "no relevant memory found").
/// Grounded queries keep the fast path; ungrounded ones defer to the full agent.
pub(super) async fn try_deterministic_memory_retrieval(
    task_prompt: &str,
    definition: &AgentDefinition,
    task_id: &str,
    started: Instant,
    loaded_config: &LoadedConfig,
) -> Option<SubagentRunOutcome> {
    let agent_id = definition.id.as_str();
    if !memory_fast_path_enabled() {
        return None;
    }
    let query = task_prompt.trim();
    if query.is_empty() {
        return None;
    }
    let config = match loaded_config.as_ref() {
        Ok(config) => config.as_ref(),
        Err(e) => {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "[subagent_runner] agent_memory fast-path config load failed — falling back to model walk (#4677)"
            );
            return None;
        }
    };
    // Relevance guard (Codex review): require entity/topic grounding before a
    // deterministic pass stands in for the model's relevance judgement — the
    // extraction is cheap (regex or one spaCy call) and `fast_retrieve` repeats
    // it internally anyway. It goes through the provider's scoring family so
    // the host no longer calls `tinymemory_core::` directly, and every failure
    // (binding unavailable, scoring not exposed, extraction error) is fail-safe
    // as entities_empty = true: the fast path is skipped and the model-driven
    // walk runs — the same conservative outcome an unavailable extractor
    // produced before scoring existed.
    let entities_empty = match crate::memory::binding::for_config(config) {
        Ok(binding) => match binding.provider().as_scoring() {
            Some(scoring) => match scoring.extract_entities(query).await {
                Ok(entities) => entities.is_empty(),
                Err(e) => {
                    tracing::debug!(
                        task_id = %task_id,
                        error = %e,
                        "[subagent_runner] scoring extract_entities failed (non-fatal) — deferring to model walk (#4677)"
                    );
                    true
                }
            },
            None => {
                tracing::debug!(
                    task_id = %task_id,
                    "[subagent_runner] driver does not expose scoring (module not loaded or policy excluded) — deferring to model walk (#4677)"
                );
                true
            }
        },
        Err(e) => {
            tracing::debug!(
                task_id = %task_id,
                error = %e,
                "[subagent_runner] memory binding unavailable (non-fatal) — deferring to model walk (#4677)"
            );
            true
        }
    };
    if entities_empty {
        tracing::debug!(
            task_id = %task_id,
            "[subagent_runner] agent_memory fast-path skipped — ungrounded query (no entities/topics); deferring to model walk (#4677)"
        );
        return None;
    }
    let opts = FastRetrieveQuery {
        limit: MEMORY_FAST_PATH_LIMIT,
        ..FastRetrieveQuery::default()
    };
    // Through the bound driver's `MemoryRetrieval`, not the engine (#5560).
    // This is an agent turn, so `as_bus_scope()` carries the turn's own
    // memory-source allowlist; `binding.provider()` is unguarded, which makes
    // that argument the gate rather than a hint.
    let scope = as_bus_scope();
    let binding = match crate::memory::binding::for_config(config) {
        Ok(binding) => binding,
        Err(e) => {
            tracing::warn!(
                task_id = %task_id,
                error = %e,
                "[subagent_runner] agent_memory fast-path could not bind the memory driver — falling back to model walk (#4677)"
            );
            return None;
        }
    };
    // A driver with no retrieval family has no summary tree to rank. Falling
    // through to the model walk is the same answer this path already gives for
    // an empty result, and strictly better than reporting a failure that is
    // really an absent capability.
    let retrieval = binding.provider().as_retrieval()?;
    let resp = match retrieval.fast_retrieve(query, opts, scope.as_ref()).await {
        Ok(resp) => resp,
        Err(e) => {
            tracing::warn!(
                task_id = %task_id,
                error = %format!("{e:#}"),
                "[subagent_runner] agent_memory fast-path retrieval errored — falling back to model walk (#4677)"
            );
            return None;
        }
    };
    let mut output = format_deterministic_memory_hits(&resp)?;
    // Honour the definition's `max_result_chars` cap just like the model-driven
    // path (YellowSnnowmann review). No-op for `agent_memory` (uncapped, and the
    // block above is already self-bounded), but keeps the paths from diverging.
    apply_max_result_chars(&mut output, definition.max_result_chars, agent_id);
    tracing::info!(
        task_id = %task_id,
        hits = resp.hits.len(),
        total = resp.total,
        elapsed_ms = started.elapsed().as_millis() as u64,
        "[subagent_runner] agent_memory deterministic fast-path hit — skipped the model walk (#4677)"
    );
    Some(SubagentRunOutcome {
        task_id: task_id.to_string(),
        agent_id: agent_id.to_string(),
        output,
        iterations: 0,
        elapsed: started.elapsed(),
        mode: SubagentMode::Typed,
        status: SubagentRunStatus::Completed,
        final_history: Vec::new(),
        usage: SubagentUsage::default(),
        // Deterministic memory hits are already bounded; nothing is offloaded
        // on this path.
        artifact_paths: Vec::new(),
    })
}
