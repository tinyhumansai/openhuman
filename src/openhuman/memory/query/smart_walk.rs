//! E2GraphRAG-inspired smart memory retrieval.
//!
//! Unlike the basic `walk` module which only navigates the time-based summary
//! tree, smart_walk combines multiple retrieval strategies:
//!
//! 1. **Vector search** — semantic similarity across all stored content
//! 2. **Keyword search** — pattern matching across raw content files on disk
//! 3. **Entity search** — find entities and follow relationships
//! 4. **Tree browse** — navigate wiki summary hierarchies
//! 5. **Content read** — read specific files (raw/wiki/document/episodic)
//! 6. **Source listing** — discover available sources and content types
//!
//! The walker LLM (defaulting to DeepSeek Flash) plans which strategies to
//! use, collects evidence snippets, then synthesizes a cited answer.

use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::config::Config;
use crate::openhuman::inference::provider::traits::{ChatMessage, Provider};
use crate::openhuman::memory::chat::{build_chat_provider, ChatPrompt};
use crate::openhuman::memory_store::chunks::types::SourceKind;
use crate::openhuman::memory_tree::retrieval;
use crate::openhuman::memory_tree::score::extract::EntityKind;
use crate::openhuman::memory_tree::tree_runtime::store::{read_children, read_node};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolCategory, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::path::{Path, PathBuf};

const SMART_WALK_TEMP: f64 = 0.2;
const HARD_MAX_TURNS: usize = 25;
const MAX_EVIDENCE_ITEMS: usize = 30;
const MAX_KEYWORD_RESULTS: usize = 15;
const MAX_FILE_READ_BYTES: usize = 8000;

fn truncate_chars(value: &str, max_chars: usize) -> String {
    value.chars().take(max_chars).collect()
}

// ── Public output types ─────────────────────────────────────────────────────

#[derive(Debug, Clone)]
pub struct SmartWalkOptions {
    pub max_turns: usize,
    pub namespace: String,
    /// Provider string override (e.g. "deepseek:deepseek-chat").
    pub model: Option<String>,
    /// Content root override. Defaults to config.memory_tree_content_root().
    pub content_root: Option<PathBuf>,
}

