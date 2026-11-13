//! [`ToolOutputMiddleware`]: the `after_tool` ladder every tool result passes
//! through before it enters the transcript — TinyJuice (LLM summary, then
//! content-aware compaction), per-tool char cap, shared byte-budget backstop,
//! disclosure.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;

use tinyagents_harness::context::RunContext;
use tinyagents_harness::error::Result as TaResult;
use tinyagents_harness::events::AgentEvent;
use tinyagents_harness::middleware::{Middleware, ToolInvocationIdentity};
use tinyinference_llm::tool::ToolCall as TaToolCall;
use tinytools::{ToolPolicy as TaToolPolicy, ToolResult as TaToolResult};

use crate::agent::harness::tool_result_artifacts::{
    artifact_read_target, page_artifact_read, HostArtifactStore,
    TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE,
};
use crate::agent::tinyagents::payload_summarizer::DynPayloadSummarizer;
use crate::inference::tokenjuice::generate::GenerateTicket;
use tinyagents_harness::artifacts::tool_results::{ArtifactRead, ToolResultArtifactOutcome};
use tinyagents_harness::middleware::library::{
    plan_output_stages, run_output_ladder, CompactedOutput, OutputCapPolicy, OutputLadderInput,
    OutputProcessor,
};
use tinyjuice_bus::types::AgentTokenjuiceCompression;

/// TinyJuice's own estimate: `ceil(characters / 4)`, not bytes. Multibyte
/// content has more bytes than characters, so a byte-based estimate here
/// would register a summary ticket TinyJuice's own threshold check would
/// call `NotNeeded` and silently skip — a wasted prepare-and-summarize call
/// for content that never gets summarized.
fn estimate_output_tokens(content: &str) -> u64 {
    content.chars().count().div_ceil(4) as u64
}

/// Tools whose results are self-describing JSON payloads that downstream
/// extractors and the frontend canvas parse structurally (the `type` marker
/// must survive). Compacting/summarizing them destroys the contract and
/// serves no purpose — the model doesn't benefit from a tabulated graph and
/// the payload is the turn's final output, not intermediate context.
///
/// These tools are exempt from *every* content-rewriting stage below —
/// tokenjuice compaction (steps 1+2) **and** the per-tool char cap / shared
/// byte-budget backstop (steps 3+4, see [`is_truncation_exempt`]). Both
/// `flows::ops::extract_workflow_proposal` and the frontend's
/// `parseWorkflowProposal` parse this content as a single whole-string JSON
/// document; a byte-cap truncation at a UTF-8 boundary produces invalid JSON
/// just as surely as tokenjuice tabulation strips the `"type"` marker — both
/// end in a silent `proposal: None` and a blank canvas. A ≥10-node graph
/// routinely clears the ~16 KiB shared budget, so the truncation exemption
/// matters just as much as the compaction one.
pub(crate) const COMPACTION_EXEMPT_TOOLS: &[&str] = &[
    "propose_workflow",
    "revise_workflow",
    "edit_workflow",
    "save_workflow",
    "create_workflow",
];

/// Tools whose results the model reads to derive an exact schema (e.g.
/// `primary_array_path` / `output_fields`) from a *real* sampled tool
/// response, per the B12 output-probe contract (`flows::builder_tools`).
/// TokenJuice's array-elision tabulation defeats their purpose outright — a
/// tabulated sample hides the very array shape the model is calling the tool
/// to observe, so it derives a wrong or nonexistent `split_out.path` from the
/// summary instead of the real response. They're compaction-exempt
/// ([`is_compaction_exempt`]) for that reason.
///
/// Unlike [`COMPACTION_EXEMPT_TOOLS`], their payload is intermediate context
/// the model reasons over — not the turn's final machine-parsed output — and
/// samples can be genuinely large (a full API response body). So they stay
/// subject to the per-tool char cap / shared byte-budget backstop
/// ([`is_truncation_exempt`] returns `false` for them): a truncated-but-not-
/// tabulated sample is still a usable (if partial) real response, and the
/// backstop keeps these calls from blowing the context budget.
pub(crate) const SAMPLING_TOOLS: &[&str] = &["get_tool_output_sample", "get_tool_contract"];

