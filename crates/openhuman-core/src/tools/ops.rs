use super::*;

use crate::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::config::{Config, DelegateAgentConfig};
use crate::runtime::javascript::NodeBootstrap;
use crate::runtime::python::PythonBootstrap;
use crate::security::{AuditLogger, SecurityPolicy};
use std::collections::HashMap;
use std::sync::Arc;
use tinytools::Tool;
#[cfg(test)]
use tinytools::{ToolResult, ToolSpec};

pub(crate) use super::capability::tool_capability;

/// Derive the browser tool's host allowlist from the unified web-access list
/// (`http_request.allowed_domains`).
///
/// The browser tool shares the single fetch allowlist rather than the
/// deprecated `[browser].allowed_domains`, but the `"*"` allow-all wildcard is
/// stripped on purpose: `web_fetch`/`curl` treat `"*"` as "open to all public
/// sites", whereas the browser (a real Chromium with JS, cookies, and
/// logged-in sessions) must NOT inherit blanket access from a fetch-side
/// toggle. Browser allow-all stays gated by `OPENHUMAN_BROWSER_ALLOW_ALL`
/// (`allow_all_browser_domains()`), and the tool itself stays behind
/// `browser.enabled`. Net effect is fail-safe: unifying can only ever narrow
/// the browser's reach, never widen it.
pub(crate) fn browser_allowed_domains(http_allowed_domains: &[String]) -> Vec<String> {
    http_allowed_domains
        .iter()
        .filter(|domain| domain.as_str() != "*")
        .cloned()
        .collect()
}

/// Create the default tool registry
pub fn default_tools(security: Arc<SecurityPolicy>) -> Vec<Box<dyn Tool>> {
    default_tools_with_runtime(security, Arc::new(NativeRuntime::new()))
}

/// Create the default tool registry with explicit runtime adapter.
///
/// Convenience entry point used by tests and the lightweight CLI surface.
/// Production assembly sites use [`all_tools_with_runtime`] and pass a real
/// [`AuditLogger`]; this wrapper substitutes [`AuditLogger::disabled`] so
/// existing test callers do not need to plumb one through.
pub fn default_tools_with_runtime(
    security: Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
) -> Vec<Box<dyn Tool>> {
    let audit = AuditLogger::disabled();
    vec![
        Box::new(ShellTool::new(security.clone(), runtime, audit)),
        Box::new(FileReadTool::new(security.clone())),
        Box::new(FileWriteTool::new(security)),
    ]
}

/// Create full tool registry including memory tools.
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    audit: Arc<AuditLogger>,
    browser_config: &crate::config::BrowserConfig,
    http_config: &crate::config::HttpRequestConfig,
    action_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    root_config: &crate::config::Config,
) -> Vec<Box<dyn Tool>> {
    all_tools_with_runtime(
        config,
        security,
        Arc::new(NativeRuntime::new()),
        audit,
        browser_config,
        http_config,
        action_dir,
        agents,
        root_config,
        None,
    )
}