impl Default for SmartWalkOptions {
    fn default() -> Self {
        Self {
            max_turns: 12,
            namespace: "default".into(),
            model: None,
            content_root: None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SmartWalkStopReason {
    Answered,
    MaxTurnsReached,
    LlmGaveUp,
    Error(String),
}

#[derive(Debug, Clone)]
pub struct SmartWalkStep {
    pub turn: usize,
    pub action: String,
    pub args_summary: String,
    pub result_preview: String,
}

#[derive(Debug, Clone)]
pub struct Evidence {
    pub source_path: String,
    pub snippet: String,
    pub relevance: String,
}

#[derive(Debug, Clone)]
pub struct SmartWalkOutcome {
    pub answer: String,
    pub evidence: Vec<Evidence>,
    pub trace: Vec<SmartWalkStep>,
    pub turns_used: usize,
    pub stopped_reason: SmartWalkStopReason,
}

// ── Tool ────────────────────────────────────────────────────────────────────

pub struct SmartMemoryWalkTool;

#[async_trait]
impl Tool for SmartMemoryWalkTool {
    fn name(&self) -> &str {
        "memory_smart_walk"
    }

    fn description(&self) -> &str {
        "Smart memory retrieval — combines vector search, keyword search, \
         entity lookup, and tree browsing to answer queries about the user's \
         memory. More capable than the basic walk: searches across raw files, \
         wiki summaries, documents, and episodic memories."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Natural-language question to answer by searching memory."
                },
                "namespace": {
                    "type": "string",
                    "description": "Memory namespace. Default: \"default\"."
                },
                "max_turns": {
                    "type": "integer",
                    "description": "Max LLM turns. Default 12, hard cap 25."
                },
                "model": {
                    "type": "string",
                    "description": "Provider:model override (e.g. 'deepseek:deepseek-chat')."
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
            .ok_or_else(|| anyhow::anyhow!("memory_smart_walk: `query` is required"))?
            .to_string();

        let namespace = args
            .get("namespace")
            .and_then(|v| v.as_str())
            .unwrap_or("default")
            .to_string();

        let max_turns = args
            .get("max_turns")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).min(HARD_MAX_TURNS))
            .unwrap_or(12);

        let model = args
            .get("model")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let cfg = config_rpc::load_config_with_timeout()
            .await
            .map_err(|e| anyhow::anyhow!("memory_smart_walk: load config failed: {e}"))?;

        let opts = SmartWalkOptions {
            max_turns,
            namespace,
            model,
            content_root: None,
        };

        let chat_provider = build_chat_provider(&cfg)
            .map_err(|e| anyhow::anyhow!("memory_smart_walk: build chat provider failed: {e}"))?;
        let adapter = ChatProviderAdapter {
            inner: chat_provider,
        };

        let outcome = run_smart_walk(&cfg, &adapter, &query, opts).await?;

        let mut out = format!("{}\n", outcome.answer);

        if !outcome.evidence.is_empty() {
            out.push_str("\n## Evidence\n");
            for (i, ev) in outcome.evidence.iter().enumerate() {
                out.push_str(&format!(
                    "{}. **{}** — {}\n   > {}\n",
                    i + 1,
                    ev.source_path,
                    ev.relevance,
                    truncate_chars(&ev.snippet, 200)
                ));
            }
        }

        out.push_str("\n## Trace\n");
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

// ── Main loop ───────────────────────────────────────────────────────────────

pub async fn run_smart_walk(
    config: &Config,
    provider: &dyn Provider,
    query: &str,
    opts: SmartWalkOptions,
) -> anyhow::Result<SmartWalkOutcome> {
    let max_turns = opts.max_turns.min(HARD_MAX_TURNS);
    let model = opts
        .model
        .clone()
        .unwrap_or_else(|| resolve_walk_model(config));

    let content_root = opts
        .content_root
        .clone()
        .unwrap_or_else(|| config.memory_tree_content_root());

    log::debug!(
        "[smart_walk] starting query_len={} namespace={} max_turns={} model={} content_root={}",
        query.len(),
        opts.namespace,
        max_turns,
        model,
        content_root.display()
    );

    let system = build_system_prompt();
    let inner_tools = build_inner_tools_text();

    let cr = content_root.clone();
    let inventory = tokio::task::spawn_blocking(move || build_content_inventory(&cr))
        .await
        .unwrap_or_else(|_| "error building content inventory".into());

    let mut history: Vec<ChatMessage> = vec![
        ChatMessage::system(format!("{system}\n\n{inner_tools}")),
        ChatMessage::user(format!(
            "Query: {query}\n\n## Available content\n{inventory}"
        )),
    ];

    let mut trace: Vec<SmartWalkStep> = Vec::new();
    let mut evidence: Vec<Evidence> = Vec::new();

    for turn in 1..=max_turns {
        log::debug!("[smart_walk] turn={turn} evidence_count={}", evidence.len());

        let response = match provider
            .chat_with_history(&history, &model, SMART_WALK_TEMP)
            .await
        {
            Ok(r) => r,
            Err(e) => {
                log::warn!("[smart_walk] provider error on turn={turn}: {e:#}");
                let err_msg = format!("Provider error on turn {turn}: {e}");
                return Ok(SmartWalkOutcome {
                    answer: format!(
                        "Walk failed: {err_msg}\n\nPartial from {} turn(s).",
                        trace.len()
                    ),
                    evidence,
                    trace,
                    turns_used: turn,
                    stopped_reason: SmartWalkStopReason::Error(err_msg),
                });
            }
        };

        log::debug!("[smart_walk] turn={turn} response_len={}", response.len());

        let (text_before, calls) = parse_tool_calls(&response);

        if calls.is_empty() {
            let trimmed = response.trim().to_string();
            if trimmed.is_empty() {
                log::debug!("[smart_walk] turn={turn} LLM gave up (empty response)");
                return Ok(SmartWalkOutcome {
                    answer: synthesize_fallback(&trace, &evidence),
                    evidence,
                    trace,
                    turns_used: turn,
                    stopped_reason: SmartWalkStopReason::LlmGaveUp,
                });
            }
            log::debug!("[smart_walk] turn={turn} no tool calls — treating as answer");
            return Ok(SmartWalkOutcome {
                answer: trimmed,
                evidence,
                trace,
                turns_used: turn,
                stopped_reason: SmartWalkStopReason::Answered,
            });
        }

        history.push(ChatMessage::assistant(response.clone()));

        // Process ALL tool calls in this turn (not just the first).
        let mut combined_results = Vec::new();
        for call in &calls {
            log::debug!(
                "[smart_walk] turn={turn} action={} args={}",
                call.name,
                call.args
            );

            let (args_summary, tool_result, is_answer, answer_text) =
                dispatch_call(config, &opts.namespace, &content_root, call, &mut evidence).await;

            let result_preview: String = tool_result.chars().take(200).collect();
            trace.push(SmartWalkStep {
                turn,
                action: call.name.clone(),
                args_summary,
                result_preview: result_preview.clone(),
            });

            if is_answer {
                log::debug!("[smart_walk] turn={turn} answer action — stopping");
                return Ok(SmartWalkOutcome {
                    answer: answer_text,
                    evidence,
                    trace,
                    turns_used: turn,
                    stopped_reason: SmartWalkStopReason::Answered,
                });
            }

            combined_results.push(format!(
                "<tool_result name=\"{}\">{}</tool_result>",
                call.name, tool_result
            ));
        }

        let evidence_summary = if evidence.is_empty() {
            String::new()
        } else {
            format!(
                "\n\nEvidence collected so far ({} items):\n{}",
                evidence.len(),
                evidence
                    .iter()
                    .enumerate()
                    .map(|(i, e)| format!("  {}. [{}] {}", i + 1, e.source_path, e.relevance))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        };

        let result_msg = format!("{}{}", combined_results.join("\n"), evidence_summary);
        history.push(ChatMessage::user(result_msg));

        if !text_before.trim().is_empty() {
            log::debug!(
                "[smart_walk] turn={turn} text before tool calls: {}",
                truncate_chars(&text_before, 80)
            );
        }
    }

    log::debug!("[smart_walk] max_turns={max_turns} reached");
    Ok(SmartWalkOutcome {
        answer: synthesize_fallback(&trace, &evidence),
        evidence,
        trace,
        turns_used: max_turns,
        stopped_reason: SmartWalkStopReason::MaxTurnsReached,
    })
}

// ── ChatProviderAdapter ─────────────────────────────────────────────────────

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
            kind: "memory_smart_walk",
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

// ── Inner call types ────────────────────────────────────────────────────────

#[derive(Clone)]
struct InnerCall {
    name: String,
    args: serde_json::Value,
}

// ── Dispatch ────────────────────────────────────────────────────────────────

async fn dispatch_call(
    config: &Config,
    namespace: &str,
    content_root: &Path,
    call: &InnerCall,
    evidence: &mut Vec<Evidence>,
) -> (String, String, bool, String) {
    match call.name.as_str() {
        "keyword_search" => {
            let cr = content_root.to_path_buf();
            let c = call.clone();
            tokio::task::spawn_blocking(move || dispatch_keyword_search(&cr, &c))
                .await
                .unwrap_or_else(|e| (String::new(), format!("error: {e}"), false, String::new()))
        }
        "entity_search" => dispatch_entity_search(config, call).await,
        "list_sources" => {
            let cr = content_root.to_path_buf();
            let c = call.clone();
            tokio::task::spawn_blocking(move || dispatch_list_sources(&cr, &c))
                .await
                .unwrap_or_else(|e| (String::new(), format!("error: {e}"), false, String::new()))
        }
        "read_content" => {
            let cr = content_root.to_path_buf();
            let c = call.clone();
            tokio::task::spawn_blocking(move || dispatch_read_content(&cr, &c))
                .await
                .unwrap_or_else(|e| (String::new(), format!("error: {e}"), false, String::new()))
        }
        "browse_tree" => dispatch_browse_tree(config, namespace, call).await,
        "collect_evidence" => dispatch_collect_evidence(call, evidence),
        "answer" => dispatch_answer(call),
        "vector_search" => dispatch_vector_search(config, call).await,
        other => {
            log::warn!("[smart_walk] unknown action: {other}");
            (
                format!("action={other}"),
                format!(
                    "unknown action '{other}'. Valid: keyword_search, entity_search, \
                     list_sources, read_content, browse_tree, vector_search, \
                     collect_evidence, answer"
                ),
                false,
                String::new(),
            )
        }
    }
}

// ── keyword_search ──────────────────────────────────────────────────────────

fn dispatch_keyword_search(
    content_root: &Path,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let pattern = call
        .args
        .get("pattern")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let content_type = call
        .args
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    if pattern.is_empty() {
        return (
            "pattern=<empty>".into(),
            "error: keyword_search requires a non-empty pattern".into(),
            false,
            String::new(),
        );
    }

    log::debug!(
        "[smart_walk] keyword_search pattern={} content_type={}",
        pattern,
        content_type
    );

    let args_summary = format!("pattern=\"{}\" type={}", pattern, content_type);

    let search_dirs: Vec<PathBuf> = match content_type {
        "raw" => vec![content_root.join("raw")],
        "wiki" => vec![content_root.join("wiki")],
        "document" => vec![content_root.join("document")],
        "episodic" => vec![content_root.join("episodic")],
        _ => vec![
            content_root.join("raw"),
            content_root.join("wiki"),
            content_root.join("document"),
            content_root.join("episodic"),
        ],
    };

    let pattern_lower = pattern.to_lowercase();
    let mut results: Vec<String> = Vec::new();

    for dir in &search_dirs {
        if !dir.exists() {
            continue;
        }
        search_dir_recursive(dir, &pattern_lower, &mut results, content_root);
        if results.len() >= MAX_KEYWORD_RESULTS {
            break;
        }
    }

    results.truncate(MAX_KEYWORD_RESULTS);

    if results.is_empty() {
        (
            args_summary,
            format!("no matches for pattern \"{}\"", pattern),
            false,
            String::new(),
        )
    } else {
        let count = results.len();
        (
            args_summary,
            format!("{count} matches:\n{}", results.join("\n")),
            false,
            String::new(),
        )
    }
}

fn search_dir_recursive(dir: &Path, pattern: &str, results: &mut Vec<String>, content_root: &Path) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };

    for entry in entries.flatten() {
        if results.len() >= MAX_KEYWORD_RESULTS {
            return;
        }

        let path = entry.path();
        if path.is_dir() {
            search_dir_recursive(&path, pattern, results, content_root);
        } else if path.extension().map_or(false, |e| e == "md") {
            if let Ok(content) = std::fs::read_to_string(&path) {
                if content.to_lowercase().contains(pattern) {
                    let rel = path
                        .strip_prefix(content_root)
                        .unwrap_or(&path)
                        .to_string_lossy()
                        .to_string();

                    let line_match = content
                        .lines()
                        .find(|l| l.to_lowercase().contains(pattern))
                        .unwrap_or("")
                        .trim();
                    let preview: String = line_match.chars().take(120).collect();
                    results.push(format!("  [{rel}] {preview}"));
                }
            }
        }
    }
}

// ── entity_search ───────────────────────────────────────────────────────────

async fn dispatch_entity_search(
    config: &Config,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let query = call
        .args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let kinds: Option<Vec<EntityKind>> =
        call.args
            .get("kinds")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .filter_map(|s| EntityKind::parse(s).ok())
                    .collect()
            });

