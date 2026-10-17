//! The single `memory` agent tool: `recall | fetch | learn | forget`.
//!
//! Registered only while memory is on (see [`crate::tools::ops`]). `learn`
//! stamps the stored learning with what the host knows and the model does
//! not get to choose:
//!
//! - `workspace` — the agent's working folder (`action_dir`, or the turn's
//!   isolated workspace when one is scoped);
//! - `thread_id` and `agent_id` (the memory agent id the turn runs as,
//!   [`super::scope`]);
//! - `namespace` — the layout's learnings node: the root of the acting
//!   identity's layout, which every agent under that root shares;
//! - `tool_call` — this call's name and provider-assigned id;
//! - `source.kind = agent`.
//!
//! `recall`, `fetch` and `forget` are confined to the identity's layout:
//! everything under its root, never another root's (another tenant's or
//! team's). A `reach` in the model's filter is overwritten.

use crate::tools::schema_cache::static_schema;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use serde_json::{json, Value};
use tinymemory_api::{
    MemoryMeta, Namespace, Reach, RecallMode, SourceKind, SourceRef, ThreadWindow, ToolCallRef,
};
use tinytools::{PermissionLevel, Tool, ToolCallOptions, ToolResult, ToolRunContext};

use crate::config::Config;
use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::core::runtime::DispatchCtx;

use super::error::MemoryError;
use super::ops;
pub use super::tool_budget::is_budget_refusal;
use super::types::{
    FetchParams, ForgetParams, LearnParams, RecallParams, RecallView, TurnCitation,
};

/// Most citations kept per thread between two drains.
const MAX_TURN_CITATIONS: usize = 20;

/// How many matches one `fetch` returns; the model pages on with `cursor`.
const FETCH_TOP: usize = 5;

type TurnCitations = Mutex<HashMap<String, Vec<TurnCitation>>>;

/// Citations `recall` produced during a thread's in-flight turn, drained by
/// the chat surface once the turn returns ([`take_turn_citations`]).
fn turn_citations(ctx: &DispatchCtx) -> Arc<TurnCitations> {
    crate::core::runtime::slot_for::<TurnCitations>(ctx)
}

fn record_turn_citations(
    ctx: &DispatchCtx,
    thread_id: &str,
    citations: &[tinymemory_api::Citation],
) {
    if citations.is_empty() {
        return;
    }
    let citations_by_thread = turn_citations(ctx);
    let mut all = citations_by_thread
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = all.entry(thread_id.to_string()).or_default();
    for citation in citations {
        if entry.len() >= MAX_TURN_CITATIONS {
            break;
        }
        if !entry.iter().any(|existing| existing.id == citation.id.0) {
            entry.push(TurnCitation::from(citation));
        }
    }
}

/// Records `citations` for `thread_id`'s in-flight turn: what the turn's
/// memory pack cited (`lifecycle::hooks::pre_turn`), beside what `recall`
/// cites.
pub fn record_pack_citations(ctx: &DispatchCtx, thread_id: &str, citations: Vec<TurnCitation>) {
    if citations.is_empty() {
        return;
    }
    let citations_by_thread = turn_citations(ctx);
    let mut all = citations_by_thread
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let entry = all.entry(thread_id.to_string()).or_default();
    for citation in citations {
        if entry.len() >= MAX_TURN_CITATIONS {
            break;
        }
        if !entry.iter().any(|existing| existing.id == citation.id) {
            entry.push(citation);
        }
    }
}

/// Drains the citations `recall` and the turn's pack produced for
/// `thread_id` since the last drain.
#[must_use]
pub fn take_turn_citations(ctx: &DispatchCtx, thread_id: &str) -> Vec<TurnCitation> {
    turn_citations(ctx)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .remove(thread_id)
        .unwrap_or_default()
}

/// The tool's name.
pub const MEMORY_TOOL_NAME: &str = "memory";

/// The `memory` tool.
pub struct MemoryTool {
    config: Arc<Config>,
    budget: super::tool_budget::ToolBudget,
}

impl MemoryTool {
    /// A tool bound to `config`'s engine.
    #[must_use]
    pub fn new(config: Arc<Config>) -> Self {
        Self {
            config,
            budget: super::tool_budget::ToolBudget::default(),
        }
    }
}