/// Tool **discovery** listings: the catalogue a bridged server answers
/// `tools/list` with.
///
/// These are not a tool's output. They are the model's only way to learn that
/// a tool exists and what arguments it takes, so every content-rewriting stage
/// below is not "shrinking a result" but "removing capability" — and removing
/// it silently, which is the part that costs turns.
///
/// Observed, on a `tools/list` over an MCP server publishing 30 tools with
/// full JSON schemas: the response ran past the 16 KiB budget, was cut at byte
/// 16000 and spilled to an artifact. The two tools at the tail of the
/// catalogue fell off the end. The model had been told in its system prompt
/// that one of them existed, could not find it in the listing, and so narrated
/// what it meant to do instead of calling anything. On other turns it called a
/// tool it *had* seen with a guessed argument name and took the refusal. The
/// tell was the model itself reaching for
/// `file_read(path="…/mcp_list_tools/….txt", offset=16000)` — it knew the
/// catalogue had been cut and was trying to page past the boundary.
///
/// Truncation-exempt for that reason, and compaction-exempt for the same
/// reason [`SAMPLING_TOOLS`] are: a catalogue is a uniform object-array of
/// many rows, exactly what tokenjuice tabulates into a `[json table: …]`
/// marker, and tabulating away the schemas is indistinguishable from not
/// having listed them.
///
/// The honest cost: a server with a very large catalogue now spends that many
/// bytes of context. That is the right trade — a listing the model cannot act
/// on is not cheaper, it is just wrong more quietly — but a host that wants a
/// bound should bound the *catalogue* (serve fewer tools, or page the listing),
/// not cut the bytes underneath it.
pub(crate) const DISCOVERY_TOOLS: &[&str] = &["mcp_list_tools", "mcp_list_servers"];

/// Steps 1 (TinyJuice summary) + 2 (tokenjuice compaction) exemption:
/// proposal tools (final-output contract, see [`COMPACTION_EXEMPT_TOOLS`]),
/// sampling tools (tabulation would corrupt the schema they exist to reveal,
/// see [`SAMPLING_TOOLS`]), discovery listings (tabulating away a
/// catalogue's schemas is indistinguishable from not having listed them, see
/// [`DISCOVERY_TOOLS`]) and the REPL tools.
///
/// The REPL tools (`juice_find`, `juice_extract`, `juice_summarize`) are how the
/// model reads a result TinyJuice already stored behind a handle. Compacting one
/// of their answers would store it behind a second handle and hand the model a
/// preview of a preview, an endless descent. They stay subject to steps 3+4:
/// each declares a `max_result_size_chars`, so the answer is bounded by the tool
/// itself and never needs a summary to fit.
pub(crate) fn is_compaction_exempt(name: &str) -> bool {
    COMPACTION_EXEMPT_TOOLS.contains(&name)
        || SAMPLING_TOOLS.contains(&name)
        || DISCOVERY_TOOLS.contains(&name)
        || crate::inference::tokenjuice::is_repl_tool(name)
}

/// Steps 3 (per-tool char cap) + 4 (shared byte-budget backstop) exemption:
/// proposal tools and discovery listings. A proposal's JSON is parsed as a
/// single whole-string document downstream, so any truncation — not just
/// tokenjuice tabulation — breaks the parse; a cut catalogue silently drops
/// whichever tools sit past the boundary. Sampling tools are deliberately
/// *not* in this set: see [`SAMPLING_TOOLS`] for why the byte cap stays in
/// force for them.
pub(crate) fn is_truncation_exempt(name: &str) -> bool {
    COMPACTION_EXEMPT_TOOLS.contains(&name) || DISCOVERY_TOOLS.contains(&name)
}

/// Whether this call is a `web_fetch` that asked for the body **as sent**
/// (`raw: true`), following `use_skill` into the tool it wraps exactly as
/// [`artifact_read_target`] does.
///
/// Such a result is exempt from the payload summarizer (step 2). `web_fetch`
/// normally returns HTML as Markdown — TinyJuice’s `ExtractHtml` bus operation, which drops scripts and styling — and `raw: true` turns
/// that off, so the payload is unconverted markup. Paying a full-price,
/// *uncached* model call to have an LLM paraphrase minified JS and CSS is the
/// worst trade in the ladder: one observed `raw: true` fetch of a 183 KB page
/// cost 44,561 prompt tokens, over half that turn's entire summarizer budget,
/// to re-describe a page the same turn had already read as clean Markdown.
///
/// It is also the wrong answer to the question asked. A caller who wants the
/// body as sent wants the bytes, not a summary of them; steps 3–4 still bound
/// the result and spill the remainder to an artifact the model pages with
/// `file_read`, which returns the real markup, losslessly and without a model
/// call.
fn is_raw_fetch(tool_name: &str, args: &serde_json::Value) -> bool {
    const FETCH_TOOL: &str = "web_fetch";
    let (name, args) = if tool_name == "use_skill" {
        match (args.get("tool").and_then(|t| t.as_str()), args.get("args")) {
            (Some(inner), Some(inner_args)) => (inner, inner_args),
            _ => return false,
        }
    } else {
        (tool_name, args)
    };
    name == FETCH_TOOL && args.get("raw").and_then(|r| r.as_bool()).unwrap_or(false)
}

