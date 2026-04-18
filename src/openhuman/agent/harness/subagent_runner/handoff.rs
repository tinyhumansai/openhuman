//! Progressive-disclosure handoff cache for oversized tool results.
//!
//! Typed sub-agents (integrations_agent in particular) regularly call tools
//! that return megabyte-scale payloads — `GMAIL_LIST_MESSAGES`,
//! `NOTION_GET_PAGE`, `GOOGLEDRIVE_LIST_FILES`. The default behaviour pushes
//! that raw blob into the sub-agent's history as a tool-result message, and
//! the NEXT iteration ships the bloated history back to the provider where
//! it hits the model's context-length ceiling.
//!
//! Progressive disclosure fixes this: when a tool returns too much data we
//! stash the full payload here, replace it in history with a short
//! placeholder (size + preview + `result_id` + how to query it), and expose
//! an `extract_from_result` tool (see [`super::extract_tool`]) that the
//! sub-agent can call with a targeted query. The extractor only runs when
//! the sub-agent actually asks for a narrower view.
//!
//! This module owns:
//! * the thresholds and limits (token cut-off, preview size, max entries);
//! * the [`ResultHandoffCache`] store itself (FIFO-evicting, `Arc`-shared);
//! * the [`build_handoff_placeholder`] renderer used when rewriting tool
//!   results into history.

use std::collections::HashMap;
use std::sync::Mutex as StdMutex;

// ── Tunables ───────────────────────────────────────────────────────────────

/// Token threshold above which a tool result is routed to the handoff
/// cache instead of being pushed into history raw. Token count is
/// estimated at ~4 chars/token (mirrors
/// `crate::openhuman::agent::harness::payload_summarizer` and
/// `crate::openhuman::tree_summarizer::types::estimate_tokens`).
///
/// Set at `50_000` so the clean Gmail / Notion envelopes emitted by provider
/// post-processing fit through unchanged for normal workloads — only
/// genuinely oversized results (bulk fetches, raw thread dumps) are routed
/// through the `extract_from_result` path.
pub(super) const HANDOFF_OVERSIZE_THRESHOLD_TOKENS: usize = 50_000;

/// Characters of the raw payload to surface in the placeholder preview.
/// Enough for the sub-agent to recognise the shape (JSON keys, first
/// record) and often small enough to answer trivial questions without a
/// follow-up `extract_from_result` call.
pub(super) const HANDOFF_PREVIEW_CHARS: usize = 1500;

/// Maximum entries per session. Bounded to keep memory use predictable on
/// long-running sub-agents that might call many large tools. When over
/// capacity we evict the oldest entry (FIFO); callers see "no cached
/// result" for evicted ids and can either re-run the tool or ask the
/// user/orchestrator to narrow the request.
pub(super) const HANDOFF_MAX_ENTRIES: usize = 8;

// ── Store ──────────────────────────────────────────────────────────────────

/// Per-spawn cache of oversized tool payloads. One instance is built at
/// the top of `run_typed_mode` and shared (via `Arc`) with both the inner
/// tool-call loop (writes) and the `extract_from_result` tool (reads).
#[derive(Default)]
pub(super) struct ResultHandoffCache {
    inner: StdMutex<HandoffInner>,
}

#[derive(Default)]
struct HandoffInner {
    /// FIFO of inserted ids, used for eviction.
    order: Vec<String>,
    /// Content by id.
    entries: HashMap<String, CachedResult>,
    /// Monotonic counter for id generation within the session.
    next_id: u64,
}

pub(super) struct CachedResult {
    pub(super) tool_name: String,
    pub(super) content: String,
}

impl ResultHandoffCache {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// Stash a payload and return a stable, short, grep-friendly id.
    pub(super) fn store(&self, tool_name: String, content: String) -> String {
        let mut g = match self.inner.lock() {
            Ok(g) => g,
            Err(poisoned) => poisoned.into_inner(),
        };
        g.next_id = g.next_id.saturating_add(1);
        let id = format!("res_{:x}", g.next_id);
        g.order.push(id.clone());
        g.entries
            .insert(id.clone(), CachedResult { tool_name, content });
        while g.order.len() > HANDOFF_MAX_ENTRIES {
            let evicted = g.order.remove(0);
            g.entries.remove(&evicted);
        }
        id
    }

    pub(super) fn get(&self, result_id: &str) -> Option<CachedResult> {
        let g = self.inner.lock().ok()?;
        g.entries.get(result_id).map(|r| CachedResult {
            tool_name: r.tool_name.clone(),
            content: r.content.clone(),
        })
    }
}

// ── Placeholder renderer ───────────────────────────────────────────────────