/// What the host knows about the call that the model does not set.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CallFacts {
    /// The agent's working folder.
    pub workspace: Option<String>,
    /// The thread the call runs in.
    pub thread_id: Option<String>,
    /// The agent that made the call.
    pub agent_id: Option<String>,
    /// The provider-assigned tool-call id.
    pub tool_call_id: Option<String>,
    /// The calling thread's turns still verbatim in the run's prompt, when
    /// the run knows them (a turn with memory on). Reads leave these out.
    pub in_prompt: Option<ThreadWindow>,
    /// The chat the call's turn is surfaced in, so the memory activity it
    /// raises reaches that chat. `None` outside a chat turn.
    pub chat: Option<crate::security::approval::ApprovalChatContext>,
    /// Where the agent's learnings go: its layout's learnings node.
    pub namespace: Namespace,
    /// What the agent may read and forget: its whole layout.
    pub reach: Reach,
}

impl CallFacts {
    /// Gathers the facts from the run context,
    /// falling back to `config.action_dir` for the workspace.
    #[must_use]
    pub fn gather(config: &Config, context: Option<&dyn ToolRunContext>) -> Self {
        let workspace = context
            .and_then(ToolRunContext::workspace_root)
            .map(std::path::Path::to_path_buf)
            .unwrap_or_else(|| config.action_dir.clone());
        // The acting identity is the run's own; direct dispatch (CLI, tests)
        // has no run, and falls back to the spawning agent, else the root.
        let identity =
            crate::agent::tinyagents::host::run_context::memory_identity_for_tool(context)
                .unwrap_or_else(|| {
                    crate::agent::tinyagents::host::run_context::parent_agent_id_for_tool(context)
                        .map_or_else(super::scope::MemoryIdentity::root, |parent_agent_id| {
                            super::scope::MemoryIdentity::agent(&parent_agent_id)
                        })
                });
        Self {
            workspace: Some(workspace.display().to_string()),
            thread_id: context
                .and_then(ToolRunContext::thread_id)
                .map(str::to_string),
            tool_call_id: crate::tools::host_extensions::tool_call_id(context),
            chat: crate::agent::tinyagents::host::run_context::approval_for_tool(context).chat,
            in_prompt: crate::agent::tinyagents::host::run_context::memory_in_prompt_for_tool(
                context,
            ),
            ..Self::of(config, &identity)
        }
    }

    /// The facts of a call made by `identity` under `config`, with no run
    /// context.
    #[must_use]
    pub fn of(config: &Config, identity: &super::scope::MemoryIdentity) -> Self {
        let resolved = identity.resolve(config);
        Self {
            agent_id: Some(resolved.agent_id.clone()),
            namespace: resolved.layout.learnings().clone(),
            reach: Reach::subtree(resolved.root().clone()),
            ..Self::default()
        }
    }

    /// The metadata `learn` stamps on the stored item.
    #[must_use]
    pub fn learn_meta(&self) -> MemoryMeta {
        MemoryMeta {
            namespace: self.namespace.clone(),
            workspace: self.workspace.clone(),
            thread_id: self.thread_id.clone(),
            agent_id: self.agent_id.clone(),
            tool_call: Some(ToolCallRef {
                name: MEMORY_TOOL_NAME.to_string(),
                id: self.tool_call_id.clone(),
            }),
            source: SourceRef {
                kind: SourceKind::Agent,
                id: None,
            },
            ..MemoryMeta::default()
        }
    }
}

/// What a `learn` result tells the model: the write was accepted, and
/// recall may not find it for a moment.
pub const LEARN_STATUS: &str = "saved; searchable in recall shortly";

fn arg_str(args: &Value, key: &str) -> Option<String> {
    args.get(key)
        .and_then(Value::as_str)
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}

fn parse<T: serde::de::DeserializeOwned>(args: &Value) -> Result<T, String> {
    serde_json::from_value(args.clone()).map_err(|error| format!("invalid arguments: {error}"))
}

/// The turns a read inside a thread leaves out: those of the calling thread
/// the agent's prompt still holds verbatim. Turns compaction folded into a
/// checkpoint are readable again, so a long thread can recall what it said
/// before; when the run does not know its window, the whole thread is left
/// out.
fn calling_thread(facts: &CallFacts) -> Option<ThreadWindow> {
    let thread_id = facts.thread_id.as_deref()?;
    let window = facts
        .in_prompt
        .clone()
        .filter(|window| window.thread_id == thread_id)
        .unwrap_or_else(|| ThreadWindow::whole(thread_id));
    tracing::debug!(
        from_turn = window.from_turn,
        "[memory:tool] reads leave out the thread's turns still in the prompt"
    );
    Some(window)
}