/// `after_tool`: apply the semantic payload summarizer (when configured) and
/// then the hard per-tool-result byte cap to each tool result's model-facing
/// content, before it enters the transcript. The graph analogue of the byte cap
/// + `payload_summarizer` interception the in-house `agent_tool_exec` ran.
pub(crate) struct ToolOutputMiddleware {
    /// Fallback per-tool-result byte cap for tools that don't declare their own.
    pub(crate) budget_bytes: usize,
    /// The model behind TinyJuice's summary stage. `None` means this agent's
    /// results are never summarized.
    pub(crate) payload_summarizer: Option<Arc<DynPayloadSummarizer>>,
    pub(crate) artifact_store: Option<HostArtifactStore>,
    pub(crate) tokenjuice_compaction_enabled: bool,
    pub(crate) tokenjuice_compression: AgentTokenjuiceCompression,
    /// Config resolved when the turn was constructed; avoids a disk reload from
    /// the deep `after_tool` stack.
    pub(crate) runtime_config: Option<Arc<crate::config::Config>>,
    /// SDK policy snapshot keyed by tool name. Used to honor the adapter-mapped
    /// `max_result_size_chars()` cap without re-querying the OpenHuman tool
    /// trait from `after_tool`.
    pub(crate) tool_policies: HashMap<String, TaToolPolicy>,
    /// Calls that read a persisted artifact, keyed by call id. Filled in
    /// `before_tool`, where the arguments are visible, and consumed in
    /// `after_tool`, where they are not.
    pub(crate) artifact_reads: Mutex<HashMap<String, ArtifactRead>>,
    /// `summary_focus` values taken out of calls in `before_tool`, keyed by
    /// call id, for the summary of the same call's result.
    pub(crate) focus_by_call: Mutex<HashMap<String, String>>,
    /// Tools whose schema carries TinyJuice's `summary_focus` property. Only
    /// their calls lose the argument; any other tool with a parameter of the
    /// same name (an MCP server's, say) keeps it.
    pub(crate) summary_focus_tools: HashSet<String>,
    /// Calls that asked `web_fetch` for the raw body, keyed by call id. Filled
    /// in `before_tool`, where the arguments are visible, and consumed in
    /// `after_tool`, where they are not — the same seam `artifact_reads` uses,
    /// and for the same reason. See [`is_raw_fetch`].
    pub(crate) raw_fetches: Mutex<std::collections::HashSet<String>>,
    /// Calls that only print files the agent can read again (`cat`, `sed -n`,
    /// `file_read`, ...), keyed by call id; the same `before_tool` →
    /// `after_tool` seam. Their results skip the LLM summary. See
    /// `tinytools_std::filesystem::is_read_only_call`.
    pub(crate) file_reads: Mutex<std::collections::HashSet<String>>,
}

impl ToolOutputMiddleware {
    /// The tool's own declared cap, if any. The adapter maps OpenHuman's
    /// `max_result_size_chars()` into `ToolRuntime.max_result_bytes`; preserving
    /// char-based truncation here keeps the existing model-facing marker stable.
    pub(crate) fn tool_char_cap(&self, name: &str) -> Option<usize> {
        self.tool_policies
            .get(name)
            .and_then(|policy| policy.runtime.max_result_bytes)
    }

