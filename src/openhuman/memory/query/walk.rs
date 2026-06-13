//! Agentic memory-tree walk tool.
//!
//! Given a free-text query, a lightweight LLM navigates the summary tree in
//! a turn-based inner loop — calling `descend`, `peek`, `fetch_leaves`, or
//! `answer` each turn — and returns a synthesised answer with a trace.
//!
//! The inner loop uses `Provider::chat_with_history` (prompt-guided tool
//! calling via XML tags) because the `Provider::chat()` default does not
//! surface native `tool_calls` for prompt-guided backends. The response text
//! is parsed for `<tool_call>…</tool_call>` blocks, matching the harness
//! convention established in `agent/harness/parse.rs`.
//!
//! For the `Tool::execute` path, a thin `ChatProviderAdapter` wraps the
//! memory-tree's `ChatProvider` (available from `build_chat_provider`) to
//! satisfy the `Provider` trait — avoiding a dependency on the full routing
//! stack which requires a configured remote backend.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::traits::{ChatMessage, Provider};
use crate::openhuman::memory::chat::{build_chat_provider, ChatPrompt};
use crate::openhuman::memory_tree::retrieval;
use crate::openhuman::memory_tree::retrieval::fetch::fetch_leaves as do_fetch_leaves;
use crate::openhuman::memory_tree::tree_runtime::store::{read_children, read_node};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
use async_trait::async_trait;
use serde_json::json;

// ── Temperature (matches SUMMARIZATION_TEMP convention) ────────────────────
const WALK_TEMP: f64 = 0.3;
/// Hard cap on LLM turns, even if the caller requests more.
const HARD_MAX_TURNS: usize = 20;

// ── Public output types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct WalkOptions {
    /// Maximum number of LLM turns before giving up.  Default: 6.
    pub max_turns: usize,
    /// Node id to start from.  `None` → namespace root.
    pub start_node_id: Option<String>,
    /// Memory namespace.  Default: `"default"`.
    pub namespace: String,
    /// Model override.  `None` → `config.local_ai.chat_model_id`.
    pub model: Option<String>,
}

impl Default for WalkOptions {
    fn default() -> Self {
        Self {
            max_turns: 6,
            start_node_id: None,
            namespace: "default".into(),
            model: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WalkStopReason {
    /// LLM called `answer { text }`.
    Answered,
    /// Loop exhausted `max_turns` without an answer action.
    MaxTurnsReached,
    /// LLM returned no tool call and no meaningful text — treated as giving up.
    LlmGaveUp,
    /// A hard error prevented the walk from completing.
    Error(String),
}

#[derive(Debug, Clone)]
pub struct WalkStep {
    pub turn: usize,
    pub action: String,
    pub args_summary: String,
    pub result_preview: String,
}

#[derive(Debug, Clone)]
pub struct WalkOutcome {
    pub answer: String,
    pub trace: Vec<WalkStep>,
    pub turns_used: usize,
    pub stopped_reason: WalkStopReason,
}

// ── Public API ──────────────────────────────────────────────────────────────

pub struct MemoryTreeWalkTool;

#[async_trait]
impl Tool for MemoryTreeWalkTool {
    fn name(&self) -> &str {
        "memory_tree_walk"
    }

    fn description(&self) -> &str {
        "Agentically walk the memory tree to answer a query — a lightweight \
         LLM navigates summaries, drills into relevant branches, and returns \
         a synthesized answer with citations."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language question to answer by walking the memory tree."
                },
                "namespace": {
                    "type": "string",
                    "description": "Memory namespace. Default: \"default\"."
                },
                "start_node_id": {
                    "type": "string",
                    "description": "Optional starting node id. Default: namespace root."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Max LLM turns. Default 6, hard cap 20."
                }
            },
            "required": ["query"]
        })
    }

    fn category(&self) -> ToolCategory {
        ToolCategory::System
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let query = args
            .get("query")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("memory_tree_walk: `query` is required"))?
            .to_string();

        let namespace = args
            .get("namespace")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let start_node_id = args
            .get("start_node_id")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let max_turns = args
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(HARD_MAX_TURNS))
            .unwrap_or(6);

        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_tree_walk: load config failed: {e}"))?;

        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let opts = WalkOptions {
            max_turns,
            start_node_id,
            namespace,
            model,
        };

        // Build a chat provider from config (same path used by the summariser)
        // and wrap it in the thin `ChatProviderAdapter` that satisfies `Provider`.
        let chat_provider = build_chat_provider(&cfg)
            .map_err(|e| anyhow::anyhow!("memory_tree_walk: build chat provider failed: {e}"))?;
        let adapter = ChatProviderAdapter {
            inner: chat_provider,
        };

        let outcome = run_walk(&cfg, &adapter, &query, opts).await?;

        // Format output as markdown with trace.
        let mut out = format!("{}\n\n## Trace\n", outcome.answer);
        for step in &outcome.trace {
            out.push_str(&format!(
                "- **Turn {}** `{}` {}: {}\n",
                step.turn, step.action, step.args_summary, step.result_preview
            ));
        }
        out.push_str(&format!(
            "\n*Stop reason: {:?}, turns used: {}*\n",
            outcome.stopped_reason, outcome.turns_used
        ));

        Ok(ToolResult::success(out))
    }
}