    if query.is_empty() {
        return (
            "query=<empty>".into(),
            "error: entity_search requires a non-empty query".into(),
            false,
            String::new(),
        );
    }

    log::debug!(
        "[smart_walk] entity_search query={} kinds={:?}",
        query,
        kinds
            .as_ref()
            .map(|ks| ks.iter().map(|k| k.as_str()).collect::<Vec<_>>())
    );
    let args_summary = format!(
        "query=\"{}\" kinds={:?}",
        query,
        kinds
            .as_ref()
            .map(|ks| ks.iter().map(|k| k.as_str()).collect::<Vec<_>>())
    );

    match retrieval::search_entities(config, &query, kinds, 10).await {
        Ok(matches) => {
            if matches.is_empty() {
                (
                    args_summary,
                    format!("no entities matching \"{}\"", query),
                    false,
                    String::new(),
                )
            } else {
                let formatted: Vec<String> = matches
                    .iter()
                    .map(|m| {
                        format!(
                            "  [{}] kind={} surface=\"{}\" mentions={} last_seen={}",
                            m.canonical_id,
                            m.kind.as_str(),
                            m.surface,
                            m.mention_count,
                            m.last_seen_ms
                        )
                    })
                    .collect();
                (
                    args_summary,
                    format!(
                        "{} entities found:\n{}",
                        formatted.len(),
                        formatted.join("\n")
                    ),
                    false,
                    String::new(),
                )
            }
        }
        Err(e) => (
            args_summary,
            format!("entity search error: {e}"),
            false,
            String::new(),
        ),
    }
}