    /// Register a summary call bound to this turn, when this agent has a
    /// summary model and the result is at least TinyJuice's threshold.
    ///
    /// `None` means no summary is wanted; `Some(Err(()))` means one was and
    /// the call could not be prepared, which the result must disclose.
    fn summary_ticket(
        &self,
        ctx: &RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        tool_name: &str,
        content: &str,
    ) -> Option<Result<GenerateTicket, ()>> {
        let summarizer = self.payload_summarizer.as_ref()?;
        let threshold_tokens = self
            .runtime_config
            .as_ref()
            .map(|config| config.context.summarizer_payload_threshold_tokens)
            .unwrap_or_default();
        if threshold_tokens == 0 || estimate_output_tokens(content) < threshold_tokens as u64 {
            return None;
        }
        match summarizer.prepare(ctx) {
            Ok(prepared) => Some(Ok(crate::inference::tokenjuice::generate::register(
                prepared,
            ))),
            Err(error) => {
                tracing::warn!(
                    tool = tool_name,
                    error = %error,
                    "[tinyagents::mw] could not prepare a summary call; compacting without one"
                );
                Some(Err(()))
            }
        }
    }
}

/// The ladder's per-tool facts: this middleware's shared budget, the cap each
/// tool declares, and the exempt name sets above. The harness decides what
/// applies from them ([`plan_output_stages`]).
impl OutputCapPolicy for ToolOutputMiddleware {
    fn default_budget_bytes(&self) -> usize {
        self.budget_bytes
    }

    fn tool_cap(&self, tool: &str) -> Option<usize> {
        self.tool_char_cap(tool)
    }

    fn is_compaction_exempt(&self, tool: &str) -> bool {
        is_compaction_exempt(tool)
    }

    fn is_truncation_exempt(&self, tool: &str) -> bool {
        is_truncation_exempt(tool)
    }

    /// An artifact nobody can read back is worse than the processed copy, so
    /// a body larger than `file_read` will open is not kept whole.
    fn max_persistable_bytes(&self) -> u64 {
        tinytools_std::filesystem::FileReadTool::MAX_FILE_SIZE_BYTES
    }
}

/// The host's side of the harness ladder: TinyJuice's summary and
/// compaction, and indexing a spilled artifact in the run store.
struct TinyJuiceProcessor<'a> {
    mw: &'a ToolOutputMiddleware,
    ctx: &'a mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
    file_read: bool,
}

#[async_trait]
impl OutputProcessor for TinyJuiceProcessor<'_> {
    async fn compact(
        &mut self,
        tool_name: &str,
        content: String,
        focus: Option<String>,
    ) -> CompactedOutput {
        // Bind a summary call to this turn only when the result is big enough
        // for TinyJuice to want one. Never for a file read.
        let (ticket, unprepared) = match (!self.file_read)
            .then(|| self.mw.summary_ticket(self.ctx, tool_name, &content))
            .flatten()
        {
            Some(Ok(ticket)) => (Some(ticket), false),
            Some(Err(())) => (None, true),
            None => (None, false),
        };
        // Summary reuse and the failure breaker are per scope. A turn with no
        // thread still gets one for the life of this run.
        let scope = self
            .ctx
            .data
            .thread_id
            .clone()
            .unwrap_or_else(|| format!("run-{}", self.ctx.instance_id()));
        let before_bytes = content.len();
        let before_tokens = estimate_output_tokens(&content);
        let compacted = crate::inference::tokenjuice::compact_tool_output(
            crate::inference::tokenjuice::ToolOutputCompaction {
                content,
                tool_name,
                enabled: self.mw.tokenjuice_compaction_enabled,
                profile: self.mw.tokenjuice_compression,
                runtime_config: self.mw.runtime_config.as_ref(),
                arguments: None,
                focus,
                context_token: ticket.as_ref().map(|t| t.token().to_string()),
                scope: Some(scope),
            },
        )
        .await;
        drop(ticket);
        if let Some(bytes) = compacted.summarized_from_bytes {
            tracing::info!(
                tool = tool_name,
                from_bytes = bytes,
                to_bytes = compacted.text.len(),
                "[tinyagents::mw] tinyjuice summarized tool output"
            );
        }
        let notice = compacted
            .notice
            .or_else(|| unprepared.then(crate::inference::tokenjuice::summary_failed_notice));
        if notice.is_some() {
            tracing::warn!(
                tool = tool_name,
                bytes = compacted.text.len(),
                "[tinyagents::mw] tinyjuice summary unavailable; disclosing raw output"
            );
        }
        if compacted.text.len() < before_bytes {
            self.ctx.emit(AgentEvent::Compressed {
                from_tokens: before_tokens,
                to_tokens: estimate_output_tokens(&compacted.text),
            });
        }
        CompactedOutput {
            text: compacted.text,
            summarized_from_bytes: compacted.summarized_from_bytes,
            notice,
        }
    }

    async fn persisted(
        &mut self,
        tool_name: &str,
        call_id: &str,
        outcome: &ToolResultArtifactOutcome,
    ) {
        let Some(path) = outcome.artifact_path.as_deref() else {
            return;
        };
        let Some(store) = self.ctx.stores.get(TINYAGENTS_TOOL_RESULT_ARTIFACT_STORE) else {
            return;
        };
        let mut fields = serde_json::Map::new();
        fields.insert("tool".to_string(), tool_name.into());
        fields.insert("call_id".to_string(), call_id.into());
        fields.insert("artifact_path".to_string(), path.to_string().into());
        fields.insert(
            "original_bytes".to_string(),
            serde_json::Value::from(outcome.original_bytes as u64),
        );
        fields.insert(
            "preview_bytes".to_string(),
            serde_json::Value::from(outcome.final_bytes as u64),
        );
        let indexed: tinyagents_harness::Result<()> =
            store.put("tool_results", call_id, fields.into()).await;
        match indexed {
            Err(err) => tracing::warn!(
                tool = tool_name,
                call_id,
                error = %err,
                "[tinyagents::mw] failed to index tool_result_artifact"
            ),
            Ok(()) => tracing::debug!(
                tool = tool_name,
                call_id,
                artifact_path = %path,
                "[tinyagents::mw] indexed tool_result_artifact in run store"
            ),
        }
    }
}