/// Build the placeholder text that replaces an oversized tool result in
/// the sub-agent's history. Shows the payload size (estimated tokens and
/// raw bytes), a preview, and a call shape for the `extract_from_result`
/// tool. The sub-agent decides whether to answer from the preview or
/// dispatch the extractor.
///
/// Token count is estimated at ~4 chars/token (same heuristic as the
/// trigger threshold in [`HANDOFF_OVERSIZE_THRESHOLD_TOKENS`]), so the
/// unit the sub-agent sees matches the unit the runtime used to decide
/// to hand off in the first place.
pub(super) fn build_handoff_placeholder(tool_name: &str, result_id: &str, raw: &str) -> String {
    let preview: String = raw.chars().take(HANDOFF_PREVIEW_CHARS).collect();
    let raw_tokens = raw.len().div_ceil(4);
    format!(
        "[oversized tool output: {raw_tokens} tokens ({raw_bytes} bytes) — stashed as result_id=\"{result_id}\"]\n\
         Preview (first {preview_chars} chars):\n{preview}\n\n\
         If the preview does not answer your task, call:\n\
         extract_from_result(result_id=\"{result_id}\", query=\"<specific question>\")\n\
         Good queries name the exact fields/identifiers you need \
         (e.g. \"subject and sender of the 5 most recent messages\"). \
         Tool: {tool_name}",
        raw_bytes = raw.len(),
        preview_chars = preview.chars().count(),
    )
}

// ── Content hygiene helpers (used by the extract path) ─────────────────────

use once_cell::sync::Lazy;
use regex::Regex;

/// Strip common noise from tool outputs before they're stashed or chunked.
///
/// Agent tools frequently return raw HTML email bodies, inline SVG, base64
/// data URIs, CSS/JS blocks, and collapsed whitespace — all of which bloat
/// the handoff cache and waste summarizer context on tokens that carry
/// zero semantic value for most extraction queries. Cleaning before the
/// oversize check means (a) some payloads drop below threshold entirely
/// and skip the extract pipeline, (b) chunked payloads fit more real
/// content per chunk, and (c) summarizers see clean text instead of
/// parsing around markup.
pub(super) fn clean_tool_output(content: &str) -> String {
    static SCRIPT_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?is)<script\b[^>]*>.*?</script\s*>").unwrap());
    static STYLE_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?is)<style\b[^>]*>.*?</style\s*>").unwrap());
    static SVG_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?is)<svg\b[^>]*>.*?</svg\s*>").unwrap());
    static HTML_COMMENT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
    static DATA_URI_RE: Lazy<Regex> =
        Lazy::new(|| Regex::new(r"(?i)data:[a-z0-9.+\-/]+;base64,[A-Za-z0-9+/=]+").unwrap());
    static HTML_TAG_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"<[^>]+>").unwrap());
    static WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[ \t\f\v]+").unwrap());
    static BLANK_LINE_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\n{3,}").unwrap());

    let cleaned = SCRIPT_RE.replace_all(content, "");
    let cleaned = STYLE_RE.replace_all(&cleaned, "");
    let cleaned = SVG_RE.replace_all(&cleaned, "[svg]");
    let cleaned = HTML_COMMENT_RE.replace_all(&cleaned, "");
    let cleaned = DATA_URI_RE.replace_all(&cleaned, "[data-uri]");
    let cleaned = HTML_TAG_RE.replace_all(&cleaned, "");
    let cleaned = WS_RE.replace_all(&cleaned, " ");
    let cleaned = BLANK_LINE_RE.replace_all(&cleaned, "\n\n");
    cleaned.trim().to_string()
}

/// Split `content` into chunks no larger than `budget` bytes, breaking
/// at natural boundaries (blank lines, then single newlines) so the
/// extraction LLM rarely sees a structure torn mid-record. Falls back to
/// char-safe slicing for pathological single-line inputs.
pub(super) fn chunk_content(content: &str, budget: usize) -> Vec<String> {
    if content.len() <= budget {
        return vec![content.to_string()];
    }

    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::with_capacity(budget.min(content.len()));

    let flush = |current: &mut String, chunks: &mut Vec<String>| {
        if !current.is_empty() {
            chunks.push(std::mem::take(current));
        }
    };

    for line in content.lines() {
        let projected = current.len() + line.len() + 1;
        if projected > budget && !current.is_empty() {
            flush(&mut current, &mut chunks);
        }
        if line.len() > budget {
            // Single line exceeds budget (e.g. JSON with no formatting).
            // Emit any pending content, then slice the line at char
            // boundaries so we don't panic on multi-byte chars.
            flush(&mut current, &mut chunks);
            let mut remaining = line;
            while !remaining.is_empty() {
                let mut cut = budget.min(remaining.len());
                while cut > 0 && !remaining.is_char_boundary(cut) {
                    cut -= 1;
                }
                if cut == 0 {
                    // Degenerate case — shouldn't happen for normal
                    // text. Take the entire remaining line to avoid
                    // an infinite loop.
                    chunks.push(remaining.to_string());
                    break;
                }
                chunks.push(remaining[..cut].to_string());
                remaining = &remaining[cut..];
            }
        } else {
            current.push_str(line);
            current.push('\n');
        }
    }
    flush(&mut current, &mut chunks);

    chunks
}