/// Drive the walk without going through the Tool trait.
/// Useful for tests and callers that already hold a `Config` and `Provider`.
pub async fn run_walk(
    config: &Config,
    provider: &dyn Provider,
    query: &str,
    opts: WalkOptions,
) -> anyhow::Result<WalkOutcome> {
    let max_turns = opts.max_turns.min(HARD_MAX_TURNS);
    let model = opts
        .model
        .clone()
        .unwrap_or_else(|| config.local_ai.chat_model_id.clone());

    log::debug!(
        "[memory_tree_walk] starting walk query_len={} namespace={} max_turns={} model={}",
        query.len(),
        opts.namespace,
        max_turns,
        model
    );

    // Determine the starting node.
    let start_id = opts
        .start_node_id
        .clone()
        .unwrap_or_else(|| "root".to_string());

    // Load the starting node summary + children to build the first context message.
    let initial_context = build_node_context(config, &opts.namespace, &start_id).await;
    log::debug!(
        "[memory_tree_walk] initial_context node_id={} context_len={}",
        start_id,
        initial_context.len()
    );

    let system = build_system_prompt();
    let inner_tools_text = build_inner_tools_text();

    // Chat history: system → tool instructions injected → user query → context.
    let mut history: Vec<ChatMessage> = vec![
        ChatMessage::system(format!("{system}\n\n{inner_tools_text}")),
        ChatMessage::user(format!(
            "Query: {query}\n\nCurrent position in memory tree:\n{initial_context}"
        )),
    ];

    let mut trace: Vec<WalkStep> = Vec::new();
    let mut current_node_id = start_id.clone();

    for turn in 1..=max_turns {
        log::debug!("[memory_tree_walk] turn={turn} current_node={current_node_id}");

        let response = match provider
            .chat_with_history(&history, &model, WALK_TEMP)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[memory_tree_walk] provider error on turn={turn}: {e:#}");
                let err_msg = format!("Provider error on turn {turn}: {e}");
                return Ok(WalkOutcome {
                    answer: format!(
                        "Walk failed: {err_msg}\n\nPartial trace from {} turn(s).",
                        trace.len()
                    ),
                    trace,
                    turns_used: turn,
                    stopped_reason: WalkStopReason::Error(err_msg),
                });
            }
        };

        log::debug!(
            "[memory_tree_walk] turn={turn} response_len={}",
            response.len()
        );

        // Parse tool calls from the response text.
        let (text_before, calls) = parse_walk_tool_calls(&response);

        if calls.is_empty() {
            // No tool call — treat as final answer if there's meaningful text.
            let trimmed = response.trim().to_string();
            if trimmed.is_empty() {
                log::debug!("[memory_tree_walk] turn={turn} LLM gave up (empty response)");
                return Ok(WalkOutcome {
                    answer: synthesize_fallback_answer(&trace),
                    trace,
                    turns_used: turn,
                    stopped_reason: WalkStopReason::LlmGaveUp,
                });
            }
            log::debug!("[memory_tree_walk] turn={turn} no tool calls — treating as final answer");
            return Ok(WalkOutcome {
                answer: trimmed,
                trace,
                turns_used: turn,
                stopped_reason: WalkStopReason::Answered,
            });
        }

        // Process the first tool call (walk is serial).
        let call = &calls[0];
        log::debug!(
            "[memory_tree_walk] turn={turn} action={} args={}",
            call.name,
            call.args
        );

        // Append assistant turn to history.
        history.push(ChatMessage::assistant(response.clone()));

        // Dispatch inner walk primitive.
        let (step_args_summary, tool_result, is_answer, answer_text) =
            dispatch_inner_call(config, &opts.namespace, call, &mut current_node_id).await;

        let result_preview: String = tool_result.chars().take(200).collect();
        trace.push(WalkStep {
            turn,
            action: call.name.clone(),
            args_summary: step_args_summary,
            result_preview: result_preview.clone(),
        });

        if is_answer {
            log::debug!("[memory_tree_walk] turn={turn} answer action — stopping");
            return Ok(WalkOutcome {
                answer: answer_text,
                trace,
                turns_used: turn,
                stopped_reason: WalkStopReason::Answered,
            });
        }

        // Append tool result as user message (prompt-guided protocol).
        let tool_msg = format!(
            "<tool_result>{}</tool_result>\n\nCurrent position: {current_node_id}\n",
            tool_result
        );
        history.push(ChatMessage::user(tool_msg));

        // Drop the preamble text if there was any (don't lose context).
        if !text_before.trim().is_empty() {
            log::debug!(
                "[memory_tree_walk] turn={turn} text before tool call: {}",
                &text_before[..text_before.len().min(80)]
            );
        }
    }

    // Max turns reached.
    log::debug!("[memory_tree_walk] max_turns={max_turns} reached — synthesising fallback");
    Ok(WalkOutcome {
        answer: synthesize_fallback_answer(&trace),
        trace,
        turns_used: max_turns,
        stopped_reason: WalkStopReason::MaxTurnsReached,
    })
}