#[async_trait]
impl Middleware<(), crate::agent::tinyagents::host::OpenHumanRunContext> for ToolOutputMiddleware {
    fn name(&self) -> &str {
        "tool_output_budget"
    }

    async fn before_tool(
        &self,
        _ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        call: &mut TaToolCall,
    ) -> TaResult<()> {
        // Taken out before validation and before the tool runs: it is an
        // argument to the summary of the result, not to the tool. Only from a
        // tool that declared it; for any other tool it is the tool's own.
        let focus = if self.summary_focus_tools.contains(&call.name) {
            crate::inference::tokenjuice::focus::take_summary_focus(&mut call.arguments)
        } else {
            None
        };
        if let Some(focus) = focus {
            tracing::debug!(
                tool = %call.name,
                call_id = %call.id,
                focus_chars = focus.chars().count(),
                "[tinyagents::mw] summary_focus captured"
            );
            if let Ok(mut by_call) = self.focus_by_call.lock() {
                by_call.insert(call.id.clone(), focus);
            }
        }
        if let Some(read) =
            artifact_read_target(self.artifact_store.as_deref(), &call.name, &call.arguments)
        {
            tracing::debug!(
                tool = %call.name,
                call_id = %call.id,
                path = %read.path,
                offset = read.offset,
                "[tinyagents::mw] call reads a persisted tool-result artifact"
            );
            // `file_read` opens the filesystem; the body lives on the blob store.
            if let Some(store) = self.artifact_store.as_ref() {
                store.resolve_read(&mut call.arguments, &read).await;
            }
            if let Ok(mut reads) = self.artifact_reads.lock() {
                reads.insert(call.id.clone(), read);
            }
        }
        if tinytools_std::filesystem::is_read_only_call(&call.name, &call.arguments) {
            tracing::debug!(
                tool = %call.name,
                call_id = %call.id,
                "[tinyagents::mw] file read: its result skips the payload summarizer"
            );
            if let Ok(mut reads) = self.file_reads.lock() {
                reads.insert(call.id.clone());
            }
        }
        if is_raw_fetch(&call.name, &call.arguments) {
            tracing::debug!(
                tool = %call.name,
                call_id = %call.id,
                "[tinyagents::mw] raw fetch: exempting the result from the payload summarizer"
            );
            if let Ok(mut raw) = self.raw_fetches.lock() {
                raw.insert(call.id.clone());
            }
        }
        Ok(())
    }