/// What `recall` shows the model: the evidence to answer from and how many
/// items it holds (their ids are kept as the turn's citations), or a note
/// when nothing was recalled. The citations' snippets repeat the evidence,
/// so they are left out.
fn evidence_json(view: &RecallView) -> Value {
    if view.answer.trim().is_empty() {
        json!({ "evidence": "", "note": "Nothing relevant is stored in memory." })
    } else {
        json!({ "evidence": capped_evidence(&view.answer), "items": view.citations.len() })
    }
}

/// About how many tokens of evidence `recall` hands the model (counted as
/// four characters a token). The rest stays reachable through `fetch`.
const EVIDENCE_TOKENS: usize = 2_500;

/// The engine's closing note when a recall searched memory only in part.
const PARTLY_SEARCHED: &str = "\n\n(Memory was only partly searched";

/// `evidence` cut to about [`EVIDENCE_TOKENS`] tokens, only ever at the end
/// of a line: whole `---`-separated entries in rank order, then as many whole
/// lines of the next entry as fit (marked `[…]`), then a note on what was
/// left out. A line is never split, so a single line longer than the budget
/// is kept whole when it is the first. The engine's closing note (memory
/// only partly searched) is always kept.
fn capped_evidence(evidence: &str) -> String {
    const SEPARATOR: &str = "\n\n---\n\n";
    let budget = EVIDENCE_TOKENS * 4;
    if evidence.len() <= budget {
        return evidence.to_string();
    }
    let (body, closing) = match evidence.rfind(PARTLY_SEARCHED) {
        Some(at) => (&evidence[..at], &evidence[at..]),
        None => (evidence, ""),
    };
    let entries: Vec<&str> = body.split(SEPARATOR).collect();
    let mut out = String::new();
    let mut whole = 0;
    let mut cut_short = false;
    for entry in &entries {
        let joint = if out.is_empty() { "" } else { SEPARATOR };
        if out.len() + joint.len() + entry.len() <= budget {
            out.push_str(joint);
            out.push_str(entry);
            whole += 1;
            continue;
        }
        // The entry does not fit: keep the whole lines of it that do.
        let mut lines = 0;
        for line in entry.split('\n') {
            let piece = if lines == 0 { joint.len() } else { 1 } + line.len();
            if out.len() + piece > budget && !out.is_empty() {
                break;
            }
            out.push_str(if lines == 0 { joint } else { "\n" });
            out.push_str(line);
            lines += 1;
        }
        if lines == entry.split('\n').count() {
            whole += 1;
        } else if lines > 0 {
            out.push_str("\n[…]");
            cut_short = true;
        }
        break;
    }
    let left = entries.len() - whole - usize::from(cut_short);
    if left > 0 || cut_short {
        let shortened = if cut_short {
            ", the last one shortened,"
        } else {
            ""
        };
        out.push_str(&format!(
            "\n\n({left} more memory entries left out{shortened} to keep this under about \
             {EVIDENCE_TOKENS} tokens; search again with more specific words to see \
             others.)"
        ));
    }
    tracing::debug!(
        entries = entries.len(),
        whole,
        cut_short,
        "[memory:tool] recall evidence capped"
    );
    out.push_str(closing);
    out
}