// ── ChatProviderAdapter ─────────────────────────────────────────────────────
//
// Bridges the memory-tree's lightweight `ChatProvider` into the top-level
// `Provider` trait so `run_walk` can accept both production adapters and
// unit-test stubs that implement `Provider` directly.

struct ChatProviderAdapter {
    inner: std::sync::Arc<dyn crate::openhuman::memory::chat::ChatProvider>,
}

#[async_trait]
impl Provider for ChatProviderAdapter {
    async fn chat_with_system(
        &self,
        system: Option<&str>,
        message: &str,
        _model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let prompt = ChatPrompt {
            system: system.unwrap_or("").to_string(),
            user: message.to_string(),
            temperature,
            kind: "memory_tree_walk",
        };
        self.inner.chat_for_text(&prompt).await
    }

    async fn chat_with_history(
        &self,
        messages: &[ChatMessage],
        model: &str,
        temperature: f64,
    ) -> anyhow::Result<String> {
        let system = messages
            .iter()
            .find(|m| m.role == "system")
            .map(|m| m.content.as_str());
        // Combine all non-system messages into the user turn.
        let user: String = messages
            .iter()
            .filter(|m| m.role != "system")
            .map(|m| m.content.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        self.chat_with_system(system, &user, model, temperature)
            .await
    }
}

// ── Inner helpers ───────────────────────────────────────────────────────────

/// A parsed tool call from the inner walk loop.
struct InnerCall {
    name: String,
    args: serde_json::Value,
}

/// Parse `<tool_call>…</tool_call>` blocks from a response string.
/// Returns `(text_before_first_call, calls)`.
fn parse_walk_tool_calls(response: &str) -> (String, Vec<InnerCall>) {
    let mut calls: Vec<InnerCall> = Vec::new();
    let mut text_parts: Vec<&str> = Vec::new();
    let mut remaining: &str = response;

    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";

    loop {
        match remaining.find(OPEN) {
            None => {
                // No more tags; collect trailing text.
                if !remaining.trim().is_empty() && calls.is_empty() {
                    text_parts.push(remaining);
                }
                break;
            }
            Some(start) => {
                let before = &remaining[..start];
                if !before.trim().is_empty() {
                    text_parts.push(before);
                }
                let after_open = &remaining[start + OPEN.len()..];
                match after_open.find(CLOSE) {
                    None => break, // malformed — stop
                    Some(close_idx) => {
                        let inner = &after_open[..close_idx];
                        if let Ok(val) = serde_json::from_str::<serde_json::Value>(inner.trim()) {
                            if let Some(name) = val.get("name").and_then(|v| v.as_str()) {
                                let args = val
                                    .get("arguments")
                                    .cloned()
                                    .unwrap_or(serde_json::Value::Object(Default::default()));
                                calls.push(InnerCall {
                                    name: name.to_string(),
                                    args,
                                });
                            }
                        }
                        remaining = &after_open[close_idx + CLOSE.len()..];
                    }
                }
            }
        }
    }

    let text_before = text_parts.concat();
    (text_before, calls)
}

/// Dispatch an inner walk primitive and return
/// `(args_summary, result_text, is_final_answer, answer_text)`.
async fn dispatch_inner_call(
    config: &Config,
    namespace: &str,
    call: &InnerCall,
    current_node_id: &mut String,
) -> (String, String, bool, String) {
    match call.name.as_str() {
        "descend" => {
            let node_id = call
                .args
                .get("node_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            log::debug!(
                "[memory_tree_walk] descend node_id={node_id} from={current_node_id} namespace={namespace}"
            );

            if node_id.is_empty() {
                return (
                    "node_id=<empty>".into(),
                    "error: descend requires a non-empty node_id".into(),
                    false,
                    String::new(),
                );
            }

            // Move to the target node.
            let ctx = build_node_context(config, namespace, &node_id).await;
            if ctx.starts_with("unknown node") {
                (
                    format!("node_id={node_id}"),
                    format!("unknown node: {node_id}"),
                    false,
                    String::new(),
                )
            } else {
                *current_node_id = node_id.clone();
                (format!("node_id={node_id}"), ctx, false, String::new())
            }
        }

        "peek" => {
            let node_ids: Vec<String> = call
                .args
                .get("node_ids")
                .and_then(|v| v.as_array())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_str())
                        .map(|s| s.to_string())
                        .collect()
                })
                .unwrap_or_default();