    async fn after_tool(
        &self,
        ctx: &mut RunContext<crate::agent::tinyagents::host::OpenHumanRunContext>,
        _state: &(),
        invocation: &ToolInvocationIdentity,
        result: &mut TaToolResult,
    ) -> TaResult<()> {
        let tool_name = invocation.tool_name();
        let call_id = invocation.call_id().to_string();
        let mut content = crate::agent::tinyagents::middleware::tool_result_text(result);
        // A read of a persisted artifact is the model following the envelope's
        // `read_with` instruction. Every stage below would defeat it: the
        // summarizer re-summarizes the body the model asked to see, TokenJuice
        // compacts it, and the byte budget persists it as a *new* artifact with
        // the same bounded preview — a loop that never reaches the data (#6284).
        // Serve it verbatim, one bounded page at a time.
        // Consumed unconditionally so the entry cannot outlive its call, even on
        // the artifact-read early return below.
        let raw_fetch = self
            .raw_fetches
            .lock()
            .ok()
            .is_some_and(|mut raw| raw.remove(&call_id));
        let file_read = self
            .file_reads
            .lock()
            .ok()
            .is_some_and(|mut reads| reads.remove(&call_id));
        let artifact_read = self
            .artifact_reads
            .lock()
            .ok()
            .and_then(|mut reads| reads.remove(&call_id));
        if let Some(read) = artifact_read {
            if let Ok(mut by_call) = self.focus_by_call.lock() {
                by_call.remove(&call_id);
            }
            tracing::info!(
                tool = tool_name,
                path = %read.path,
                offset = read.offset,
                bytes = content.len(),
                "[tinyagents::mw] artifact read: skipping summarizer, compaction and re-persistence"
            );
            content = page_artifact_read(content, &read, self.budget_bytes);
            crate::agent::tinyagents::middleware::replace_tool_result_text(result, content);
            return Ok(());
        }

        // Which stages apply to which tool is decided by the harness ladder
        // ([`run_output_ladder`]) from [`OutputCapPolicy`]; see
        // [`is_compaction_exempt`]/[`is_truncation_exempt`] for the sets and why.
        let plan = plan_output_stages(
            self,
            tool_name,
            content.len(),
            self.artifact_store.is_some(),
        );
        if plan.compaction_exempt {
            tracing::debug!(
                tool = tool_name,
                bytes = content.len(),
                "[tinyagents::mw] compaction-exempt: skipping tokenjuice + payload summarizer"
            );
        }
        if plan.truncation_exempt {
            tracing::debug!(
                tool = tool_name,
                bytes = content.len(),
                "[tinyagents::mw] truncation-exempt: skipping per-tool char cap + shared byte-budget backstop"
            );
        }
        let focus = self
            .focus_by_call
            .lock()
            .ok()
            .and_then(|mut focus| focus.remove(&call_id));
        let wants_tinyjuice =
            self.tokenjuice_compaction_enabled || self.payload_summarizer.is_some();
        // A `raw: true` `web_fetch` asked for the body as sent; a summary of
        // minified markup is an uncached model call spent paraphrasing JS (one
        // observed fetch cost 44,561 prompt tokens). The cap still bounds it
        // and spills the rest to an artifact. See [`is_raw_fetch`].
        if raw_fetch {
            tracing::info!(
                tool = tool_name,
                bytes = content.len(),
                "[tinyagents::mw] raw fetch: skipping the tinyjuice summary, \
                 capping and spilling to an artifact instead"
            );
        }
        // A file read within the budget goes in verbatim; past it, it never
        // gets an LLM summary (see `tinytools_std::filesystem::is_read_only_call`).
        let verbatim_file_read =
            file_read && (plan.budget_bytes == 0 || content.len() <= plan.budget_bytes);
        if file_read {
            tracing::info!(
                tool = tool_name,
                bytes = content.len(),
                budget_bytes = plan.budget_bytes,
                verbatim = verbatim_file_read,
                "[tinyagents::mw] file read: no payload summary"
            );
        }
        let mut processor = TinyJuiceProcessor {
            mw: self,
            ctx,
            file_read,
        };
        let content = run_output_ladder(
            self,
            &mut processor,
            OutputLadderInput {
                tool: tool_name,
                call_id: &call_id,
                content,
                focus,
                wants_compaction: wants_tinyjuice && !raw_fetch && !verbatim_file_read,
                store: self.artifact_store.as_deref(),
                summary_label: "openhuman",
            },
        )
        .await;

        crate::agent::tinyagents::middleware::replace_tool_result_text(result, content);

        Ok(())
    }
}

#[cfg(test)]
#[path = "tool_output_tests.rs"]
mod tests;