// ── list_sources ────────────────────────────────────────────────────────────

fn dispatch_list_sources(content_root: &Path, call: &InnerCall) -> (String, String, bool, String) {
    let content_type = call
        .args
        .get("content_type")
        .and_then(|v| v.as_str())
        .unwrap_or("all");

    log::debug!("[smart_walk] list_sources type={}", content_type);
    let args_summary = format!("type={}", content_type);

    let mut listing = Vec::new();

    let types_to_scan: Vec<&str> = match content_type {
        "all" => vec!["raw", "wiki", "document", "episodic"],
        t => vec![t],
    };

    for ctype in types_to_scan {
        let dir = content_root.join(ctype);
        if !dir.exists() {
            listing.push(format!("  {ctype}/: (empty)"));
            continue;
        }

        match std::fs::read_dir(&dir) {
            Ok(entries) => {
                let mut subdirs: Vec<String> = entries
                    .flatten()
                    .filter(|e| e.path().is_dir())
                    .filter_map(|e| e.file_name().into_string().ok())
                    .collect();
                subdirs.sort();

                if subdirs.is_empty() {
                    listing.push(format!("  {ctype}/: (no subdirectories)"));
                } else {
                    let count = subdirs.len();
                    let preview: Vec<&str> = subdirs.iter().map(|s| s.as_str()).take(10).collect();
                    listing.push(format!(
                        "  {ctype}/ ({count} sources): {}{}",
                        preview.join(", "),
                        if count > 10 { ", ..." } else { "" }
                    ));
                }
            }
            Err(e) => listing.push(format!("  {ctype}/: error: {e}")),
        }
    }

    (
        args_summary,
        format!("Content sources:\n{}", listing.join("\n")),
        false,
        String::new(),
    )
}

// ── read_content ────────────────────────────────────────────────────────────

fn dispatch_read_content(content_root: &Path, call: &InnerCall) -> (String, String, bool, String) {
    let path_str = call
        .args
        .get("path")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    if path_str.is_empty() {
        return (
            "path=<empty>".into(),
            "error: read_content requires a non-empty path".into(),
            false,
            String::new(),
        );
    }

    let requested = Path::new(&path_str);
    if requested.is_absolute() || path_str.contains("..") {
        return (
            format!("path={path_str}"),
            "error: path must stay within the content root".into(),
            false,
            String::new(),
        );
    }

    log::debug!("[smart_walk] read_content path={}", path_str);

    let full_path = content_root.join(requested);
    if !full_path.exists() {
        return (
            format!("path={path_str}"),
            format!("file not found: {path_str}"),
            false,
            String::new(),
        );
    }

    let canonical_root = match content_root.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return (
                format!("path={path_str}"),
                format!("error resolving content root: {e}"),
                false,
                String::new(),
            );
        }
    };
    let canonical_path = match full_path.canonicalize() {
        Ok(p) => p,
        Err(e) => {
            return (
                format!("path={path_str}"),
                format!("error resolving path: {e}"),
                false,
                String::new(),
            );
        }
    };
    if !canonical_path.starts_with(&canonical_root) {
        return (
            format!("path={path_str}"),
            "error: path escapes content root".into(),
            false,
            String::new(),
        );
    }

    match std::fs::read_to_string(&canonical_path) {
        Ok(content) => {
            let truncated: String = content.chars().take(MAX_FILE_READ_BYTES).collect();
            let was_truncated = content.len() > MAX_FILE_READ_BYTES;
            let suffix = if was_truncated {
                format!("\n\n[...truncated, {} total chars]", content.len())
            } else {
                String::new()
            };
            (
                format!("path={path_str}"),
                format!("{truncated}{suffix}"),
                false,
                String::new(),
            )
        }
        Err(e) => (
            format!("path={path_str}"),
            format!("error reading: {e}"),
            false,
            String::new(),
        ),
    }
}