            log::debug!(
                "[memory_tree_walk] peek node_ids={} namespace={namespace}",
                node_ids.len()
            );

            let args_summary = format!("node_ids=[{}]", node_ids.join(", "));

            let config_owned = config.clone();
            let ns_owned = namespace.to_string();
            let ids_owned = node_ids.clone();

            let result = tokio::task::spawn_blocking(move || -> Vec<String> {
                ids_owned
                    .iter()
                    .map(|id| match read_node(&config_owned, &ns_owned, id) {
                        Ok(Some(node)) => {
                            format!(
                                "id={} level={:?} summary={}",
                                id,
                                node.level,
                                &node.summary[..node.summary.len().min(120)]
                            )
                        }
                        Ok(None) => format!("id={id} unknown node"),
                        Err(e) => format!("id={id} error: {e}"),
                    })
                    .collect()
            })
            .await
            .unwrap_or_default();

            (args_summary, result.join("\n"), false, String::new())
        }

        "fetch_leaves" => {
            let node_id = call
                .args
                .get("node_id")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();

            log::debug!("[memory_tree_walk] fetch_leaves node_id={node_id} namespace={namespace}");

            if node_id.is_empty() {
                return (
                    "node_id=<empty>".into(),
                    "error: fetch_leaves requires a non-empty node_id".into(),
                    false,
                    String::new(),
                );
            }

            // fetch_leaves in retrieval takes a list of chunk ids. Reuse
            // drill_down to get the leaf hits under this node, then return content.
            let hits = match retrieval::drill_down(config, &node_id, 1, None, Some(10)).await {
                Ok(h) => h,
                Err(e) => {
                    return (
                        format!("node_id={node_id}"),
                        format!("error fetching leaves: {e}"),
                        false,
                        String::new(),
                    );
                }
            };

            let text = if hits.is_empty() {
                // Try to fetch the node itself as a leaf (chunk).
                let chunk_ids = vec![node_id.clone()];
                match do_fetch_leaves(config, &chunk_ids).await {
                    Ok(leaf_hits) if !leaf_hits.is_empty() => leaf_hits
                        .iter()
                        .map(|h| format!("[{}] {}", h.node_id, h.content))
                        .collect::<Vec<_>>()
                        .join("\n---\n"),
                    _ => format!("no leaves found under node_id={node_id}"),
                }
            } else {
                hits.iter()
                    .map(|h| format!("[{}] {}", h.node_id, h.content))
                    .collect::<Vec<_>>()
                    .join("\n---\n")
            };

            (format!("node_id={node_id}"), text, false, String::new())
        }

        "answer" => {
            let text = call
                .args
                .get("text")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            log::debug!("[memory_tree_walk] answer action text_len={}", text.len());
            ("(final answer)".into(), text.clone(), true, text)
        }

        other => {
            log::warn!("[memory_tree_walk] unknown inner action: {other}");
            (
                format!("action={other}"),
                format!(
                    "unknown walk action '{other}'. Valid: descend, peek, fetch_leaves, answer"
                ),
                false,
                String::new(),
            )
        }
    }
}