/// Create full tool registry including memory tools.
///
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools_with_runtime(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    audit: Arc<AuditLogger>,
    browser_config: &crate::config::BrowserConfig,
    http_config: &crate::config::HttpRequestConfig,
    action_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    root_config: &crate::config::Config,
    approval_workspace_root: Option<&std::path::Path>,
) -> Vec<Box<dyn Tool>> {
    // One shared snapshot of this session's configuration for both language
    // clients. They each hand it to the `tinyruntime` module on every call —
    // the module holds no configuration of its own — so the two must not be
    // able to disagree about which version this session asked for. The
    // registry is assembled under `config`, so the bootstraps share that same
    // Arc rather than a separately-cloned `root_config` — one configuration
    // snapshot for everything this session builds.
    let shared_config = Arc::clone(&config);

    // Build a session-scoped managed Node.js bootstrap once, so ShellTool,
    // NodeExecTool, and NpmExecTool all share the same memoised resolution
    // state. Disabled when `node.enabled = false` — in that case shell skips
    // PATH injection and node/npm tools are not registered.
    // `runtime-node` off => never construct a bootstrap: the stub resolves to
    // nothing anyway, and this keeps the shell's PATH-injection branch dead
    // rather than a silent per-invocation no-op.
    let node_bootstrap: Option<Arc<NodeBootstrap>> = if cfg!(feature = "runtime-node")
        && root_config.node.enabled
    {
        tracing::debug!(
            version = %root_config.node.version,
            prefer_system = root_config.node.prefer_system,
            "[tools::ops] node runtime enabled — constructing shared NodeBootstrap"
        );
        Some(Arc::new(NodeBootstrap::new(Arc::clone(&shared_config))))
    } else {
        tracing::debug!(
            "[tools::ops] node runtime disabled — shell PATH injection + node_exec/npm_exec suppressed"
        );
        None
    };
    let python_bootstrap: Option<Arc<PythonBootstrap>> = if root_config.runtime_python.enabled {
        tracing::debug!(
            minimum_version = %root_config.runtime_python.minimum_version,
            prefer_system = root_config.runtime_python.prefer_system,
            "[tools::ops] python runtime enabled — constructing shared PythonBootstrap"
        );
        Some(Arc::new(PythonBootstrap::new(Arc::clone(&shared_config))))
    } else {
        tracing::debug!(
            "[tools::ops] python runtime disabled — shell python/pip PATH injection suppressed"
        );
        None
    };

    let shell: Box<dyn Tool> = Box::new(ShellTool::with_language_bootstraps(
        security.clone(),
        Arc::clone(&runtime),
        Arc::clone(&audit),
        node_bootstrap.as_ref().map(Arc::clone),
        python_bootstrap.as_ref().map(Arc::clone),
    ));

    let file_write: Box<dyn Tool> = match approval_workspace_root {
        Some(root) => Box::new(FileWriteTool::with_approval_workspace_root(
            security.clone(),
            root.to_path_buf(),
        )),
        None => Box::new(FileWriteTool::new(security.clone())),
    };

    let mut tools: Vec<Box<dyn Tool>> = vec![
        shell,
        Box::new(FileReadTool::new(security.clone())),
        file_write,
        // Coding-harness baseline tools (issue #1205): file navigation
        // + atomic editing primitives. Use these instead of falling
        // through to `shell` for grep/find/sed work.
        Box::new(GrepTool::new(security.clone())),
        Box::new(GlobTool::new(security.clone())),
        Box::new(ListFilesTool::new(security.clone())),
        Box::new(EditFileTool::new(security.clone())),
        Box::new(ApplyPatchTool::new(security.clone())),
        Box::new(CsvExportTool::new(security.clone())),
        // Sub-agent dispatch — lets the parent agent delegate focused
        // sub-tasks (research, code execution, API specialists, …) by
        // calling `spawn_subagent { agent_id, prompt, … }`. The runner
        // builds a narrow Agent from an `AgentDefinition` lookup and
        // returns a single text result. See
        // `agent::subagent_host` for the dispatch path.
        Box::new(SpawnSubagentTool::new()),
        Box::new(SpawnAsyncSubagentTool::new()),
        // Interactive clarification early-exit. Sub-agents pause on this tool
        // (checkpoint → `AwaitingUser`, see `subagent_host`) and
        // the orchestrator resumes them via `continue_subagent` (#4291).
        // Several agent scopes (orchestrator, crypto, markets, scheduler,
        // desktop control) name it, so it must exist in the base
        // registry or none of them can actually ask the user anything.
        Box::new(AskClarificationTool::new()),
        // Read-only project overview (git status, recent commits, top-level
        // tree) rooted at the agent action dir. Named by the orchestrator and
        // planner scopes.
        Box::new(WorkspaceStateTool::new(action_dir.to_path_buf())),
        // "Plan mode as a subagent": runs the read-only `context_scout`
        // inline and returns a bounded context bundle + recommended next
        // tool calls. Visible only to agents that allowlist it
        // (orchestrator / planner).
        Box::new(AgentPrepareContextTool::new()),
        // Steer/list/close reusable async sub-agents and collect results by
        // durable `subagent_session_id` (preferred) or transient `task_id`.
        Box::new(ListSubagentsTool::new()),
        Box::new(SteerSubagentTool::new()),
        Box::new(WaitTool::new()),
        Box::new(WaitLoopTool::new()),
        Box::new(WaitSubagentTool::new()),
        Box::new(CloseSubagentTool::new()),
        Box::new(ContinueSubagentTool::new()),
        Box::new(SpawnParallelAgentsTool::new()),
        // Multi-stage durable delegation (issue #4249, Phase 3): runs the chosen
        // sub-agent through the tinyagents plan→execute→review→finalize graph,
        // checkpointed to the session DB. Heavier than spawn_subagent; for
        // sub-tasks that benefit from a self-review/revision loop.
        Box::new(DelegateGraphTool::new()),
        // The session todo list (Claude/Codex style): one whole-list write per
        // call, scoped to the conversation thread. `plan_exit` is the marker
        // that hands a plan-mode pass off to a build-mode pass.
        Box::new(TodoTool::new(root_config.workspace_dir.clone())),
        // Interactive plan-review gate: parks the live turn on a thread-scoped
        // plan the user must approve before execution (Codex/Claude plan mode).
        Box::new(crate::agent::plan_review::RequestPlanReviewTool::new()),
        Box::new(PlanExitTool::new()),
        // Workflow composition: `run_workflow` runs another workflow as a
        // subagent and (by default) waits on its result like a function call;
        // `await_workflow` re-attaches to a run that outlived its inline wait.
        // Both wrap `skill_runtime::spawn_workflow_run_background` +
        // `await_run_outcome` — the same spawn path `openhuman.skills_run`
        // JSON-RPC uses, so RPC and tool callers stay in sync.
        #[cfg(feature = "skills")]
        Box::new(RunWorkflowTool::new()),
        #[cfg(feature = "skills")]
        Box::new(AwaitWorkflowTool::new()),
        Box::new(CurrentTimeTool::new()),
        // Reversibility for native tool-output compaction (Stage 1a): when a
        // large result is compacted with a `retrieve_tool_output("<hash>")`
        // marker, this hands the original back from the CCR store on demand.
        Box::new(RetrieveToolOutputTool::new()),
        // TokenJuice 2.0 content-router retrieval: fetches the original (full or
        // by byte/line range) for a `⟦tj:<hash>⟧` marker from the CCR cache.
        // Supersedes `retrieve_tool_output`; both are kept live during migration.
        Box::new(crate::inference::tokenjuice::TokenjuiceRetrieveTool::new()),
        // Deterministic time-expression → timestamp resolver. `current_time`
        // only returns *now*, leaving the model to do epoch arithmetic by hand
        // (a real incident had an agent compute "24h ago" ~10 months off, then
        // fetch Slack history ascending from that wrong floor and miss the
        // latest messages). `resolve_time` does the conversion and returns the
        // value ready to paste into a tool argument.
        Box::new(ResolveTimeTool::new()),
        Box::new(DetectToolsTool::new()),
        Box::new(InstallToolTool::new(security.clone())),
        // The compact, advertised scheduler surface. Keep the six legacy
        // tools registered below as hidden aliases so saved transcripts and
        // skills remain replayable.
        Box::new(CronTool::new(config.clone(), security.clone())),
        Box::new(CronAddTool::new(config.clone(), security.clone())),
        Box::new(CronListTool::new(config.clone())),
        Box::new(CronRemoveTool::new(config.clone())),
        Box::new(CronUpdateTool::new(config.clone(), security.clone())),
        Box::new(CronRunTool::new(config.clone())),
        Box::new(CronRunsTool::new(config.clone())),
        // Agent-first Workflow authoring (issue B4): validates a candidate
        // graph and returns a proposal summary — never creates/enables a
        // flow itself. Only the chat UI's WorkflowProposalCard "Save &
        // enable" action calls `flows_create`.
        #[cfg(feature = "flows")]
        Box::new(ProposeWorkflowTool::new(config.clone())),
        // workflow-builder agent tool belt (Phase 5b). A deliberately narrow,
        // propose-or-read surface: revise a draft (validate-only), read saved
        // flows/runs/connections, ground tool_call slugs in the real catalog,
        // and dry-run a draft against MOCK capabilities. None of these persist
        // or enable a flow (only the user's own `flows_create` click does); the
        // read tools are `PermissionLevel::None`, and `dry_run_workflow` is
        // autonomy-tier gated + wired to deterministic mock capabilities.
        #[cfg(feature = "flows")]
        Box::new(ReviseWorkflowTool::new(config.clone())),
        // Structured incremental edits (F1): apply a small ops[] list to a base
        // graph (saved flow or inline) instead of re-emitting the whole graph,
        // then validate + gate + return a proposal (same contract as revise).
        // Proposal-only — never persists.
        #[cfg(feature = "flows")]
        Box::new(EditWorkflowTool::new(config.clone())),
        // Standalone validate (F3): run the SAME structural + hard-gate stack
        // the propose/save tools use, without emitting a proposal — a pure
        // check so the agent can self-verify a draft mid-build. Read-only.
        #[cfg(feature = "flows")]
        Box::new(ValidateWorkflowTool::new(config.clone())),
        // Read a saved flow's revision history (F6) — prior graph snapshots the
        // agent can inspect / pick a rollback target from. Read-only.
        #[cfg(feature = "flows")]
        Box::new(GetFlowHistoryTool::new(config.clone())),
        // Phase 4 self-debug loop (F4): find a failing run, resume a parked
        // run (approval-gated), or cancel a runaway one.
        #[cfg(feature = "flows")]
        Box::new(ListFlowRunsTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(ResumeFlowRunTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(CancelFlowRunTool::new(config.clone())),
        // Gated create (F4/F12): create a NEW flow — born disabled, approval
        // gated — and duplicate an existing one (disabled copy) for
        // clone-then-edit. Behind the Phase 3 safety rails.
        #[cfg(feature = "flows")]
        Box::new(CreateWorkflowTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(DuplicateFlowTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(ListFlowsTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(GetFlowTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(GetFlowRunTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(ListFlowConnectionsTool::new(config.clone())),
        #[cfg(feature = "flows")]
        Box::new(SearchToolCatalogTool::new(config.clone())),
        // Full live contract (schemas, real required_args/output_fields,
        // primary_array_path) for one action slug found via
        // search_tool_catalog — the grounding step before WIRING a node's
        // args/downstream bindings (systemic tool-contract fix, Part 1).
        #[cfg(feature = "flows")]
        Box::new(GetToolContractTool::new(config.clone())),
        // B12: ONE bounded, READ-ONLY, REAL Composio call to derive the real
        // primary_array_path/output_fields when the live listing publishes no
        // output schema at all (verified for every GitHub action) — overrides
        // get_tool_contract's schema-derived hint for that slug from then on.
        // Read-scope actions only (hard-refused otherwise), connected
        // toolkits only — see builder_tools.rs's module doc for the carve-out
        // this makes in the workflow-builder agent's "no composio_execute"
        // invariant.
        #[cfg(feature = "flows")]
        Box::new(GetToolOutputSampleTool::new(config.clone())),
        // Ground an `agent` node's `agent_ref` in real registered agent-kind ids
        // (researcher / code_executor / …) — the agent analogue of
        // search_tool_catalog. Read-only.
        #[cfg(feature = "flows")]
        Box::new(ListAgentDefinitionsTool::new()),
        // Steer toolkit choice toward what's already connected + surface which
        // toolkits a flow still needs (Phase 5, item 19). Read-only.
        #[cfg(feature = "flows")]
        Box::new(ListConnectableToolkitsTool::new(config.clone())),
        // Queryable DSL schema (F2): enumerate the 13 node kinds and fetch one
        // kind's full config-field/port/example/gotcha contract — the DSL
        // analogue of search_tool_catalog + get_tool_contract, so an agent need
        // not rely on prompt prose or memory for node config shapes. Read-only.
        #[cfg(feature = "flows")]
        Box::new(ListNodeKindsTool::new()),
        #[cfg(feature = "flows")]
        Box::new(GetNodeKindContractTool::new()),
        #[cfg(feature = "flows")]
        Box::new(DryRunWorkflowTool::new(config.clone())),
        // Real end-to-end test run of a SAVED flow (Write / external-effect). The
        // workflow-builder prompt requires it to ask the user for confirmation
        // first, and the flow's own approval gate still pauses outbound nodes.
        #[cfg(feature = "flows")]
        Box::new(RunFlowTool::new(config.clone())),
        // Persist a built graph onto an EXISTING saved flow (Write). Used only
        // when the USER explicitly asks the agent to save; the seeded build
        // turn from the Flows prompt bar is propose-only (see #4596) — Accept
        // + the canvas's own Save persist the graph. The tool itself can
        // never create a flow or change enabled/require_approval.
        #[cfg(feature = "flows")]
        Box::new(SaveWorkflowTool::new(config.clone())),
        // Flow Scout discovery: the `flow_discovery` agent's terminal emit
        // sink. Read-only reasoning over the user's data ends by calling
        // `suggest_workflows`, which persists workflow ideas for the Flows page
        // "Suggested for you" section. `PermissionLevel::None`, no external
        // effect — writes only to the agent's own suggestions store.
        #[cfg(feature = "flows")]
        Box::new(SuggestWorkflowsTool::new(config.clone())),
        // Per-flow sandboxed memory (issue #5173): lets a running flow
        // (e.g. a scheduled newsletter-digest) remember what it already did
        // — dedupe across runs — without ever touching the user's own
        // memory. Namespace is derived internally from `flow_id`.
        // `flow_memory_remember` (write) only resolves that `flow_id` from
        // the run's own trusted `TrustedAutomation { Workflow }` turn origin
        // (T-M2 fix) — a chat/orchestrator turn with no trusted run origin
        // is refused outright, never routed to a model-supplied `flow_id`.
        // `flow_memory_recall`'s `scope: "flows"` is a deliberate read-only
        // cross-flow exception — it can see every flow's namespace by
        // design, but can never be used to write outside a flow's own.
        #[cfg(feature = "flows")]
        Box::new(FlowMemoryRecallTool::new()),
        #[cfg(feature = "flows")]
        Box::new(FlowMemoryRememberTool::new(security.clone())),
        // Wallet tools — expose wallet operations to the agent tool-call pipeline
        // so the crypto sub-agent can prepare transfers, check status, etc.
        // Gated with the `web3` feature (the wallet domain is compiled out when
        // web3 is disabled; the concrete tool types live under `wallet::tools`).
        #[cfg(feature = "web3")]
        Box::new(WalletStatusTool::new()),
        #[cfg(feature = "web3")]
        Box::new(WalletChainStatusTool::new()),
        #[cfg(feature = "web3")]
        Box::new(WalletPrepareTransferTool::new()),
        #[cfg(feature = "web3")]
        Box::new(WalletTxStatusTool::new()),
        #[cfg(feature = "web3")]
        Box::new(WalletTxReceiptTool::new()),
        #[cfg(feature = "web3")]
        Box::new(WalletLookupTxTool::new()),
        // The memory surface the model sees. The eleven per-operation tools it
        // dispatches to stay registered as `ToolExposure::Hidden` so a
        // replayed transcript or a saved skill naming `memory_*` still works —
        // see `memory::tools::collapsed`.
        Box::new(crate::memory::tools::MemoryTool::new(
            config.clone(),
            security.clone(),
        )),
        Box::new(MemoryStoreTool::new(security.clone())),
        Box::new(MemoryRecallTool::new()),
        Box::new(MemoryForgetTool::new(security.clone())),
        // #4458: the memory read→dedupe→write→update-index protocol
        // (`agent::harness::memory_protocol`) can only close its write cycle via a
        // successful `update_memory_md` call, and the archivist's `[tools] named`
        // allowlist selects it — but subagents only filter the *parent* tool set,
        // so if this tool is absent from the registry the archivist silently loses
        // it and the model hits a permanent unsatisfiable "call update_memory_md"
        // nag loop (unknown-tool error → the tracker never sees IndexUpdate). It is
        // always registered here (same as the other memory tools); per-agent
        // visibility is governed by each agent's `named` allowlist. Targets the
        // workspace `MEMORY.md`/`SKILL.md` (where `channels_prompt`/`session_memory`
        // read them from), and prefers the live TinyAgents workspace descriptor at
        // execution time when one is present.
        Box::new(UpdateMemoryMdTool::new(root_config.workspace_dir.clone())),
        // #002: read-only self-diagnosis of the memory pipeline so the agent
        // can explain an empty/stalled wiki + the fix.
        Box::new(MemoryDoctorTool::new(config.clone())),
        // #5172: read-only access to the compiled persona flavour profiles
        // (communication/coding_style/stack/workflow/environment/directives/
        // anti_preferences) that persona ingestion builds but nothing
        // previously surfaced to the agent loop.
        Box::new(MemoryFlavourTool::new(config.clone())),
        Box::new(MemoryQueryTool),
        // memory_search tools — vector search, chunk context, hybrid search,
        // and previously unregistered raw store tools.
        Box::new(MemoryVectorSearchTool),
        Box::new(MemoryChunkContextTool),
        Box::new(MemoryHybridSearchTool),
        Box::new(MemoryStoreRawSearchTool),
        Box::new(MemoryStoreRawChunksTool),
        Box::new(MemoryStoreKindsTool),
        // Explicit user-preference pinning — always registered so the model
        // can save user-stated preferences regardless of whether the full
        // inference-based learning subsystem is enabled.  The preference
        // injection into the system prompt is controlled independently by
        // `config.learning.explicit_preferences_enabled`.
        Box::new(RememberPreferenceTool::new(security.clone())),
        // Two-lane explicit preferences (general → system prompt, situational →
        // per-query recall). Written verbatim to user_pref_{general,situational};
        // bypasses the inference/stability pipeline. Always registered.
        Box::new(SavePreferenceTool::new(security.clone())),
        Box::new(ScheduleTool::new(security.clone(), root_config.clone())),
        Box::new(ProxyConfigTool::new(config.clone(), security.clone())),
        Box::new(UpdateCheckTool::new()),
        Box::new(UpdateApplyTool::new(security.clone())),
        Box::new(GitOperationsTool::new(
            security.clone(),
            action_dir.to_path_buf(),
        )),
        Box::new(PushoverTool::new(
            security.clone(),
            action_dir.to_path_buf(),
        )),
        // Audio-toolkit podcast tools — gated with the `voice` feature (they
        // live in the `audio_toolkit` domain, which is compiled out when voice
        // is disabled).
        #[cfg(feature = "voice")]
        Box::new(AudioGeneratePodcastTool::new(
            config.clone(),
            security.clone(),
        )),
        #[cfg(feature = "voice")]
        Box::new(AudioEmailPodcastTool::new(config.clone(), security.clone())),
        #[cfg(feature = "voice")]
        Box::new(AudioGenerateAndEmailPodcastTool::new(
            config.clone(),
            security.clone(),
        )),
        Box::new(GmailUnsubscribeTool),
        // Skills metadata tools. `skill_run` is already exposed by RunSkillTool
        // above, so it is not duplicated. Reads ship default-ON; the
        // create/install/uninstall mutators ship default-OFF via
        // `tools::user_filter` (install also fetches remote content).
        #[cfg(feature = "skills")]
        Box::new(WorkflowListTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(WorkflowDescribeTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(SkillSearchTool::new(config.clone())),
        // Skill registry tools — browse/search/install from remote registries.
        // Browse and search are read-only (default-ON); install is a write
        // operation (fetches remote content and writes to disk).
        #[cfg(feature = "skills")]
        Box::new(SkillRegistryBrowseTool),
        #[cfg(feature = "skills")]
        Box::new(SkillRegistrySearchTool),
        #[cfg(feature = "skills")]
        Box::new(SkillRegistryInstallTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(SkillRegistrySourcesTool),
        #[cfg(feature = "skills")]
        Box::new(SkillRegistryUninstallTool),
        // Skill runtime probes — resolve the reusable Node/Python runtimes
        // that skill execution relies on before a script-backed skill runs.
        #[cfg(feature = "skills")]
        Box::new(SkillRuntimeResolveRuntimesTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(WorkflowReadResourceTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(WorkflowRecentRunsTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(WorkflowReadRunLogTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(WorkflowCreateTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(WorkflowInstallFromUrlTool::new(config.clone())),
        #[cfg(feature = "skills")]
        Box::new(WorkflowUninstallTool),
        // Learning (user-profile facet cache) tools. Reads ship default-ON;
        // every mutator ships default-OFF via `tools::user_filter`
        // (learning_manage toggle) — they persistently rewrite the assistant's
        // model of the user. enrich_profile also flags external_effect.
        Box::new(LearningListFacetsTool),
        Box::new(LearningGetFacetTool),
        Box::new(LearningCacheStatsTool),
        Box::new(LearningUpdateFacetTool),
        Box::new(LearningPinFacetTool),
        Box::new(LearningUnpinFacetTool),
        Box::new(LearningForgetFacetTool),
        Box::new(LearningRebuildCacheTool),
        Box::new(LearningResetCacheTool),
        Box::new(LearningSaveProfileTool),
        Box::new(LearningEnrichProfileTool),
        // Task & productivity tools (issue: agent-tool expansion).
        // Read/observe + bounded-write tools are registered here; the
        // destructive/overextending siblings (artifact_delete,
        // task_source_add/update/remove) are registered too but ship
        // default-OFF via `tools::user_filter` (their toggle IDs default off
        // in onboarding). The per-call permission ladder still gates them.
        Box::new(ArtifactListTool::new(config.clone())),
        Box::new(ArtifactGetTool::new(config.clone())),
        Box::new(ArtifactDeleteTool::new(config.clone())),
        Box::new(TaskSourceListTool::new(config.clone())),
        Box::new(TaskSourceGetTool::new(config.clone())),
        Box::new(TaskSourceFetchTool::new(config.clone())),
        Box::new(TaskSourceListTasksTool::new(config.clone())),
        Box::new(TaskSourcePreviewFilterTool::new(config.clone())),
        Box::new(TaskSourceStatusTool::new(config.clone())),
        Box::new(TaskSourceAddTool::new(config.clone())),
        Box::new(TaskSourceUpdateTool::new(config.clone())),
        Box::new(TaskSourceRemoveTool::new(config.clone())),
        // System & self-management: observability (default-ON) + service
        // lifecycle. doctor/health/cost/dashboard/security reads are default-ON.
        // service_status / daemon_host_prefs_get default-ON; the lifecycle
        // mutators ship default-OFF via `tools::user_filter` (service_lifecycle).
        Box::new(DoctorHealthTool::new(config.clone())),
        Box::new(DoctorModelsTool::new(config.clone())),
        Box::new(HealthSnapshotTool),
        Box::new(HealthSystemInfoTool),
        Box::new(CostDashboardTool::new(config.clone())),
        Box::new(CostDailyHistoryTool::new(config.clone())),
        Box::new(CostSummaryTool::new(config.clone())),
        Box::new(DashboardModelHealthTool::new(config.clone())),
        #[cfg(feature = "modules")]
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Apps)),
        #[cfg(feature = "modules")]
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Windows)),
        #[cfg(feature = "modules")]
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Launch)),
        #[cfg(feature = "modules")]
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Snapshot)),
        #[cfg(feature = "modules")]
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Find)),
        #[cfg(feature = "modules")]
        Box::new(DesktopTool::new(config.clone(), DesktopToolKind::Goal)),
        #[cfg(feature = "modules")]
        Box::new(DesktopTool::new(
            config.clone(),
            DesktopToolKind::ContinueGoal,
        )),
        Box::new(SecurityPolicyInfoTool::new(config.clone())),
        Box::new(ServiceStatusTool::new(config.clone())),
        Box::new(DaemonHostPrefsGetTool::new(config.clone())),
        Box::new(ServiceStartTool::new(config.clone())),
        Box::new(ServiceStopTool::new(config.clone())),
        Box::new(ServiceRestartTool),
        Box::new(ServiceShutdownTool),
        Box::new(ServiceInstallTool::new(config.clone())),
        Box::new(ServiceUninstallTool::new(config.clone())),
        Box::new(DaemonHostPrefsSetTool::new(config.clone())),
        // Config: read-only surface (default-ON). The config_update_* mutators
        // are deferred (their apply fns take non-Deserialize patch structs);
        // see config/tools.rs.
        Box::new(ConfigSnapshotTool::new(config.clone())),
        Box::new(ConfigClientConfigTool),
        Box::new(ConfigAutonomyTool),
        Box::new(ConfigSearchTool),
        Box::new(ConfigRuntimeFlagsTool),
        Box::new(ConfigResolveApiUrlTool),
        Box::new(ConfigDataPathsTool),
        // Account & money. The billing / team / referral agent-tool families were
        // removed: money movement and team administration are dashboard
        // surfaces, not things an agent should reach for mid-turn, and their
        // controllers remain registered for the UI. `credentials` exposes only
        // non-secret reads.
        Box::new(CredentialListTool::new(config.clone())),
        Box::new(SessionStateTool::new(config.clone())),
        Box::new(OAuthConnectUrlTool::new(config.clone())),
        Box::new(OAuthListTool::new(config.clone())),
        // MCP registry and workspace persona. Observe/connect/call tools
        // default-ON; MCP uninstall (mcp_manage), and persona/workspace writers
        // (workspace_manage) ship default-OFF via `tools::user_filter`. There
        // is no install tool: servers are declared by the user in mcp.json.
        //
        // MCP registry (dynamic, user-installed servers) — compiled out with
        // the `mcp` feature. Per-element attrs inside the `vec![]` mirror the
        // voice idiom used earlier in this same literal.
        #[cfg(feature = "mcp")]
        Box::new(McpRegistrySearchTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryGetTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryInstalledListTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryStatusTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryListToolsTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryConnectTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryDisconnectTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryToolCallTool::new(config.clone())),
        #[cfg(feature = "mcp")]
        Box::new(McpRegistryUninstallTool::new(config.clone())),
        Box::new(WorkspaceReadPersonaTool::new(config.clone())),
        Box::new(WorkspaceUpdatePersonaTool::new(config.clone())),
        Box::new(WorkspaceResetPersonaTool::new(config.clone())),
        Box::new(WorkspaceInitTool),
    ];

    log::debug!(
        "[tools::ops][memory_search] registered memory_vector_search, memory_chunk_context, \
         memory_hybrid_search, memory_store_raw_search, memory_store_raw_chunks, memory_store_kinds"
    );

    // Presentation generation (#2778). Native-Rust engine (ppt-rs
    // backed) as of the #2780-follow-up rust-engine refactor — no
    // managed Python venv, no first-call install latency. Always
    // registered.
    #[cfg(feature = "documents")]
    tools.push(Box::new(PresentationTool::new(
        root_config.workspace_dir.clone(),
        security.clone(),
    )));

    // Document generation (#4847, Problem 3). Native-Rust engine
    // (docx-rs backed) — no managed runtime, no subprocess — emitting a
    // real `.docx` through the same byte-agnostic artifact pipeline as
    // the presentation tool. Always registered; same constructor shape.
    #[cfg(feature = "documents")]
    tools.push(Box::new(DocumentTool::new(
        root_config.workspace_dir.clone(),
        security.clone(),
    )));

    // Long-term goals list tool. Used primarily by the background
    // `goals_agent` (which filters to it via its `[tools] named` allowlist);
    // also available to the main agent for explicit edits. One `op`-dispatched
    // tool, not four — see the module docs on `memory::tools::goals`.
    tools.push(Box::new(crate::memory::tools::goals::GoalsTool::new(
        root_config.workspace_dir.clone(),
    )));

    // Thread-level goal tools (Codex-style per-thread completion contract).
    // Visible only to agents that allowlist them (orchestrator). The target
    // thread is resolved from the ambient `thread_id`, so no thread arg is
    // taken. `goal_get`/`goal_set`/`goal_complete` — pause/resume/budget are
    // system-driven and have no model tool.
    {
        let goal_dir = root_config.workspace_dir.clone();
        tools.push(Box::new(crate::agent::goals::GoalGetTool::new(
            goal_dir.clone(),
        )));
        tools.push(Box::new(crate::agent::goals::GoalSetTool::new(
            goal_dir.clone(),
        )));
        tools.push(Box::new(crate::agent::goals::GoalCompleteTool::new(
            goal_dir,
        )));
    }

    #[cfg(feature = "modules")]
    if browser_config.enabled {
        // BrowserClient enforces the shared `http_request.allowed_domains`
        // policy for both browser tools.
        let browser_client = Arc::new(crate::modules::browser::BrowserClient::new(config.clone()));
        tools.push(Box::new(BrowserOpenTool::new(
            security.clone(),
            browser_client.clone(),
        )));
        tools.push(Box::new(BrowserTool::new(
            security.clone(),
            browser_client,
            browser_config.max_task_steps,
        )));
    }

    // HTTP request — always registered. `http_request.allowed_domains`
    // + `security` still gate which hosts are reachable; there is no
    // enable flag because every session needs basic HTTP as a baseline
    // capability.
    tools.push(Box::new(HttpRequestTool::new(
        security.clone(),
        http_config.allowed_domains.clone(),
        http_config.max_response_size,
        http_config.timeout_secs,
    )));

    // x402 — dedicated tool for making paid HTTP requests to x402-enabled
    // APIs (Base USDC / Solana USDC). Handles the 402 challenge, EIP-3009
    // or SPL payment signing, and ledger recording. Gated with the `web3`
    // feature (the x402 domain is compiled out when web3 is disabled).
    #[cfg(feature = "web3")]
    tools.push(Box::new(crate::web3::x402::tools::X402RequestTool::new()));

    // Coding-harness baseline `web_fetch` (issue #1205) — single-purpose
    // GET-and-read primitive that reuses the same allowed-domains gate
    // as `http_request`. Use this for docs/READMEs; reach for
    // `http_request` only when you need richer HTTP semantics.
    tools.push(Box::new(WebFetchTool::new(
        security.clone(),
        http_config.allowed_domains.clone(),
        Some(http_config.max_response_size),
        Some(http_config.timeout_secs),
    )));

    // curl — always registered. Shares `http_request.allowed_domains`,
    // adds streaming-to-disk with a hard byte ceiling. Writes land
    // under `<workspace>/<curl.dest_subdir>`.
    tools.push(Box::new(CurlTool::new(
        security.clone(),
        http_config.allowed_domains.clone(),
        action_dir.to_path_buf(),
        root_config.curl.dest_subdir.clone(),
        root_config.curl.max_download_bytes,
        root_config.curl.timeout_secs,
    )));

    // gitbooks — answers questions about OpenHuman by calling the
    // GitBook MCP server. Two tools mirroring the upstream MCP tools.
    if root_config.gitbooks.enabled {
        // Building the client can fail on a malformed proxy or an unusable TLS
        // setting. Both are logged and the tools are simply not registered:
        // taking the whole surface down over a documentation server would cost
        // the user every other tool for no reason.
        match (
            GitbooksSearchTool::new(
                root_config.gitbooks.endpoint.clone(),
                root_config.gitbooks.timeout_secs,
            ),
            GitbooksGetPageTool::new(
                root_config.gitbooks.endpoint.clone(),
                root_config.gitbooks.timeout_secs,
            ),
        ) {
            (Ok(search), Ok(get_page)) => {
                tools.push(Box::new(search));
                tools.push(Box::new(get_page));
                tracing::debug!("[gitbooks] registered gitbooks_search + gitbooks_get_page");
            }
            (Err(error), _) | (_, Err(error)) => {
                tracing::warn!("[gitbooks] tools not registered: {error}");
            }
        }
    }

    // Generic remote MCP bridge tools. These let the agent enumerate
    // named MCP servers and forward `tools/call` through the core
    // instead of hardcoding one bespoke MCP integration per server.
    //
    // Backed by the STATIC, config-declared server set (`[[mcp_client.servers]]`
    // in TOML) — despite the local binding's name, this is NOT the dynamic
    // `mcp::registry` domain gated above. Both are compiled out by the `mcp`
    // feature; see the static-vs-dynamic note in AGENTS.md.
    #[cfg(feature = "mcp")]
    {
        let mcp_registry = {
            // Built from the converted configuration, which is the one place
            // the two vocabularies meet. A registry that cannot be built is
            // logged and treated as empty: a malformed proxy or TLS setting
            // must not take the whole tool surface down with it.
            let base = crate::mcp::host::static_registry(root_config);
            Arc::new(base)
        };
        log::debug!(
            "[tools::ops][mcp_client] static servers={}",
            mcp_registry.list().len()
        );
        if !mcp_registry.is_empty() {
            tools.push(Box::new(McpListServersTool::new(Arc::clone(&mcp_registry))));
            tools.push(Box::new(McpListToolsTool::new(Arc::clone(&mcp_registry))));
            tools.push(Box::new(McpCallTool::new(
                Arc::clone(&mcp_registry),
                security.clone(),
            )));
            tracing::debug!(
                count = mcp_registry.list().len(),
                "[mcp_client] registered generic MCP bridge tools"
            );
        } else {
            tracing::debug!("[mcp_client] no MCP servers registered — bridge tools skipped");
        }
    }

    tools.extend(crate::search::build_search_tools(root_config));

    // Media generation (image/video via GMI through the backend). Skipped when
    // no integration client is configured; artifacts land under `action_dir`.
    // Gated by the `media` compile-time feature (#4804); absent from slim
    // builds. Runtime `DomainSet::media` (#4796) still gates it when compiled.
    #[cfg(feature = "media")]
    tools.extend(crate::media::generation::build_media_tools(
        root_config,
        action_dir,
    ));

    // Managed cloud file storage (S3 via the backend). Skipped when no
    // integration client is configured; downloads land under `action_dir`.
    tools.extend(crate::integrations::file_storage::build_file_storage_tools(
        root_config,
        action_dir,
    ));

    // Hosting tools — deploy a workspace directory to a real hosting provider,
    // with a managed database wired into it. Registered only once a credential
    // actually resolves (`[hosting].api_key`, else the provider's environment
    // variables): a tool that cannot work is worse than one that is absent,
    // because a model retries it. A misconfigured section — an unknown provider
    // slug, a blank key — is logged and skipped rather than failing startup,
    // since nothing else in the process depends on hosting.
    #[cfg(feature = "hosting")]
    match crate::hosting::Account::from_config(root_config) {
        Ok(Some(account)) => {
            let hosting_tools = account.tools();
            tracing::debug!(
                count = hosting_tools.len(),
                "[tools::ops] registered hosting tools"
            );
            tools.extend(hosting_tools);
        }
        Ok(None) => {}
        Err(error) => {
            tracing::warn!(%error, "[tools::ops] hosting is enabled but misconfigured; tools not registered");
        }
    }

    // High-level web3 tools (swaps / bridges / dapp calls) built on the wallet.
    // They call the backend deBridge proxy per-invocation and error gracefully
    // when the user is not signed in, so they register unconditionally.
    tools.extend(crate::web3::all_web3_agent_tools());

    // Managed Node.js exec tools — gated on `root_config.node.enabled`.
    // Both share the same `NodeBootstrap` as ShellTool so the download +
    // extract + install pipeline runs at most once per session.
    #[cfg(feature = "runtime-node")]
    if let Some(bootstrap) = node_bootstrap.as_ref() {
        tools.push(Box::new(NodeExecTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
            root_config.runtime_pool.clone(),
            root_config.workspace_dir.clone(),
        )));
        tools.push(Box::new(NpmExecTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
        )));
        tracing::debug!("[tools::ops] registered node_exec + npm_exec");
    }

    // Managed Python exec tool — gated on `root_config.runtime_python.enabled`.
    // Shares the same `PythonBootstrap` as ShellTool. Inline code routes through
    // the shared runtime pool (#5106) when enabled.
    if let Some(bootstrap) = python_bootstrap.as_ref() {
        tools.push(Box::new(PythonExecTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
            root_config.runtime_pool.clone(),
            root_config.workspace_dir.clone(),
        )));
        tracing::debug!("[tools::ops] registered python_exec");
    }

    // Image metadata is always available for user-provided images.
    tools.push(Box::new(ImageInfoTool::new(security.clone())));

    // Tool effectiveness stats (enabled when learning is on)
    tracing::debug!(
        learning_enabled = root_config.learning.enabled,
        tool_tracking_enabled = root_config.learning.tool_tracking_enabled,
        "evaluating ToolStatsTool registration"
    );
    if root_config.learning.enabled && root_config.learning.tool_tracking_enabled {
        tools.push(Box::new(ToolStatsTool::new()));
        tracing::debug!("ToolStatsTool registered");
    }

    // Add delegation tool when agents are configured
    if !agents.is_empty() {
        let delegate_agents: HashMap<String, DelegateAgentConfig> = agents
            .iter()
            .map(|(name, cfg)| (name.clone(), cfg.clone()))
            .collect();
        tools.push(Box::new(DelegateTool::new_with_options(
            delegate_agents,
            security.clone(),
            crate::inference::provider::ProviderRuntimeOptions {
                auth_profile_override: None,
                openhuman_dir: root_config
                    .config_path
                    .parent()
                    .map(std::path::PathBuf::from),
                secrets_encrypt: root_config.secrets.encrypt,
                reasoning_enabled: root_config.runtime.reasoning_enabled,
            },
        )));
    }

    // ── Agent integration tools (backend-proxied) ─────────────────
    if let Some(client) = crate::integrations::build_client(root_config) {
        tracing::debug!("[integrations] client built successfully");
        if root_config.integrations.google_places.is_active() {
            tools.push(Box::new(crate::tools::GooglePlacesSearchTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(crate::tools::GooglePlacesDetailsTool::new(
                Arc::clone(&client),
            )));
            tracing::debug!("[integrations] registered google_places tools");
        } else {
            tracing::debug!("[integrations] google_places disabled — skipping");
        }
        // NOTE: parallel tools moved to the unified [search] engine
        // selector above. `integrations.parallel` is parsed but no
        // longer registers tools directly — set
        // `search.engine = "parallel"` instead.
        if root_config.integrations.parallel.is_active() {
            tracing::debug!(
                "[integrations] parallel toggle is active but tools are governed by search.engine now"
            );
        }
        // TinyFish is search-owned and registers through the unified search
        // surface above so `search.engine = "disabled"` suppresses it too.
        if root_config.integrations.stock_prices.is_active() {
            tools.push(Box::new(crate::tools::StockQuoteTool::new(Arc::clone(
                &client,
            ))));
            tools.push(Box::new(crate::tools::StockExchangeRateTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(crate::tools::StockOptionsTool::new(Arc::clone(
                &client,
            ))));
            tools.push(Box::new(crate::tools::StockCryptoSeriesTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(crate::tools::StockCommodityTool::new(Arc::clone(
                &client,
            ))));
            tracing::debug!("[integrations] registered stock_prices tools");
        } else {
            tracing::debug!("[integrations] stock_prices disabled — skipping");
        }
        if root_config.integrations.twilio.is_active() {
            tools.push(Box::new(crate::tools::TwilioCallTool::new(Arc::clone(
                &client,
            ))));
            tracing::debug!("[integrations] registered twilio tools");
        } else {
            tracing::debug!("[integrations] twilio disabled — skipping");
        }
    } else {
        tracing::debug!(
            "[integrations] build_client returned None — integration tools not registered"
        );
    }

    let composio_tools = crate::integrations::composio::all_composio_agent_tools(root_config);
    if composio_tools.is_empty() {
        tracing::debug!("[integrations] composio unavailable — skipping");
    } else {
        tracing::debug!(
            count = composio_tools.len(),
            "[integrations] registered composio tools"
        );
        tools.extend(composio_tools);
    }

    // Coding-harness `lsp` tool (issue #1205) — capability-gated by the
    // OPENHUMAN_LSP_ENABLED env var. The backend (real language-server
    // bridge) is a follow-up; today the gate just controls visibility
    // so agents don't see a method that always errors.
    if crate::tools::implementations::lsp_capability_enabled() {
        tools.push(Box::new(crate::tools::implementations::LspTool::new()));
        tracing::debug!("[lsp] capability gate on — LspTool registered");
    } else {
        tracing::debug!("[lsp] capability gate off (set OPENHUMAN_LSP_ENABLED=1 to register)");
    }

    // Two INDEPENDENT post-filters over the assembled list (kernel.md §3.7's
    // separate axes — a narrowed DomainSet must not narrow capabilities, and
    // vice versa):
    //
    // 1. DomainSet (#4796): drop tools whose DomainGroup is disabled under the
    //    ambient CoreContext. With no active context, or under
    //    `DomainSet::full()`, every tool is kept (byte-identical). Under
    //    `harness()` the gate-family tools (web3/mcp/skills/flows/media/voice)
    //    are dropped so agent turns can't call a domain that isn't live;
    //    only the memory + threads tools survive (the mapped harness families)
    //    — see `tool_group` for the classification and its Platform-default
    //    caveat.
    // 2. Memory capability (M5.3): drop tools whose memory family the bound
    //    driver does not advertise — see `tool_capability`.
    //
    // Both default OPEN: with no ambient context and with nothing bound the
    // list is unchanged. Absence beats a stub that errors — a
    // registered-but-failing memory tool teaches the model the capability
    // exists and makes it retry (the `flows` compile-gate's reasoning).
    let before = tools.len();
    let domains = crate::core::runtime::context::CoreContext::current().map(|c| c.domains());
    let mut tools: Vec<Box<dyn Tool>> = if let Some(set) = domains {
        tools
            .into_iter()
            .filter(|t| set.allows(tool_group(t.name())))
            .collect()
    } else {
        // No ambient context (unit tests / pre-boot) ⇒ no domain filtering.
        tools
    };
    let after_domains = tools.len();

    tools.retain(|t| crate::core::all::capability_allowed(tool_capability(t.name())));
    let after_capabilities = tools.len();

    // 3. ToolGroups: a group an embedder set to `Off` is not registered at all.
    //    `Advertised` and `Withheld` both keep the tool here — they differ only
    //    in whether its schema reaches the provider, which is decided later by
    //    `strip_packed_from_visible`. Same default-open rule as the two filters
    //    above: with no ambient context every group is `Withheld`, so nothing
    //    is dropped and the desktop list is unchanged.
    {
        let groups = crate::tools::toolpacks::groups::current();
        tools.retain(|t| groups.mode_for_tool(t.name()) != crate::tools::toolpacks::GroupMode::Off);
    }

    log::debug!(
        "[tools::ops][post-filter] {before} assembled → {after_domains} after DomainSet → \
         {after_capabilities} after memory capabilities → {} after ToolGroups",
        tools.len()
    );

    // Append the two always-on pack tools. They resolve packed tools by name
    // out of the agent's live registry (bound after `Arc::new`), so they also
    // cover the `delegate_*` tools synthesised later by
    // `orchestrator_tools::collect_orchestrator_tools` — which never pass
    // through this function.
    crate::tools::toolpacks::append_pack_tools(&mut tools);
    // The lookup half of `ToolExposure::Deferred` is not registered here: the
    // tinyagents harness advertises its intrinsic `tool_search` / `tool_call`
    // bridge whenever a run has a deferred tool (`tool::discover`), ranked by
    // whatever `agent::tinyagents::discovery` installed. A host-registered
    // `tool_search` would shadow that bridge.
    tools
}

/// Classify an agent tool into its [`DomainGroup`](crate::core::all::DomainGroup)
/// by its `name()`, so [`all_tools_with_runtime`] can drop tools whose family is
/// disabled under the ambient [`DomainSet`](crate::core::runtime::DomainSet).
///
/// Named-family tools are matched here; everything without a domain family
/// defaults to `Platform`. Under `harness()`, the Agent/Memory/Threads/Config/
/// Security tools remain while gate-family and generic Platform tools drop.
/// (Names verified against each Tool impl's `fn name()` on 2026-07-13.)
fn tool_group(name: &str) -> crate::core::all::DomainGroup {
    use crate::core::all::DomainGroup;

    // Gate families with a domain-exclusive name prefix are matched by prefix
    // (not an exact list) so a NEW tool in the family auto-gates instead of
    // silently defaulting to Platform and leaking under a custom DomainSet
    // (#4808 maintainer review). Web3 = wallet_/web3_/x402_, Media = media_,
    // Mcp = mcp_ (below). Families without a clean prefix (Skills/Flows) keep
    // their exact lists; `no_gate_family_tool_silently_defaults_to_platform`
    // guards the prefix families.
    const SKILLS: &[&str] = &[
        "run_workflow",
        "await_workflow",
        "list_workflows",
        "create_skill",
        "describe_workflow",
        "read_workflow_resource",
        "list_workflow_runs",
        "read_workflow_run_log",
        "install_workflow_from_url",
        "uninstall_workflow",
        "skill_registry_browse",
        "skill_registry_search",
        "skill_registry_install",
        "skill_registry_sources",
        "skill_registry_uninstall",
        "skill_runtime_resolve_runtimes",
    ];
    // Flows has no clean tool-name prefix, so it MUST list every flow-owned
    // tool explicitly — a missing name falls through to `Platform` below and
    // stays callable under a custom `DomainSet { platform: true, flows: false }`,
    // leaking the flows surface past the runtime gate (#4808 review; #4797
    // maintainer review). Keep this in lockstep with the `#[cfg(feature =
    // "flows")]` registrations in `all_tools_with_runtime` above — the same 28
    // names asserted by `default_tools_omits_flows_tools_when_feature_off`.
    const FLOWS: &[&str] = &[
        "propose_workflow",
        "revise_workflow",
        "edit_workflow",
        "validate_workflow",
        "get_flow_history",
        "dry_run_workflow",
        "save_workflow",
        "suggest_workflows",
        "run_flow",
        "list_flow_runs",
        "resume_flow_run",
        "cancel_flow_run",
        "create_workflow",
        "duplicate_flow",
        "list_flows",
        "get_flow",
        "get_flow_run",
        "list_flow_connections",
        "search_tool_catalog",
        "get_tool_contract",
        "get_tool_output_sample",
        "list_agent_definitions",
        "list_connectable_toolkits",
        "list_node_kinds",
        "get_node_kind_contract",
        // Per-flow sandboxed memory (issue #5173) — `flow_` prefixed, not
        // `memory_`, so it does NOT fall under the `memory_` prefix check
        // below and must be listed here explicitly like every other
        // flow-owned tool.
        "flow_memory_recall",
        "flow_memory_remember",
    ];
    // Voice family agent tools (audio_toolkit) — no `voice_`/`tts_`/`stt_`
    // prefix, so they must be listed explicitly or they fall through to
    // Platform and stay callable when Voice is gated off (#4808 review).
    const VOICE: &[&str] = &[
        "audio_generate_podcast",
        "audio_email_podcast",
        "audio_generate_and_email_podcast",
    ];
    // Threads: thread_* / todo_* handled by prefix below; these are the extras.
    // Subconscious monitor + proactive-notify tools (Automation family).
    const MONITORS: &[&str] = &[
        "monitor",
        "monitor_list",
        "monitor_read",
        "monitor_stop",
        "notify_user",
    ];
    const THREADS_EXTRA: &[&str] = &["goal_get", "goal_set", "goal_complete"];
    // Memory extras not covered by the `memory_`/`goals_` prefixes. `goals`
    // has no trailing underscore since the four `goals_*` tools collapsed into
    // one `op`-dispatched tool, so it needs an entry here rather than a prefix.
    const MEMORY_EXTRA: &[&str] = &[
        "remember_preference",
        "save_preference",
        "update_memory_md",
        "tool_stats",
        "goals",
    ];

    // MCP: every MCP tool name is `mcp_` prefixed (mcp_registry_*,
    // mcp_call_tool, mcp_list_servers, mcp_list_tools).
    if name.starts_with("mcp_") {
        return DomainGroup::Mcp;
    }
    // Web3: wallet_/web3_/x402_ are all Web3-exclusive prefixes.
    if name.starts_with("wallet_") || name.starts_with("web3_") || name.starts_with("x402_") {
        return DomainGroup::Web3;
    }
    if SKILLS.contains(&name) {
        return DomainGroup::Skills;
    }
    if FLOWS.contains(&name) {
        return DomainGroup::Flows;
    }
    // Media generation: `media_` prefix (media_generate_image/video, media_list_models).
    if name.starts_with("media_") {
        return DomainGroup::Media;
    }
    // Voice family: explicit audio_* podcast tools plus the defensive
    // voice_/tts_/stt_ prefixes for any future tool.
    if VOICE.contains(&name)
        || name.starts_with("voice_")
        || name.starts_with("tts_")
        || name.starts_with("stt_")
    {
        return DomainGroup::Voice;
    }
    // Memory family (harness-kept): memory_* store/search/etc + goals_* + extras.
    //
    // The bare `memory` name is matched explicitly: the collapsed tool drops
    // the `memory_` prefix its members carry, so prefix matching alone would
    // land it in `Platform` and leave the whole memory surface callable under
    // a `DomainSet { platform: true, memory: false }`.
    if name == crate::memory::tools::MEMORY_TOOL_NAME
        || name.starts_with("memory_")
        || name.starts_with("goals_")
        || MEMORY_EXTRA.contains(&name)
    {
        return DomainGroup::Memory;
    }
    // Threads family (harness-kept): thread_* + per-thread goal + search.
    // `thread_` is kept as a prefix even though the `thread_*` agent-tool
    // family was removed: `goal_*` and the THREADS_EXTRA entries still
    // classify here, and a future threads tool should land in Threads rather
    // than falling through to Platform.
    if name.starts_with("thread_") || THREADS_EXTRA.contains(&name) {
        return DomainGroup::Threads;
    }
    // Harness families realigned out of Platform.
    if name.starts_with("artifact_")
        || name.starts_with("learning_")
        || name.contains("subagent")
        || matches!(
            name,
            "ask_user_clarification"
                | "agent_prepare_context"
                | "delegate"
                | "delegate_graph"
                | "delegate_to_personality"
                | "todo"
                | "wait"
                | "wait_loop"
                | "request_plan_review"
                | "plan_exit"
                | "spawn_parallel_agents"
        )
    {
        return DomainGroup::Agent;
    }
    if name.starts_with("config_") || name.starts_with("workspace_") {
        return DomainGroup::Config;
    }
    if name.starts_with("security_")
        || name.starts_with("credential_")
        || name.starts_with("session_")
        || name.starts_with("oauth_")
    {
        return DomainGroup::Security;
    }
    // ── Families carved out of Platform by the DomainGroup realignment ──────
    // Each of these previously fell through to Platform, which meant the tool
    // stayed callable when its family was gated off under a custom DomainSet —
    // leak the #4808 review flagged. Keep these in
    // lockstep with the `push(...)` tags in `core::all`.
    //
    // Automation: scheduled jobs (`cron_*`) plus the subconscious monitor +
    // proactive-notify surface.
    if name.starts_with("cron_") || name == "schedule" || MONITORS.contains(&name) {
        return DomainGroup::Automation;
    }
    // Integrations: every external connector reached on the user's behalf.
    if name.starts_with("composio")
        || name == "web_search_tool"
        || name.starts_with("tinyfish_")
        || name.starts_with("exa_")
        || name.starts_with("brave_")
        || name.starts_with("parallel_")
        || name.starts_with("querit_") || name.starts_with("tavily_")
        || name.starts_with("google_places_")
        || name.starts_with("stock_")
        || name.starts_with("storage_")
        || name.starts_with("task_source_")
        || name == "twilio_call"
        // Hosting: `hosting_` is a domain-exclusive prefix, so a NEW hosting
        // tool auto-gates rather than falling through to Platform and staying
        // callable under a custom DomainSet. `hosting_launch_site` uploads a
        // workspace directory to a third party and can provision a paid
        // database, so it must not outlive its family's gate.
        || name.starts_with("hosting_")
    {
        return DomainGroup::Integrations;
    }
    // Hosted: clients of the TinyHumans backend. The `billing_` / `team_` /
    // `referral_` prefixes were removed with those agent-tool families; their
    // controllers stay registered for the dashboard, which does not route
    // through this classifier.
    if name.starts_with("orchestration_") {
        return DomainGroup::Hosted;
    }
    // Desktop: shell-facing surfaces.
    if name.starts_with("dashboard_") || name.starts_with("desktop_") {
        return DomainGroup::Desktop;
    }
    // Runtimes: the managed Node/Python execution tools. These live under
    // `tools/impl/system/` rather than `runtime/`, so they are matched by name.
    if name == "node_exec" || name == "npm_exec" || name == "python_exec" {
        return DomainGroup::Runtimes;
    }
    // Inference: the CCR retrieval surface. Matched against the crate's own
    // constant list rather than a name prefix — the live tool is
    // `tinyjuice_retrieve`, and `tokenjuice_retrieve` / `retrieve_tool_output`
    // are migration aliases, so a prefix rule silently missed the real one.
    if crate::inference::tokenjuice::RECOVERY_TOOL_NAMES.contains(&name) {
        return DomainGroup::Inference;
    }
    // Everything else — shell/file and other kernel utilities — is Platform:
    // present under full(), absent under harness()/none().
    DomainGroup::Platform
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