// ── browse_tree ─────────────────────────────────────────────────────────────

async fn dispatch_browse_tree(
    config: &Config,
    namespace: &str,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let node_id = call
        .args
        .get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("root")
        .to_string();

    log::debug!("[smart_walk] browse_tree node_id={}", node_id);

    let config_owned = config.clone();
    let ns_owned = namespace.to_string();
    let id_owned = node_id.clone();

    let result = tokio::task::spawn_blocking(move || {
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
            out.push_str("Children: (none — leaf node)\n");
        } else {
            out.push_str(&format!("Children ({}):\n", children.len()));
            for c in &children {
                let preview: String = c.summary.chars().take(100).collect();
                out.push_str(&format!(
                    "  - id={} level={:?}: {}\n",
                    c.node_id, c.level, preview
                ));
            }
        }
        out
    })
    .await
    .unwrap_or_else(|_| format!("error building context for node {node_id}"));

    (format!("node_id={node_id}"), result, false, String::new())
}

// ── vector_search ───────────────────────────────────────────────────────────

async fn dispatch_vector_search(
    config: &Config,
    call: &InnerCall,
) -> (String, String, bool, String) {
    let query = call
        .args
        .get("query")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();

    let source_kind = call
        .args
        .get("source_kind")
        .and_then(|v| v.as_str())
        .and_then(|s| match s {
            "chat" => Some(SourceKind::Chat),
            "email" => Some(SourceKind::Email),
            "document" => Some(SourceKind::Document),
            _ => None,
        });

    let time_window_days = call
        .args
        .get("time_window_days")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32);

    if query.is_empty() {
        return (
            "query=<empty>".into(),
            "error: vector_search requires a non-empty query".into(),
            false,
            String::new(),
        );
    }

    log::debug!(
        "[smart_walk] vector_search query={} source_kind={:?} window_days={:?}",
        query,
        source_kind,
        time_window_days
    );
    let args_summary = format!(
        "query=\"{}\" kind={:?} window={:?}",
        truncate_chars(&query, 40),
        source_kind,
        time_window_days
    );

    match retrieval::query_source(
        config,
        None,
        source_kind,
        time_window_days,
        Some(&query),
        10,
    )
    .await
    {
        Ok(resp) => {
            if resp.hits.is_empty() {
                (
                    args_summary,
                    format!("no vector matches for \"{}\"", query),
                    false,
                    String::new(),
                )
            } else {
                let formatted: Vec<String> = resp
                    .hits
                    .iter()
                    .map(|h| {
                        let preview: String = h.content.chars().take(120).collect();
                        format!("  [{}] (score={:.2}) {}", h.node_id, h.score, preview)
                    })
                    .collect();
                (
                    args_summary,
                    format!(
                        "{} semantic matches:\n{}",
                        formatted.len(),
                        formatted.join("\n")
                    ),
                    false,
                    String::new(),
                )
            }
        }
        Err(e) => (
            args_summary,
            format!("vector search error: {e}"),
            false,
            String::new(),
        ),
    }
}

// ── collect_evidence ────────────────────────────────────────────────────────

fn dispatch_collect_evidence(
    call: &InnerCall,
    evidence: &mut Vec<Evidence>,
) -> (String, String, bool, String) {
    let items = call
        .args
        .get("items")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    if items.is_empty() {
        return (
            "items=[]".into(),
            "error: collect_evidence requires non-empty items array".into(),
            false,
            String::new(),
        );
    }

    let mut added = 0;
    for item in &items {
        if evidence.len() >= MAX_EVIDENCE_ITEMS {
            break;
        }
        let source_path = item
            .get("source")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown")
            .to_string();
        let snippet = item
            .get("snippet")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let relevance = item
            .get("relevance")
            .and_then(|v| v.as_str())
            .unwrap_or("relevant")
            .to_string();

        if !snippet.is_empty() {
            evidence.push(Evidence {
                source_path,
                snippet,
                relevance,
            });
            added += 1;
        }
    }

    log::debug!(
        "[smart_walk] collect_evidence added={} total={}",
        added,
        evidence.len()
    );

    (
        format!("{added} items"),
        format!(
            "collected {added} evidence items (total: {})",
            evidence.len()
        ),
        false,
        String::new(),
    )
}

// ── answer ──────────────────────────────────────────────────────────────────

fn dispatch_answer(call: &InnerCall) -> (String, String, bool, String) {
    let text = call
        .args
        .get("text")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    log::debug!("[smart_walk] answer text_len={}", text.len());
    ("(final answer)".into(), text.clone(), true, text)
}

// ── Prompts ─────────────────────────────────────────────────────────────────

fn build_system_prompt() -> String {
    r#"You are a smart memory retrieval agent. Your task is to answer queries by
searching through a user's personal memory — which includes raw files (emails,
chats, commits, documents), wiki summaries, episodic conversation memories,
and document archives.

## Strategy

Use a multi-strategy approach inspired by graph-based retrieval:

1. **Start broad**: Use `list_sources` to understand what content is available,
   then `keyword_search` or `vector_search` to find relevant starting points.