/// Build a human-readable context string for a node: its summary + children list.
async fn build_node_context(config: &Config, namespace: &str, node_id: &str) -> String {
    let config_owned = config.clone();
    let ns_owned = namespace.to_string();
    let id_owned = node_id.to_string();

    tokio::task::spawn_blocking(move || {
        let node = match read_node(&config_owned, &ns_owned, &id_owned) {
            Ok(Some(n)) => n,
            Ok(None) => return format!("unknown node: {id_owned}"),
            Err(e) => return format!("error reading node {id_owned}: {e}"),
        };

        let children = match read_children(&config_owned, &ns_owned, &id_owned) {
            Ok(c) => c,
            Err(_) => vec![],
        };

        let mut out = format!(
            "Node: {} (level={:?})\nSummary: {}\n",
            node.node_id, node.level, node.summary
        );

        if children.is_empty() {
            out.push_str("Children: (none — this is a leaf)\n");
        } else {
            out.push_str(&format!("Children ({}):\n", children.len()));
            for c in &children {
                out.push_str(&format!(
                    "  - id={} level={:?} summary_preview={}\n",
                    c.node_id,
                    c.level,
                    &c.summary[..c.summary.len().min(80)]
                ));
            }
        }

        out
    })
    .await
    .unwrap_or_else(|_| format!("error building context for node {node_id}"))
}

fn build_system_prompt() -> String {
    "You are a memory-tree navigator. Your job is to answer user queries \
     by walking a hierarchical summary tree.\n\
     Use the provided tools to navigate: `descend` to move into a child node, \
     `peek` to preview multiple children without descending, \
     `fetch_leaves` to retrieve raw content from a node, \
     and `answer` when you have enough information to respond.\n\
     Be efficient — prefer `peek` to survey options before `descend`.\n\
     Always end with `answer { \"text\": \"...\" }` when ready.\n\
     Use XML tool_call tags:\n\
     <tool_call>{\"name\": \"descend\", \"arguments\": {\"node_id\": \"some/id\"}}</tool_call>"
        .into()
}

fn build_inner_tools_text() -> String {
    "## Inner walk tools\n\n\
     **descend** `{\"node_id\": \"<id>\"}` — move to a child node and see its summary and children.\n\
     **peek** `{\"node_ids\": [\"<id1>\", \"<id2>\"]}` — preview summaries for a list of nodes without descending.\n\
     **fetch_leaves** `{\"node_id\": \"<id>\"}` — retrieve raw chunk text under a node for citation.\n\
     **answer** `{\"text\": \"<final answer>\"}` — stop and return your synthesised answer."
        .into()
}