/// Runs one action. Errors become a model-visible tool error, never a hard
/// failure, so the model can adapt (memory off, unsupported mode, …).
pub async fn run_action(
    ctx: &DispatchCtx,
    config: &Config,
    args: &Value,
    facts: &CallFacts,
) -> ToolResult {
    let action = arg_str(args, "action").unwrap_or_default();
    tracing::debug!(action = %action, "[memory:tool] dispatch");
    let result: Result<Value, String> = match action.as_str() {
        // The agent answers from the evidence itself, so recall writes no
        // answer of its own; and the thread the call runs in is left out of
        // recall and fetch, since its user turn is logged before the tool
        // reads and would otherwise come back as a memory.
        "recall" => match parse::<RecallParams>(args) {
            Ok(mut params) => {
                confine(&mut params.filter, facts);
                // How much recall reads is the host's call, not the model's.
                params.limit = None;
                let question = params.question.clone();
                ops::recall_with(config, params, RecallMode::Evidence, calling_thread(facts))
                    .await
                    .map(|view| {
                        if let Some(thread_id) = facts.thread_id.as_deref() {
                            record_turn_citations(ctx, thread_id, &view.citations);
                        }
                        BUS.publish(DomainEvent::MemoryRecalled {
                            query: question,
                            hit_count: view.citations.len(),
                            thread_id: facts.chat.as_ref().map(|chat| chat.thread_id.clone()),
                            client_id: facts.chat.as_ref().map(|chat| chat.client_id.clone()),
                        });
                        evidence_json(&view)
                    })
                    .map_err(render_error)
            }
            Err(error) => Err(error),
        },
        "fetch" => match parse::<FetchParams>(args) {
            Ok(mut params) => {
                confine(&mut params.filter, facts);
                // Five at a time, always; more come through `cursor`.
                params.limit = Some(FETCH_TOP);
                ops::fetch_with(config, params, calling_thread(facts))
                    .await
                    .map(|view| {
                        let mut out =
                            json!({ "matches": fetch_page(&view), "items": view.hits.len() });
                        if let Some(cursor) = view.next_cursor {
                            out["next_cursor"] = json!(cursor);
                        }
                        out
                    })
                    .map_err(render_error)
            }
            Err(error) => Err(error),
        },
        "learn" => match parse::<LearnParams>(args) {
            Ok(mut params) => {
                params.meta = None;
                let kind = params.kind.unwrap_or(tinymemory_api::LearningKind::Fact);
                // Accepted, not visible: indexing can take minutes on a
                // hosted engine and the turn must not wait for it.
                ops::learn_with(
                    config,
                    params,
                    Some(facts.learn_meta()),
                    tinymemory_api::WriteOptions::accepted(),
                )
                .await
                .map(|view| {
                    BUS.publish(DomainEvent::MemoryStored {
                        key: view.id.clone(),
                        category: kind.as_str().to_string(),
                        namespace: "learnings".to_string(),
                        thread_id: facts.chat.as_ref().map(|chat| chat.thread_id.clone()),
                        client_id: facts.chat.as_ref().map(|chat| chat.client_id.clone()),
                    });
                    json!({"id": view.id, "status": LEARN_STATUS})
                })
                .map_err(render_error)
            }
            Err(error) => Err(error),
        },
        "forget" => match parse::<ForgetParams>(args) {
            Ok(mut params) => {
                params.reach = Some(facts.reach.clone());
                ops::forget(config, params)
                    .await
                    .map(|view| json!(view))
                    .map_err(render_error)
            }
            Err(error) => Err(error),
        },
        other => Err(format!(
            "unknown action `{other}`; use one of recall, fetch, learn, forget"
        )),
    };
    match result {
        Ok(value) => ToolResult::success(value.to_string()),
        Err(message) => ToolResult::error(message),
    }
}

/// What `fetch` shows the model: its top matches as dated lines, best first,
/// each with the id `forget` takes. The engine's `next_cursor`, when there
/// is one, rides beside this text; the tool description says how to use it.
fn fetch_page(view: &super::types::FetchView) -> String {
    let mut out = match view.hits.len() {
        0 => "Nothing in memory matches.".to_string(),
        1 => "1 match.".to_string(),
        n => format!("Top {n} matches, best first."),
    };
    for hit in &view.hits {
        out.push_str("\n\n");
        if let Some(at) = hit.meta.observed_at {
            out.push_str(&format!("[{}] ", at.format("%Y-%m-%d %H:%M UTC")));
        }
        match hit.kind {
            tinymemory_api::ItemKind::Learning => out.push_str("learning: "),
            tinymemory_api::ItemKind::Document => out.push_str("document: "),
            tinymemory_api::ItemKind::Conversation => {}
        }
        out.push_str(hit.text.trim());
        out.push_str(&format!("\n(id {})", hit.id));
    }
    tracing::debug!(
        hits = view.hits.len(),
        "[memory:tool] fetch matches rendered"
    );
    out
}

