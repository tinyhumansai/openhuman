use super::*;

use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::config::{Config, DelegateAgentConfig};
use crate::openhuman::javascript::NodeBootstrap;
use crate::openhuman::memory::Memory;
use crate::openhuman::runtime_python::PythonBootstrap;
use crate::openhuman::security::{AuditLogger, SecurityPolicy};
use std::collections::HashMap;
use std::sync::Arc;

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
    memory: Arc<dyn Memory>,
    browser_config: &crate::openhuman::config::BrowserConfig,
    http_config: &crate::openhuman::config::HttpRequestConfig,
    action_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    root_config: &crate::openhuman::config::Config,
) -> Vec<Box<dyn Tool>> {
    all_tools_with_runtime(
        config,
        security,
        Arc::new(NativeRuntime::new()),
        audit,
        memory,
        browser_config,
        http_config,
        action_dir,
        agents,
        root_config,
        None,
        None,
    )
}

/// Create full tool registry including memory tools.
///
/// `skill_allowlist` / `mcp_allowlist` scope the skill (workflow) and MCP-server
/// surfaces to an active agent profile's selection. `None` for either means
/// "all" (the default for every non-profile caller).
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools_with_runtime(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    audit: Arc<AuditLogger>,
    memory: Arc<dyn Memory>,
    browser_config: &crate::openhuman::config::BrowserConfig,
    http_config: &crate::openhuman::config::HttpRequestConfig,
    action_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    root_config: &crate::openhuman::config::Config,
    skill_allowlist: Option<&std::collections::HashSet<String>>,
    mcp_allowlist: Option<&[String]>,
) -> Vec<Box<dyn Tool>> {
    // Build a session-scoped managed Node.js bootstrap once, so ShellTool,
    // NodeExecTool, and NpmExecTool all share the same memoised resolution
    // state. Disabled when `node.enabled = false` — in that case shell skips
    // PATH injection and node/npm tools are not registered.
    let node_bootstrap: Option<Arc<NodeBootstrap>> = if root_config.node.enabled {
        tracing::debug!(
            version = %root_config.node.version,
            prefer_system = root_config.node.prefer_system,
            "[tools::ops] node runtime enabled — constructing shared NodeBootstrap"
        );
        Some(Arc::new(NodeBootstrap::new(
            root_config.node.clone(),
            action_dir.to_path_buf(),
            reqwest::Client::new(),
        )))
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
        Some(Arc::new(PythonBootstrap::new(
            root_config.runtime_python.clone(),
        )))
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

    let mut tools: Vec<Box<dyn Tool>> = vec![
        shell,
        Box::new(FileReadTool::new(security.clone())),
        Box::new(FileWriteTool::new(security.clone())),
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
        // `agent::harness::subagent_runner` for the dispatch path.
        Box::new(SpawnSubagentTool::new()),
        Box::new(SpawnAsyncSubagentTool::new()),
        // "Plan mode as a subagent": runs the read-only `context_scout`
        // inline and returns a bounded context bundle + recommended next
        // tool calls. Visible only to agents that allowlist it
        // (orchestrator / planner).
        Box::new(AgentPrepareContextTool::new()),
        // Steer/list/close reusable async sub-agents and collect results by
        // durable `subagent_session_id` (preferred) or transient `task_id`.
        Box::new(ListSubagentsTool::new()),
        Box::new(SteerSubagentTool::new()),
        Box::new(WaitSubagentTool::new()),
        Box::new(CloseSubagentTool::new()),
        Box::new(ContinueSubagentTool::new()),
        Box::new(SpawnParallelAgentsTool::new()),
        Box::new(DelegateToPersonalityTool::new()),
        // Coding-harness control flow (issue #1205): a process-global
        // todo registry the agent can rewrite end-to-end, plus the
        // `plan_exit` marker that hands a plan-mode pass off to a
        // build-mode pass. The plan→build mode switch itself is a
        // follow-up; the tool emits a stable marker today.
        Box::new(TodoTool::new()),
        // Move/update a specific task card by id on a target board (defaults to
        // the proactive `task-sources` board) — lets the agent advance the task
        // it's working (in_progress / done+evidence / blocked+reason) from any
        // thread, complementing `todo` which only touches the current thread.
        Box::new(UpdateTaskTool::new()),
        Box::new(PlanExitTool::new()),
        // Workflow composition: `run_workflow` runs another workflow as a
        // subagent and (by default) waits on its result like a function call;
        // `await_workflow` re-attaches to a run that outlived its inline wait.
        // Both wrap `skill_runtime::spawn_workflow_run_background` +
        // `await_run_outcome` — the same spawn path `openhuman.workflows_run`
        // JSON-RPC uses, so RPC and tool callers stay in sync.
        Box::new(RunWorkflowTool::new().with_skill_allowlist(skill_allowlist.cloned())),
        Box::new(AwaitWorkflowTool::new()),
        Box::new(CurrentTimeTool::new()),
        // Reversibility for native tool-output compaction (Stage 1a): when a
        // large result is compacted with a `retrieve_tool_output("<hash>")`
        // marker, this hands the original back from the CCR store on demand.
        Box::new(RetrieveToolOutputTool::new()),
        // Deterministic time-expression → timestamp resolver. `current_time`
        // only returns *now*, leaving the model to do epoch arithmetic by hand
        // (a real incident had an agent compute "24h ago" ~10 months off, then
        // fetch Slack history ascending from that wrong floor and miss the
        // latest messages). `resolve_time` does the conversion and returns the
        // value ready to paste into a tool argument.
        Box::new(ResolveTimeTool::new()),
        Box::new(LaunchAppTool::new()),
        Box::new(AxInteractTool::new(
            root_config.computer_control.ax_interact_mutations,
        )),
        // Multi-step UI automation in one call. Shares the ax_interact opt-in
        // (mutations) and sensitive-app denylist; runs a Rust perceive→act→verify
        // loop with a fast model so the chat model stays out of the click loop.
        Box::new(AutomateTool::new(
            root_config.computer_control.ax_interact_mutations,
        )),
        Box::new(CodegraphIndexTool::new(
            config.clone(),
            action_dir.to_path_buf(),
        )),
        Box::new(CodegraphSearchTool::new(
            config.clone(),
            action_dir.to_path_buf(),
        )),
        Box::new(DetectToolsTool::new()),
        Box::new(InstallToolTool::new(security.clone())),
        Box::new(CronAddTool::new(config.clone(), security.clone())),
        Box::new(CronListTool::new(config.clone())),
        Box::new(CronRemoveTool::new(config.clone())),
        Box::new(CronUpdateTool::new(config.clone(), security.clone())),
        Box::new(CronRunTool::new(config.clone())),
        Box::new(CronRunsTool::new(config.clone())),
        // Wallet tools — expose wallet operations to the agent tool-call pipeline
        // so the crypto sub-agent can prepare transfers, check status, etc.
        Box::new(WalletStatusTool::new()),
        Box::new(WalletChainStatusTool::new()),
        Box::new(WalletPrepareTransferTool::new()),
        Box::new(WalletTxStatusTool::new()),
        Box::new(WalletTxReceiptTool::new()),
        Box::new(WalletLookupTxTool::new()),
        Box::new(MemoryStoreTool::new(memory.clone(), security.clone())),
        Box::new(MemoryRecallTool::new(memory.clone())),
        Box::new(MemoryForgetTool::new(memory.clone(), security.clone())),
        // #002: read-only self-diagnosis of the memory pipeline so the agent
        // can explain an empty/stalled wiki + the fix.
        Box::new(MemoryDoctorTool::new(config.clone())),
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
        Box::new(RememberPreferenceTool::new(
            memory.clone(),
            security.clone(),
        )),
        // Two-lane explicit preferences (general → system prompt, situational →
        // per-query recall). Written verbatim to user_pref_{general,situational};
        // bypasses the inference/stability pipeline. Always registered.
        Box::new(SavePreferenceTool::new(memory.clone(), security.clone())),
        Box::new(MonitorTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(&audit),
        )),
        Box::new(MonitorListTool),
        Box::new(MonitorStopTool),
        Box::new(MonitorReadTool),
        // WhatsApp data store — read-only agent surface (issue #1341).
        // The matching `whatsapp_data_ingest` write-path stays internal-only
        // (registered in `src/core/all.rs::build_internal_only_controllers`)
        // and is intentionally NOT wrapped here.
        Box::new(WhatsAppDataListChatsTool),
        Box::new(WhatsAppDataListMessagesTool),
        Box::new(WhatsAppDataSearchMessagesTool),
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
        Box::new(AudioGeneratePodcastTool::new(
            config.clone(),
            security.clone(),
        )),
        Box::new(AudioEmailPodcastTool::new(config.clone(), security.clone())),
        Box::new(AudioGenerateAndEmailPodcastTool::new(
            config.clone(),
            security.clone(),
        )),
        Box::new(GmailUnsubscribeTool),
        // Knowledge & memory tools (agent-tool expansion). Read/bounded-write
        // ship default-ON; the overextending siblings (people_refresh_address_book —
        // bulk OS contacts ingest with a permission prompt) ship default-OFF via
        // `tools::user_filter`. (The vault domain was removed upstream in #3040.)
        Box::new(PeopleListTool),
        Box::new(PeopleResolveTool),
        Box::new(PeopleScoreTool),
        Box::new(PeopleGetTool),
        Box::new(PeopleAddAliasTool),
        Box::new(PeopleRecordInteractionTool),
        Box::new(PeopleRefreshAddressBookTool),
        // Skills metadata tools. `skill_run` is already exposed by RunSkillTool
        // above, so it is not duplicated. Reads ship default-ON; the
        // create/install/uninstall mutators ship default-OFF via
        // `tools::user_filter` (install also fetches remote content).
        Box::new(
            WorkflowListTool::new(config.clone()).with_skill_allowlist(skill_allowlist.cloned()),
        ),
        Box::new(
            WorkflowDescribeTool::new(config.clone())
                .with_skill_allowlist(skill_allowlist.cloned()),
        ),
        // Skill registry tools — browse/search/install from remote registries.
        // Browse and search are read-only (default-ON); install is a write
        // operation (fetches remote content and writes to disk).
        Box::new(SkillRegistryBrowseTool),
        Box::new(SkillRegistrySearchTool),
        Box::new(SkillRegistryInstallTool::new(config.clone())),
        Box::new(SkillRegistrySourcesTool),
        Box::new(SkillRegistryUninstallTool),
        // Skill runtime probes — resolve the reusable Node/Python runtimes
        // that skill execution relies on before a script-backed skill runs.
        Box::new(SkillRuntimeResolveRuntimesTool::new(config.clone())),
        Box::new(
            WorkflowReadResourceTool::new(config.clone())
                .with_skill_allowlist(skill_allowlist.cloned()),
        ),
        Box::new(WorkflowRecentRunsTool::new(config.clone())),
        Box::new(WorkflowReadRunLogTool::new(config.clone())),
        Box::new(WorkflowCreateTool::new(config.clone())),
        Box::new(WorkflowInstallFromUrlTool::new(config.clone())),
        Box::new(WorkflowUninstallTool),
        // Threads (conversation) tools. Read/bounded-write ship default-ON;
        // the destructive thread_delete / thread_purge_all ship default-OFF
        // via `tools::user_filter` (thread_destructive toggle).
        Box::new(ThreadListTool),
        Box::new(ThreadReadTool),
        Box::new(ThreadCreateTool),
        Box::new(ThreadUpdateTitleTool),
        Box::new(ThreadUpdateLabelsTool),
        Box::new(ThreadMessageListTool),
        // Read-only cross-thread transcript search (trigram index). Lets the
        // context scout and other agents recall what was said in earlier chats.
        Box::new(ThreadTranscriptSearchTool),
        Box::new(ThreadMessageAppendTool),
        Box::new(ThreadMessageUpdateTool),
        Box::new(ThreadTitleGenerateTool),
        Box::new(ThreadTurnStateGetTool),
        Box::new(ThreadTurnStateListTool),
        Box::new(ThreadTurnStateClearTool),
        Box::new(ThreadTaskBoardReadTool::new(config.clone())),
        Box::new(ThreadTaskBoardWriteTool::new(config.clone())),
        Box::new(ThreadDeleteTool),
        Box::new(ThreadPurgeAllTool),
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
        // destructive/overextending siblings (artifact_delete, todo_remove/
        // replace/clear, task_source_add/update/remove) are registered too
        // but ship default-OFF via `tools::user_filter` (their toggle IDs
        // default off in onboarding). The per-call permission ladder still
        // gates them.
        Box::new(ArtifactListTool::new(config.clone())),
        Box::new(ArtifactGetTool::new(config.clone())),
        Box::new(ArtifactDeleteTool::new(config.clone())),
        Box::new(TodoListTool::new(config.clone())),
        Box::new(TodoAddTool::new(config.clone())),
        Box::new(TodoEditTool::new(config.clone())),
        Box::new(TodoUpdateStatusTool::new(config.clone())),
        Box::new(TodoDecidePlanTool::new(config.clone())),
        Box::new(TodoRemoveTool::new(config.clone())),
        Box::new(TodoReplaceTool::new(config.clone())),
        Box::new(TodoClearTool::new(config.clone())),
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
        // Account & money. Reads default-ON; billing money-movers (billing_writes)
        // and team admin ops (team_admin) ship default-OFF via `tools::user_filter`.
        // credentials exposes only non-secret reads.
        Box::new(ReferralStatsTool::new(config.clone())),
        Box::new(ReferralClaimTool::new(config.clone())),
        Box::new(BillingPlanTool::new(config.clone())),
        Box::new(BillingBalanceTool::new(config.clone())),
        Box::new(BillingTransactionsTool::new(config.clone())),
        Box::new(BillingAutoRechargeTool::new(config.clone())),
        Box::new(BillingCardsTool::new(config.clone())),
        Box::new(BillingCouponsTool::new(config.clone())),
        Box::new(BillingPortalTool::new(config.clone())),
        Box::new(BillingPurchasePlanTool::new(config.clone())),
        Box::new(BillingTopUpTool::new(config.clone())),
        Box::new(BillingCoinbaseChargeTool::new(config.clone())),
        Box::new(BillingSetupIntentTool::new(config.clone())),
        Box::new(BillingUpdateCardTool::new(config.clone())),
        Box::new(BillingDeleteCardTool::new(config.clone())),
        Box::new(BillingRedeemCouponTool::new(config.clone())),
        Box::new(BillingUpdateAutoRechargeTool::new(config.clone())),
        Box::new(TeamListTool::new(config.clone())),
        Box::new(TeamUsageTool::new(config.clone())),
        Box::new(TeamGetTool::new(config.clone())),
        Box::new(TeamListMembersTool::new(config.clone())),
        Box::new(TeamListInvitesTool::new(config.clone())),
        Box::new(TeamCreateTool::new(config.clone())),
        Box::new(TeamUpdateTool::new(config.clone())),
        Box::new(TeamDeleteTool::new(config.clone())),
        Box::new(TeamSwitchTool::new(config.clone())),
        Box::new(TeamJoinTool::new(config.clone())),
        Box::new(TeamLeaveTool::new(config.clone())),
        Box::new(TeamCreateInviteTool::new(config.clone())),
        Box::new(TeamRevokeInviteTool::new(config.clone())),
        Box::new(TeamRemoveMemberTool::new(config.clone())),
        Box::new(TeamChangeMemberRoleTool::new(config.clone())),
        Box::new(CredentialListTool::new(config.clone())),
        Box::new(SessionStateTool::new(config.clone())),
        Box::new(SessionGetUserTool::new(config.clone())),
        Box::new(OAuthConnectUrlTool::new(config.clone())),
        Box::new(OAuthListTool::new(config.clone())),
        // Desktop perception, MCP registry, workspace persona. Observe/connect/
        // call tools default-ON; OS permission prompts (screen_permissions),
        // MCP install/uninstall (mcp_manage), and persona/workspace writers
        // (workspace_manage) ship default-OFF via `tools::user_filter`.
        Box::new(ScreenStatusTool),
        Box::new(ScreenCaptureImageRefTool),
        Box::new(ScreenVisionRecentTool),
        Box::new(ScreenVisionFlushTool),
        Box::new(ScreenRefreshPermissionsTool),
        Box::new(ScreenCaptureNowTool),
        Box::new(ScreenCaptureTestTool),
        Box::new(ScreenSessionStartTool),
        Box::new(ScreenSessionStopTool),
        Box::new(ScreenInputActionTool),
        Box::new(ScreenGlobeStartTool),
        Box::new(ScreenGlobePollTool),
        Box::new(ScreenGlobeStopTool),
        Box::new(ScreenRequestPermissionsTool),
        Box::new(ScreenRequestPermissionTool),
        Box::new(McpRegistrySearchTool::new(config.clone())),
        Box::new(McpRegistryGetTool::new(config.clone())),
        Box::new(McpRegistryInstalledListTool::new(config.clone())),
        Box::new(McpRegistryStatusTool::new(config.clone())),
        Box::new(McpRegistryListToolsTool),
        Box::new(McpRegistryConnectTool::new(config.clone())),
        Box::new(McpRegistryDisconnectTool),
        Box::new(McpRegistryToolCallTool),
        Box::new(McpRegistryConfigAssistTool::new(config.clone())),
        Box::new(McpRegistryInstallTool::new(config.clone())),
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

    // Subconscious scratchpad tools — persistent working memory across ticks.
    tools.extend(crate::openhuman::subconscious::scratchpad::tools::all_scratchpad_tools());

    // Subconscious user-facing handoff — notify_user proactive delivery.
    tools.extend(crate::openhuman::subconscious::user_thread::all_user_thread_tools());

    // tiny.place agent surface. These wrap the internal tiny.place controllers
    // so the dedicated tinyplace subagent can register identities, inspect
    // inbox/DM state, trade marketplace assets, manage groups, and work jobs
    // through the same validation/client paths as JSON-RPC.
    let tinyplace_tools = crate::openhuman::tinyplace::tools::all_tinyplace_agent_tools();
    log::debug!(
        "[tools::ops][tinyplace] registering tinyplace agent tools count={}",
        tinyplace_tools.len()
    );
    tools.extend(tinyplace_tools);

    // Presentation generation (#2778). Native-Rust engine (ppt-rs
    // backed) as of the #2780-follow-up rust-engine refactor — no
    // managed Python venv, no first-call install latency. Always
    // registered.
    tools.push(Box::new(PresentationTool::new(
        root_config.workspace_dir.clone(),
        security.clone(),
    )));

    // Long-term goals list tools. Used primarily by the background
    // `goals_agent` (which filters to these via its `[tools] named`
    // allowlist); also available to the main agent for explicit edits.
    {
        let goals_dir = root_config.workspace_dir.clone();
        tools.push(Box::new(
            crate::openhuman::memory_goals::GoalsListTool::new(goals_dir.clone()),
        ));
        tools.push(Box::new(crate::openhuman::memory_goals::GoalsAddTool::new(
            goals_dir.clone(),
        )));
        tools.push(Box::new(
            crate::openhuman::memory_goals::GoalsEditTool::new(goals_dir.clone()),
        ));
        tools.push(Box::new(
            crate::openhuman::memory_goals::GoalsDeleteTool::new(goals_dir),
        ));
    }

    // Thread-level goal tools (Codex-style per-thread completion contract).
    // Visible only to agents that allowlist them (orchestrator). The target
    // thread is resolved from the ambient `thread_id`, so no thread arg is
    // taken. `goal_get`/`goal_set`/`goal_complete` — pause/resume/budget are
    // system-driven and have no model tool.
    {
        let goal_dir = root_config.workspace_dir.clone();
        tools.push(Box::new(crate::openhuman::thread_goals::GoalGetTool::new(
            goal_dir.clone(),
        )));
        tools.push(Box::new(crate::openhuman::thread_goals::GoalSetTool::new(
            goal_dir.clone(),
        )));
        tools.push(Box::new(
            crate::openhuman::thread_goals::GoalCompleteTool::new(goal_dir),
        ));
    }

    if browser_config.enabled {
        // Unified web-access allowlist (merge fetch + browser firewalls): the
        // browser tool shares the single `http_request.allowed_domains` host
        // list rather than the now-deprecated `[browser].allowed_domains`. See
        // `browser_allowed_domains` for why the `"*"` wildcard is stripped.
        let browser_allowed_domains = browser_allowed_domains(&http_config.allowed_domains);
        // Add legacy browser_open tool for simple URL opening
        tools.push(Box::new(BrowserOpenTool::new(
            security.clone(),
            browser_allowed_domains.clone(),
        )));
        // Add full browser automation tool (pluggable backend)
        tools.push(Box::new(BrowserTool::new_with_backend(
            security.clone(),
            browser_allowed_domains.clone(),
            browser_config.session_name.clone(),
            browser_config.backend.clone(),
            browser_config.native_headless,
            browser_config.native_webdriver_url.clone(),
            browser_config.native_chrome_path.clone(),
            ComputerUseConfig {
                endpoint: browser_config.computer_use.endpoint.clone(),
                api_key: None,
                timeout_ms: browser_config.computer_use.timeout_ms,
                allow_remote_endpoint: browser_config.computer_use.allow_remote_endpoint,
                window_allowlist: browser_config.computer_use.window_allowlist.clone(),
                max_coordinate_x: browser_config.computer_use.max_coordinate_x,
                max_coordinate_y: browser_config.computer_use.max_coordinate_y,
            },
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
    // or SPL payment signing, and ledger recording.
    tools.push(Box::new(
        crate::openhuman::x402::tools::X402RequestTool::new(),
    ));

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
    // Gitbooks is modelled as a legacy MCP server (`McpServerRegistry`), so it
    // honours the same per-profile `mcp_allowlist`: a profile that scopes its
    // MCP servers and omits "gitbooks" must not see this surface either.
    let gitbooks_allowed = mcp_allowlist.is_none_or(|allowed| {
        allowed
            .iter()
            .any(|name| name.eq_ignore_ascii_case("gitbooks"))
    });
    if root_config.gitbooks.enabled && gitbooks_allowed {
        tools.push(Box::new(GitbooksSearchTool::new(
            root_config.gitbooks.endpoint.clone(),
            root_config.gitbooks.timeout_secs,
        )));
        tools.push(Box::new(GitbooksGetPageTool::new(
            root_config.gitbooks.endpoint.clone(),
            root_config.gitbooks.timeout_secs,
        )));
        tracing::debug!("[gitbooks] registered gitbooks_search + gitbooks_get_page");
    } else if root_config.gitbooks.enabled {
        tracing::debug!("[profiles] gitbooks tools suppressed by profile mcp allowlist");
    }

    // MCP setup-agent tool surface (search/get/request_secret/test/install).
    // Registered unconditionally — the `mcp_setup` sub-agent filters to just
    // these via its `[tools] named = [...]` allowlist, and the host agent's
    // own tool list is wide enough that the extra five entries are negligible.
    {
        let cfg = Arc::new(root_config.clone());
        tools.push(Box::new(McpSetupSearchTool::new(Arc::clone(&cfg))));
        tools.push(Box::new(McpSetupGetTool::new(Arc::clone(&cfg))));
        tools.push(Box::new(McpSetupRequestSecretTool::new()));
        tools.push(Box::new(McpSetupTestConnectionTool::new(Arc::clone(&cfg))));
        tools.push(Box::new(McpSetupInstallAndConnectTool::new(cfg)));
        tracing::debug!("[mcp_setup] registered 5 setup-agent tools");
    }

    // Generic remote MCP bridge tools. These let the agent enumerate
    // named MCP servers and forward `tools/call` through the core
    // instead of hardcoding one bespoke MCP integration per server.
    let mcp_registry = {
        let base = crate::openhuman::mcp_client::McpServerRegistry::from_config(root_config);
        // Scope the MCP surface to the active profile's allowlist. `None` keeps
        // every configured server; `Some(&[])` yields an empty registry.
        match mcp_allowlist {
            Some(allowed) => Arc::new(base.retaining_servers(allowed)),
            None => Arc::new(base),
        }
    };
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

    tools.extend(crate::openhuman::search::build_search_tools(root_config));

    // High-level web3 tools (swaps / bridges / dapp calls) built on the wallet.
    // They call the backend deBridge proxy per-invocation and error gracefully
    // when the user is not signed in, so they register unconditionally.
    tools.extend(crate::openhuman::web3::all_web3_agent_tools());

    // Managed Node.js exec tools — gated on `root_config.node.enabled`.
    // Both share the same `NodeBootstrap` as ShellTool so the download +
    // extract + install pipeline runs at most once per session.
    if let Some(bootstrap) = node_bootstrap.as_ref() {
        tools.push(Box::new(NodeExecTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
        )));
        tools.push(Box::new(NpmExecTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(bootstrap),
        )));
        tracing::debug!("[tools::ops] registered node_exec + npm_exec");
    }

    // Vision tools are always available
    tools.push(Box::new(ScreenshotTool::new(security.clone())));
    tools.push(Box::new(ImageInfoTool::new(security.clone())));

    // Native mouse + keyboard control (disabled by default)
    if root_config.computer_control.enabled {
        tools.push(Box::new(MouseTool::new(security.clone())));
        tools.push(Box::new(KeyboardTool::new(security.clone())));
        tracing::debug!("[computer] mouse and keyboard tools registered");
    }

    // Tool effectiveness stats (enabled when learning is on)
    tracing::debug!(
        learning_enabled = root_config.learning.enabled,
        tool_tracking_enabled = root_config.learning.tool_tracking_enabled,
        "evaluating ToolStatsTool registration"
    );
    if root_config.learning.enabled && root_config.learning.tool_tracking_enabled {
        tools.push(Box::new(ToolStatsTool::new(memory.clone())));
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
            crate::openhuman::inference::provider::ProviderRuntimeOptions {
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
    if let Some(client) = crate::openhuman::integrations::build_client(root_config) {
        tracing::debug!("[integrations] client built successfully");
        if root_config.integrations.apify.is_active() {
            tools.push(Box::new(crate::openhuman::tools::ApifyRunActorTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(
                crate::openhuman::tools::ApifyGetRunStatusTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::tools::ApifyGetRunResultsTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered apify tools");
        } else {
            tracing::debug!("[integrations] apify disabled — skipping");
        }
        if root_config.integrations.google_places.is_active() {
            tools.push(Box::new(
                crate::openhuman::tools::GooglePlacesSearchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::tools::GooglePlacesDetailsTool::new(Arc::clone(&client)),
            ));
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
            tools.push(Box::new(crate::openhuman::tools::StockQuoteTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(
                crate::openhuman::tools::StockExchangeRateTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(crate::openhuman::tools::StockOptionsTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(
                crate::openhuman::tools::StockCryptoSeriesTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(crate::openhuman::tools::StockCommodityTool::new(
                Arc::clone(&client),
            )));
            tracing::debug!("[integrations] registered stock_prices tools");
        } else {
            tracing::debug!("[integrations] stock_prices disabled — skipping");
        }
        if root_config.integrations.twilio.is_active() {
            tools.push(Box::new(crate::openhuman::tools::TwilioCallTool::new(
                Arc::clone(&client),
            )));
            tracing::debug!("[integrations] registered twilio tools");
        } else {
            tracing::debug!("[integrations] twilio disabled — skipping");
        }

        // Composio — backend-proxied 1000+ OAuth integrations. Registers
        // five agent tools (list_toolkits, list_connections, authorize,
        // list_tools, execute) when the composio toggle is on. See
        // `src/openhuman/composio/tools.rs` for per-tool details.
        let composio_tools = crate::openhuman::composio::all_composio_agent_tools(root_config);
        if !composio_tools.is_empty() {
            tracing::debug!(
                count = composio_tools.len(),
                "[integrations] registered composio tools"
            );
            tools.extend(composio_tools);
        } else {
            tracing::debug!("[integrations] composio disabled — skipping");
        }
    } else {
        tracing::debug!(
            "[integrations] build_client returned None — integration tools not registered"
        );
    }

    if root_config.integrations.polymarket.enabled {
        tools.push(Box::new(PolymarketTool::new(
            &root_config.integrations.polymarket,
            security.clone(),
        )));
        tracing::debug!("[integrations] registered polymarket tool (read + trading)");
    } else {
        tracing::debug!("[integrations] polymarket disabled — skipping");
    }

    // Coding-harness `lsp` tool (issue #1205) — capability-gated by the
    // OPENHUMAN_LSP_ENABLED env var. The backend (real language-server
    // bridge) is a follow-up; today the gate just controls visibility
    // so agents don't see a method that always errors.
    if crate::openhuman::tools::implementations::lsp_capability_enabled() {
        tools.push(Box::new(
            crate::openhuman::tools::implementations::LspTool::new(),
        ));
        tracing::debug!("[lsp] capability gate on — LspTool registered");
    } else {
        tracing::debug!("[lsp] capability gate off (set OPENHUMAN_LSP_ENABLED=1 to register)");
    }

    tools
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