2. **Follow connections**: When you find a relevant entity or topic, use
   `entity_search` to find related entities and follow the connections.

3. **Drill into details**: Use `read_content` to read specific files for
   full context. Use `browse_tree` to navigate wiki summary hierarchies.

4. **Collect evidence**: As you find relevant information, use `collect_evidence`
   to save snippets. This builds your citation buffer for the final answer.

5. **Synthesize**: When you have enough evidence, use `answer` to provide a
   comprehensive response with citations.

## Rules

- Be efficient: don't re-search for things you already found.
- Prefer vector_search for semantic/conceptual queries.
- Prefer keyword_search for specific names, IDs, or exact phrases.
- Use entity_search when the query mentions people, projects, or organizations.
- Always collect_evidence before answering, so your answer has citations.
- Use XML tool_call tags for actions.
- You can call multiple tools in one turn by including multiple <tool_call> blocks."#
        .into()
}

fn build_inner_tools_text() -> String {
    r#"## Available tools

**keyword_search** `{"pattern": "<text>", "content_type": "all|raw|wiki|document|episodic"}`
Search for a text pattern (case-insensitive) across memory files. Returns matching file paths and line previews.

**vector_search** `{"query": "<semantic query>", "source_kind": "chat|email|document", "time_window_days": 30}`
Semantic similarity search over indexed summaries. All params except query are optional.

**entity_search** `{"query": "<name or term>", "kinds": ["person", "email", "url", "handle"]}`
Find entities (people, emails, URLs, handles) in the entity index. kinds is optional.

**list_sources** `{"content_type": "all|raw|wiki|document|episodic"}`
List available content sources and their subdirectories.

**read_content** `{"path": "<relative/path/to/file.md>"}`
Read a specific content file. Path is relative to the content root (e.g. "raw/github-com-example/commits/123.md").

**browse_tree** `{"node_id": "root"}`
Navigate the wiki summary tree. Returns node summary and children. Use "root" to start.

**collect_evidence** `{"items": [{"source": "<path>", "snippet": "<text>", "relevance": "<why relevant>"}]}`
Save evidence snippets for citation in your final answer. Call this as you find relevant information.

**answer** `{"text": "<final synthesized answer with citations>"}`
Return your final answer. Reference collected evidence by source path."#
        .into()
}

// ── Content inventory ───────────────────────────────────────────────────────

fn build_content_inventory(content_root: &Path) -> String {
    let mut parts = Vec::new();

    for (label, subdir) in &[
        ("Raw content", "raw"),
        ("Wiki summaries", "wiki"),
        ("Documents", "document"),
        ("Episodic memories", "episodic"),
    ] {
        let dir = content_root.join(subdir);
        if dir.exists() {
            let count = count_files_recursive(&dir);
            if count > 0 {
                parts.push(format!("- **{label}** ({subdir}/): {count} files"));
            }
        }
    }

    if parts.is_empty() {
        "No content files found.".into()
    } else {
        parts.join("\n")
    }
}

fn count_files_recursive(dir: &Path) -> usize {
    let mut count = 0;
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                count += count_files_recursive(&path);
            } else if path.extension().map_or(false, |e| e == "md") {
                count += 1;
            }
        }
    }
    count
}

// ── Model resolution ────────────────────────────────────────────────────────

const DEFAULT_SMART_WALK_MODEL: &str = "hint:summarization";

fn resolve_walk_model(config: &Config) -> String {
    // 1. Explicit smart_walk_model config takes priority
    if let Some(ref swm) = config.memory_tree.smart_walk_model {
        if !swm.is_empty() {
            return swm.clone();
        }
    }
    // 2. Default to summarization-v1 (routed through the OpenHuman backend)
    DEFAULT_SMART_WALK_MODEL.to_string()
}

// ── Tool call parser ────────────────────────────────────────────────────────