fn synthesize_fallback_answer(trace: &[WalkStep]) -> String {
    if trace.is_empty() {
        return "Could not converge on an answer — no steps taken.".into();
    }
    let preview: Vec<String> = trace
        .iter()
        .map(|s| {
            format!(
                "Turn {}: {} → {}",
                s.turn,
                s.action,
                &s.result_preview[..s.result_preview.len().min(100)]
            )
        })
        .collect();
    format!(
        "Could not converge on an answer within the turn limit. Here is what I saw:\n\n{}",
        preview.join("\n")
    )
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;
    use crate::openhuman::inference::provider::traits::ChatMessage;
    use crate::openhuman::memory_tree::tree_runtime::store::write_node;
    use crate::openhuman::memory_tree::tree_runtime::types::{NodeLevel, TreeNode};
    use async_trait::async_trait;
    use chrono::Utc;
    use std::sync::Mutex;
    use tempfile::TempDir;

    // ── Stub provider ──────────────────────────────────────────────────

    /// A scripted stub provider that returns predefined responses in sequence.
    /// Each `chat_with_history` call pops the next response from the queue.
    struct StubProvider {
        responses: Mutex<Vec<String>>,
    }

    impl StubProvider {
        fn new(responses: Vec<&str>) -> Self {
            Self {
                responses: Mutex::new(responses.into_iter().map(|s| s.to_string()).collect()),
            }
        }
    }

    #[async_trait]
    impl Provider for StubProvider {
        async fn chat_with_system(
            &self,
            _system: Option<&str>,
            _message: &str,
            _model: &str,
            _temp: f64,
        ) -> anyhow::Result<String> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(anyhow::anyhow!("StubProvider: no more scripted responses"));
            }
            Ok(responses.remove(0))
        }

        async fn chat_with_history(
            &self,
            _messages: &[ChatMessage],
            _model: &str,
            _temp: f64,
        ) -> anyhow::Result<String> {
            let mut responses = self.responses.lock().unwrap();
            if responses.is_empty() {
                return Err(anyhow::anyhow!("StubProvider: no more scripted responses"));
            }
            Ok(responses.remove(0))
        }
    }

    // ── Tree helpers ───────────────────────────────────────────────────

    fn test_config(tmp: &TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        cfg
    }

    fn make_node(namespace: &str, node_id: &str, summary: &str, child_count: u32) -> TreeNode {
        let level = crate::openhuman::memory_tree::tree_runtime::types::level_from_node_id(node_id);
        let parent_id =
            crate::openhuman::memory_tree::tree_runtime::types::derive_parent_id(node_id);
        let ts = Utc::now();
        TreeNode {
            node_id: node_id.to_string(),
            namespace: namespace.to_string(),
            level,
            parent_id,
            summary: summary.to_string(),
            token_count: crate::openhuman::memory_tree::tree_runtime::types::estimate_tokens(
                summary,
            ),
            child_count,
            created_at: ts,
            updated_at: ts,
            metadata: None,
        }
    }

    /// Seed: root → 2024 (child of root) → 2024/01 (leaf).
    fn seed_tree(cfg: &Config, ns: &str) {
        write_node(
            cfg,
            &make_node(ns, "root", "All-time summary: project logs 2024", 1),
        )
        .unwrap();
        write_node(
            cfg,
            &make_node(ns, "2024", "Year 2024: shipped v1, v2, v3", 1),
        )
        .unwrap();
        write_node(
            cfg,
            &make_node(ns, "2024/01", "January 2024: initial project launch", 0),
        )
        .unwrap();
    }

    // ── Test 1: walks_and_answers ──────────────────────────────────────

    /// Script: turn1=descend(2024), turn2=fetch_leaves(2024/01), turn3=answer.
    #[tokio::test]
    async fn walks_and_answers() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let ns = "default";
        seed_tree(&cfg, ns);

        let provider = StubProvider::new(vec![
            // Turn 1: descend into the year node.
            r#"<tool_call>{"name":"descend","arguments":{"node_id":"2024"}}</tool_call>"#,
            // Turn 2: fetch leaves under the month node.
            r#"<tool_call>{"name":"fetch_leaves","arguments":{"node_id":"2024/01"}}</tool_call>"#,
            // Turn 3: answer.
            r#"<tool_call>{"name":"answer","arguments":{"text":"The project launched in January 2024."}}</tool_call>"#,
        ]);

        let opts = WalkOptions {
            max_turns: 6,
            start_node_id: None,
            namespace: ns.to_string(),
            model: Some("test-model".into()),
        };

        let outcome = run_walk(&cfg, &provider, "When did the project launch?", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, WalkStopReason::Answered);
        assert!(
            outcome.answer.contains("January 2024"),
            "answer should mention January 2024, got: {}",
            outcome.answer
        );
        assert_eq!(outcome.trace.len(), 3, "expected 3 steps");
        assert_eq!(outcome.trace[0].action, "descend");
        assert_eq!(outcome.trace[1].action, "fetch_leaves");
        assert_eq!(outcome.trace[2].action, "answer");
        assert_eq!(outcome.turns_used, 3);
    }

    // ── Test 2: max_turns_cap ─────────────────────────────────────────

    /// Script: always `descend` in a loop — should stop at max_turns.
    #[tokio::test]
    async fn max_turns_cap() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let ns = "default";
        seed_tree(&cfg, ns);

        // Provide more `descend` responses than max_turns (3) so the cap fires.
        let provider = StubProvider::new(vec![
            r#"<tool_call>{"name":"descend","arguments":{"node_id":"2024"}}</tool_call>"#,
            r#"<tool_call>{"name":"descend","arguments":{"node_id":"2024"}}</tool_call>"#,
            r#"<tool_call>{"name":"descend","arguments":{"node_id":"2024"}}</tool_call>"#,
            r#"<tool_call>{"name":"descend","arguments":{"node_id":"2024"}}</tool_call>"#,
        ]);

        let opts = WalkOptions {
            max_turns: 3,
            start_node_id: None,
            namespace: ns.to_string(),
            model: Some("test-model".into()),
        };

        let outcome = run_walk(&cfg, &provider, "infinite loop query", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, WalkStopReason::MaxTurnsReached);
        assert_eq!(outcome.turns_used, 3);
        assert!(
            outcome.answer.contains("Could not converge"),
            "expected fallback answer, got: {}",
            outcome.answer
        );
    }

    // ── Test 3: unknown_node_recovers ─────────────────────────────────

    /// Script: descend into a non-existent node → result says "unknown node" → loop continues.
    #[tokio::test]
    async fn unknown_node_recovers() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let ns = "default";
        seed_tree(&cfg, ns);

        let provider = StubProvider::new(vec![
            // Turn 1: descend into a node that does not exist.
            r#"<tool_call>{"name":"descend","arguments":{"node_id":"does_not_exist"}}</tool_call>"#,
            // Turn 2: answer (loop continues after bad descend).
            r#"<tool_call>{"name":"answer","arguments":{"text":"I could not find that node."}}</tool_call>"#,
        ]);

        let opts = WalkOptions {
            max_turns: 6,
            start_node_id: None,
            namespace: ns.to_string(),
            model: Some("test-model".into()),
        };

        let outcome = run_walk(&cfg, &provider, "find nonexistent data", opts)
            .await
            .unwrap();

        // The first trace step should indicate "unknown node".
        assert_eq!(outcome.trace.len(), 2);
        assert!(
            outcome.trace[0].result_preview.contains("unknown node"),
            "expected 'unknown node' in preview, got: {}",
            outcome.trace[0].result_preview
        );
        // The walk should continue and eventually answer.
        assert_eq!(outcome.stopped_reason, WalkStopReason::Answered);
        assert!(outcome.answer.contains("could not find that node"));
    }
}