/// Confines a model-supplied filter to the calling agent's reach.
fn confine(filter: &mut Option<tinymemory_api::MetaFilter>, facts: &CallFacts) {
    filter.get_or_insert_with(Default::default).reach = Some(facts.reach.clone());
}

fn render_error(error: MemoryError) -> String {
    format!("{} ({})", error, error.code())
}

#[async_trait]
impl Tool for MemoryTool {
    fn name(&self) -> &str {
        MEMORY_TOOL_NAME
    }

    fn description(&self) -> &str {
        "Long-term memory across conversations, documents and learnings. \
         `recall` searches memory for a question and returns what it holds (facts and \
         dated conversation lines) for you to answer from; it does not answer for you. \
         Search matches meaning and wording: if a search misses something, search again \
         with different words before concluding it is not in memory, and search \
         separately for each part of a question that combines several things. `fetch` \
         returns matching items five at a time, best first, each with its id (filter by \
         metadata such as workspace, repo, file_path, kinds). When it also returns \
         `next_cursor`, more items match: for the next five of the same search, call fetch \
         again with the same query and filter and `cursor` set to it. A cursor only \
         continues the search it came from; for different words, fetch without one. \
         `learn` queues a durable fact, preference, procedure or correction about the \
         user or their work, shared with every agent working alongside you; `forget` \
         queues removal of items by id. Write acknowledgements mean locally queued, \
         not yet saved or removed remotely; do not repeat them to force indexing. \
         Cached memory may be added as <memory-context>; use `recall` or `fetch` \
         for current information, and set `refers_to` when \
         the question is about a particular time. Recall before asking \
         the user something they may already have told you; learn things worth \
         remembering next time."
    }

    fn parameters_schema(&self) -> Value {
        static_schema!(include_str!("parameters/memory.json"))
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    fn permission_level_with_args(&self, args: &Value) -> PermissionLevel {
        match args.get("action").and_then(Value::as_str) {
            Some("recall" | "fetch") => PermissionLevel::ReadOnly,
            _ => PermissionLevel::Write,
        }
    }

    fn is_concurrency_safe(&self, args: &Value) -> bool {
        matches!(
            args.get("action").and_then(Value::as_str),
            Some("recall" | "fetch")
        )
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        self.execute_with_context(args, ToolCallOptions::default(), None)
            .await
    }

    async fn execute_with_context(
        &self,
        args: Value,
        _options: ToolCallOptions,
        context: Option<&dyn ToolRunContext>,
    ) -> anyhow::Result<ToolResult> {
        let facts = CallFacts::gather(&self.config, context);
        let dispatch = crate::agent::tinyagents::host::run_context::dispatch_ctx_for_tool(context);
        let run_id = context
            .and_then(ToolRunContext::host_extension)
            .and_then(|any| any.downcast_ref::<tinyagents_harness::tool::ToolExecutionContext>())
            .map(|ctx| ctx.run_id.as_str().to_string());
        // Writes are local queue entries, bounded by the outbox (`tool_writes`),
        // so only engine reads spend the run's call and latency budget.
        if matches!(
            args.get("action").and_then(Value::as_str),
            Some("learn" | "forget")
        ) {
            let config = self.config.clone();
            let write_args = args.clone();
            let queued_root = facts.reach.at.clone();
            let queued = dispatch
                .spawn_blocking(move || super::tool_writes::enqueue(&config, &write_args, &facts))
                .await;
            return Ok(
                match queued.unwrap_or_else(|_| {
                    Err(MemoryError::Unavailable(
                        "memory write queue unavailable".into(),
                    ))
                }) {
                    Ok(value) => {
                        super::tool_writes::schedule(&dispatch, self.config.clone(), &queued_root);
                        ToolResult::success(value.to_string())
                    }
                    Err(error) => ToolResult::error(render_error(error)),
                },
            );
        }
        Ok(self
            .budget
            .run(run_id, run_action(&dispatch, &self.config, &args, &facts))
            .await)
    }
}

#[cfg(test)]
#[path = "tools_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "tools_agent_tests.rs"]
mod agent_tests;

#[cfg(test)]
#[path = "tools_schema_tests.rs"]
mod schema_tests;

#[cfg(test)]
#[path = "tools_budget_tests.rs"]
mod budget_tests;

#[cfg(test)]
#[path = "tools_read_tests.rs"]
mod read_tests;