fn parse_tool_calls(response: &str) -> (String, Vec<InnerCall>) {
    let mut calls: Vec<InnerCall> = Vec::new();
    let mut text_parts: Vec<&str> = Vec::new();
    let mut remaining: &str = response;

    const OPEN: &str = "<tool_call>";
    const CLOSE: &str = "</tool_call>";

    loop {
        match remaining.find(OPEN) {
            None => {
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
                    None => break,
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

// ── Fallback synthesis ──────────────────────────────────────────────────────

fn synthesize_fallback(trace: &[SmartWalkStep], evidence: &[Evidence]) -> String {
    let mut out = String::new();

    if !evidence.is_empty() {
        out.push_str("Based on the evidence collected:\n\n");
        for (i, ev) in evidence.iter().enumerate() {
            out.push_str(&format!(
                "{}. [{}] {}: {}\n",
                i + 1,
                ev.source_path,
                ev.relevance,
                truncate_chars(&ev.snippet, 150)
            ));
        }
    } else if !trace.is_empty() {
        out.push_str("Could not converge on an answer. Steps taken:\n\n");
        for s in trace {
            out.push_str(&format!(
                "- Turn {}: {} → {}\n",
                s.turn,
                s.action,
                truncate_chars(&s.result_preview, 100)
            ));
        }
    } else {
        out.push_str("Could not converge on an answer — no steps taken.");
    }
    out
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::inference::provider::traits::ChatMessage;
    use async_trait::async_trait;
    use std::sync::Mutex;
    use tempfile::TempDir;

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
                return Err(anyhow::anyhow!("StubProvider: no more responses"));
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
                return Err(anyhow::anyhow!("StubProvider: no more responses"));
            }
            Ok(responses.remove(0))
        }
    }

    fn test_config(tmp: &TempDir) -> Config {
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().join("workspace");
        std::fs::create_dir_all(&cfg.workspace_dir).unwrap();
        cfg
    }

    fn seed_content(content_root: &Path) {
        let raw_dir = content_root.join("raw").join("test-source").join("commits");
        std::fs::create_dir_all(&raw_dir).unwrap();
        std::fs::write(
            raw_dir.join("123_abc.md"),
            "---\nsource_kind: document\n---\n# Test Commit\nFixed the login bug in auth module.\n",
        )
        .unwrap();

        let doc_dir = content_root.join("document").join("test-doc");
        std::fs::create_dir_all(&doc_dir).unwrap();
        std::fs::write(
            doc_dir.join("readme.md"),
            "---\nsource_kind: document\n---\n# README\nProject documentation for the auth system.\n",
        )
        .unwrap();

        let wiki_dir = content_root
            .join("wiki")
            .join("summaries")
            .join("source-test");
        std::fs::create_dir_all(wiki_dir.join("L1")).unwrap();
        std::fs::write(
            wiki_dir.join("L1").join("summary-001.md"),
            "---\nkind: summary\nlevel: 1\n---\nSummary of auth changes in May 2026.\n",
        )
        .unwrap();
    }

    #[tokio::test]
    async fn smart_walk_keyword_search_and_answer() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_content(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: keyword search for "login"
            r#"<tool_call>{"name":"keyword_search","arguments":{"pattern":"login","content_type":"all"}}</tool_call>"#,
            // Turn 2: read the matching file
            r#"<tool_call>{"name":"read_content","arguments":{"path":"raw/test-source/commits/123_abc.md"}}</tool_call>"#,
            // Turn 3: collect evidence and answer
            r#"<tool_call>{"name":"collect_evidence","arguments":{"items":[{"source":"raw/test-source/commits/123_abc.md","snippet":"Fixed the login bug in auth module.","relevance":"directly mentions login fix"}]}}</tool_call>
<tool_call>{"name":"answer","arguments":{"text":"The login bug was fixed in the auth module, as documented in commit 123_abc."}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 10,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "What happened with the login bug?", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert!(outcome.answer.contains("login"));
        assert_eq!(outcome.evidence.len(), 1);
        assert!(outcome.evidence[0].snippet.contains("login bug"));
    }

    #[tokio::test]
    async fn smart_walk_list_sources() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_content(&content_root);

        let provider = StubProvider::new(vec![
            // Turn 1: list sources
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"all"}}</tool_call>"#,
            // Turn 2: answer
            r#"<tool_call>{"name":"answer","arguments":{"text":"Found raw, document, and wiki content."}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 5,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "What sources are available?", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::Answered);
        assert!(outcome.answer.contains("raw"));
    }

    #[tokio::test]
    async fn smart_walk_max_turns() {
        let tmp = TempDir::new().unwrap();
        let cfg = test_config(&tmp);
        let content_root = cfg.workspace_dir.join("memory_tree").join("content");
        seed_content(&content_root);

        let provider = StubProvider::new(vec![
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"all"}}</tool_call>"#,
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"raw"}}</tool_call>"#,
            r#"<tool_call>{"name":"list_sources","arguments":{"content_type":"wiki"}}</tool_call>"#,
        ]);

        let opts = SmartWalkOptions {
            max_turns: 3,
            namespace: "default".into(),
            model: Some("test-model".into()),
            content_root: Some(content_root),
        };

        let outcome = run_smart_walk(&cfg, &provider, "loop test", opts)
            .await
            .unwrap();

        assert_eq!(outcome.stopped_reason, SmartWalkStopReason::MaxTurnsReached);
        assert_eq!(outcome.turns_used, 3);
    }

    #[test]
    fn parse_multiple_tool_calls() {
        let response = r#"Let me search.
<tool_call>{"name":"keyword_search","arguments":{"pattern":"test"}}</tool_call>
<tool_call>{"name":"entity_search","arguments":{"query":"Alice"}}</tool_call>"#;

        let (text, calls) = parse_tool_calls(response);
        assert_eq!(calls.len(), 2);
        assert_eq!(calls[0].name, "keyword_search");
        assert_eq!(calls[1].name, "entity_search");
        assert!(text.contains("Let me search"));
    }

    #[test]
    fn content_inventory_counts_files() {
        let tmp = TempDir::new().unwrap();
        let content_root = tmp.path().join("content");
        seed_content(&content_root);

        let inventory = build_content_inventory(&content_root);
        assert!(inventory.contains("Raw content"));
        assert!(inventory.contains("Documents"));
        assert!(inventory.contains("Wiki summaries"));
    }

    // ── Staging integration tests (run with --ignored) ────────────────

    fn staging_content_root() -> Option<std::path::PathBuf> {
        let path = std::path::PathBuf::from(
            "/Users/enamakel/.openhuman-staging/users/69d9cb73e61f755583c3671f/workspace/memory_tree/content",
        );
        if path.exists() {
            Some(path)
        } else {
            None
        }
    }

    #[test]
    #[ignore]
    fn staging_keyword_search_finds_steven() {
        let content_root = staging_content_root().expect("staging content not available");
        let mut results = Vec::new();
        search_dir_recursive(
            &content_root.join("raw"),
            "steven",
            &mut results,
            &content_root,
        );
        println!("keyword 'steven': {} results", results.len());
        for r in results.iter().take(5) {
            println!("  {}", r);
        }
        assert!(
            !results.is_empty(),
            "should find 'steven' in staging raw content"
        );
    }

    #[test]
    #[ignore]
    fn staging_content_inventory() {
        let content_root = staging_content_root().expect("staging content not available");
        let inventory = build_content_inventory(&content_root);
        println!("Inventory:\n{}", inventory);
        assert!(inventory.contains("Raw content"));
        assert!(inventory.contains("Documents"));
    }

    #[test]
    #[ignore]
    fn staging_list_sources_shows_github() {
        let content_root = staging_content_root().expect("staging content not available");
        let call = InnerCall {
            name: "list_sources".into(),
            args: serde_json::json!({"content_type": "all"}),
        };
        let (_, result, _, _) = dispatch_list_sources(&content_root, &call);
        println!("list_sources:\n{}", result);
        assert!(result.contains("raw/"), "should list raw sources");
    }

    #[test]
    #[ignore]
    fn staging_read_wiki_summary() {
        let content_root = staging_content_root().expect("staging content not available");
        let wiki_dir = content_root.join("wiki").join("summaries");
        if !wiki_dir.exists() {
            println!("no wiki summaries found — skipping");
            return;
        }
        // Find first summary file
        let first = walkdir_first_md(&wiki_dir);
        if let Some(path) = first {
            let rel = path
                .strip_prefix(&content_root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            println!("Reading wiki: {}", rel);
            let call = InnerCall {
                name: "read_content".into(),
                args: serde_json::json!({"path": rel}),
            };
            let (_, result, _, _) = dispatch_read_content(&content_root, &call);
            println!("Content preview: {}", &result[..result.len().min(300)]);
            assert!(
                !result.starts_with("error"),
                "should read wiki file without error"
            );
        }
    }

    #[test]
    #[ignore]
    fn staging_read_episodic_memory() {
        let content_root = staging_content_root().expect("staging content not available");
        let ep_dir = content_root.join("episodic");
        if !ep_dir.exists() {
            println!("no episodic memories — skipping");
            return;
        }
        let first = walkdir_first_md(&ep_dir);
        if let Some(path) = first {
            let rel = path
                .strip_prefix(&content_root)
                .unwrap()
                .to_string_lossy()
                .to_string();
            println!("Reading episodic: {}", rel);
            let call = InnerCall {
                name: "read_content".into(),
                args: serde_json::json!({"path": rel}),
            };
            let (_, result, _, _) = dispatch_read_content(&content_root, &call);
            println!("Content preview: {}", &result[..result.len().min(300)]);
            assert!(
                !result.starts_with("error"),
                "should read episodic file without error"
            );
        }
    }

    #[test]
    #[ignore]
    fn staging_full_smart_walk_keyword_pipeline() {
        let content_root = staging_content_root().expect("staging content not available");

        // Simulate the pipeline: list_sources → keyword_search → read_content
        let call = InnerCall {
            name: "list_sources".into(),
            args: serde_json::json!({"content_type": "raw"}),
        };
        let (_, sources, _, _) = dispatch_list_sources(&content_root, &call);
        println!("Step 1 - Sources:\n{}", sources);

        let call = InnerCall {
            name: "keyword_search".into(),
            args: serde_json::json!({"pattern": "memory", "content_type": "all"}),
        };
        let (_, search_result, _, _) = dispatch_keyword_search(&content_root, &call);
        println!("Step 2 - Search 'memory':\n{}", search_result);

        if search_result.contains("[") {
            // Extract first file path from results
            if let Some(path_start) = search_result.find('[') {
                if let Some(path_end) = search_result[path_start + 1..].find(']') {
                    let file_path = &search_result[path_start + 1..path_start + 1 + path_end];
                    println!("Step 3 - Reading: {}", file_path);
                    let call = InnerCall {
                        name: "read_content".into(),
                        args: serde_json::json!({"path": file_path}),
                    };
                    let (_, content, _, _) = dispatch_read_content(&content_root, &call);
                    println!(
                        "Step 3 - Content ({} chars): {}",
                        content.len(),
                        &content[..content.len().min(200)]
                    );
                    assert!(
                        !content.starts_with("error"),
                        "pipeline should complete without errors"
                    );
                }
            }
        }
    }

    fn walkdir_first_md(dir: &std::path::Path) -> Option<std::path::PathBuf> {
        fn recurse(dir: &std::path::Path) -> Option<std::path::PathBuf> {
            for entry in std::fs::read_dir(dir).ok()?.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    if let Some(found) = recurse(&path) {
                        return Some(found);
                    }
                } else if path.extension().map_or(false, |e| e == "md") {
                    return Some(path);
                }
            }
            None
        }
        recurse(dir)
    }
}
