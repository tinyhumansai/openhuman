use super::*;

use crate::openhuman::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::openhuman::config::{Config, DelegateAgentConfig};
use crate::openhuman::javascript::NodeBootstrap;
use crate::openhuman::memory::Memory;
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
    workspace_dir: &std::path::Path,
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
        workspace_dir,
        agents,
        root_config,
    )
}

/// Create full tool registry including memory tools.
#[allow(clippy::implicit_hasher, clippy::too_many_arguments)]
pub fn all_tools_with_runtime(
    config: Arc<Config>,
    security: &Arc<SecurityPolicy>,
    runtime: Arc<dyn RuntimeAdapter>,
    audit: Arc<AuditLogger>,
    memory: Arc<dyn Memory>,
    browser_config: &crate::openhuman::config::BrowserConfig,
    http_config: &crate::openhuman::config::HttpRequestConfig,
    workspace_dir: &std::path::Path,
    agents: &HashMap<String, DelegateAgentConfig>,
    root_config: &crate::openhuman::config::Config,
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
            workspace_dir.to_path_buf(),
            reqwest::Client::new(),
        )))
    } else {
        tracing::debug!(
            "[tools::ops] node runtime disabled — shell PATH injection + node_exec/npm_exec suppressed"
        );
        None
    };

    let shell: Box<dyn Tool> = if let Some(bootstrap) = node_bootstrap.as_ref() {
        Box::new(ShellTool::with_node_bootstrap(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(&audit),
            Arc::clone(bootstrap),
        ))
    } else {
        Box::new(ShellTool::new(
            security.clone(),
            Arc::clone(&runtime),
            Arc::clone(&audit),
        ))
    };

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
        Box::new(SpawnParallelAgentsTool::new()),
        // Coding-harness control flow (issue #1205): a process-global
        // todo registry the agent can rewrite end-to-end, plus the
        // `plan_exit` marker that hands a plan-mode pass off to a
        // build-mode pass. The plan→build mode switch itself is a
        // follow-up; the tool emits a stable marker today.
        Box::new(TodoTool::new()),
        Box::new(PlanExitTool::new()),
        // Skill chaining: let an in-flight autonomous skill (e.g.
        // `github-issue-crusher`) kick off another bundled skill_run as a
        // fresh background job (e.g. `pr-review-shepherd` against the PR it
        // just opened) so each skill stays narrow + composable. Thin
        // wrapper over `skills::schemas::spawn_skill_run_background` — the
        // same helper `openhuman.skills_run` JSON-RPC uses, so RPC callers
        // and tool callers share one spawn path.
        Box::new(RunSkillTool::new()),
        Box::new(CurrentTimeTool::new()),
        Box::new(CodegraphIndexTool::new(
            config.clone(),
            workspace_dir.to_path_buf(),
        )),
        Box::new(CodegraphSearchTool::new(
            config.clone(),
            workspace_dir.to_path_buf(),
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
        Box::new(MemoryStoreTool::new(memory.clone(), security.clone())),
        Box::new(MemoryRecallTool::new(memory.clone())),
        Box::new(MemoryForgetTool::new(memory.clone(), security.clone())),
        Box::new(MemoryQueryTool),
        Box::new(MemoryQueryWalkTool),
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
            workspace_dir.to_path_buf(),
        )),
        Box::new(PushoverTool::new(
            security.clone(),
            workspace_dir.to_path_buf(),
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
    ];

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
        workspace_dir.to_path_buf(),
        root_config.curl.dest_subdir.clone(),
        root_config.curl.max_download_bytes,
        root_config.curl.timeout_secs,
    )));

    // gitbooks — answers questions about OpenHuman by calling the
    // GitBook MCP server. Two tools mirroring the upstream MCP tools.
    if root_config.gitbooks.enabled {
        tools.push(Box::new(GitbooksSearchTool::new(
            root_config.gitbooks.endpoint.clone(),
            root_config.gitbooks.timeout_secs,
        )));
        tools.push(Box::new(GitbooksGetPageTool::new(
            root_config.gitbooks.endpoint.clone(),
            root_config.gitbooks.timeout_secs,
        )));
        tracing::debug!("[gitbooks] registered gitbooks_search + gitbooks_get_page");
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
    let mcp_registry =
        Arc::new(crate::openhuman::mcp_client::McpServerRegistry::from_config(root_config));
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

    // ── Unified search engine ───────────────────────────────────────
    //
    // Exactly one engine drives `web_search_tool` plus any
    // engine-specific tools (Parallel research/extract/etc., Brave
    // news/image/video, Querit advanced filters). Mirrors the
    // LLM-provider API-key model: a single switch, BYO credentials,
    // layered tool surface.
    //
    // Legacy `seltz` / `searxng` config blocks are still parsed but
    // no longer register tools — they were superseded by this
    // selector. Use `search.engine = "managed" | "parallel" | "brave"
    // | "querit"` instead.
    {
        use crate::openhuman::config::SearchEngine;
        let search = &root_config.search;
        let max_results = search.max_results.clamp(1, 20);
        let timeout_secs = search.timeout_secs.max(1);
        match search.effective_engine() {
            SearchEngine::Managed => {
                tracing::debug!(
                    requested = %search.requested_engine_str(),
                    "[search] active engine = managed (backend-proxied web_search)"
                );
                tools.push(Box::new(WebSearchTool::new(
                    crate::openhuman::integrations::build_client(root_config),
                    max_results,
                    timeout_secs,
                )));
            }
            SearchEngine::Parallel => {
                tracing::debug!("[search] active engine = parallel (BYO direct API)");
                // Direct-mode Parallel still goes through the
                // backend-proxy client today; the BYO key is stored on
                // the integration client so the upstream tools can
                // pick it up once direct Parallel routing lands.
                let client = crate::openhuman::integrations::build_client(root_config);
                if let Some(client) = client {
                    tools.push(Box::new(
                        crate::openhuman::integrations::ParallelSearchTool::new(Arc::clone(
                            &client,
                        )),
                    ));
                    tools.push(Box::new(
                        crate::openhuman::integrations::ParallelExtractTool::new(Arc::clone(
                            &client,
                        )),
                    ));
                    tools.push(Box::new(
                        crate::openhuman::integrations::ParallelChatTool::new(Arc::clone(&client)),
                    ));
                    tools.push(Box::new(
                        crate::openhuman::integrations::ParallelResearchTool::new(Arc::clone(
                            &client,
                        )),
                    ));
                    tools.push(Box::new(
                        crate::openhuman::integrations::ParallelEnrichTool::new(Arc::clone(
                            &client,
                        )),
                    ));
                    tools.push(Box::new(
                        crate::openhuman::integrations::ParallelDatasetTool::new(Arc::clone(
                            &client,
                        )),
                    ));
                    // Layer the unified web_search slot too so the
                    // agent's default research path keeps working.
                    tools.push(Box::new(WebSearchTool::new(
                        Some(Arc::clone(&client)),
                        max_results,
                        timeout_secs,
                    )));
                } else {
                    tracing::warn!(
                        "[search] engine=parallel but no backend client — falling back to managed surface"
                    );
                    tools.push(Box::new(WebSearchTool::new(
                        None,
                        max_results,
                        timeout_secs,
                    )));
                }
            }
            SearchEngine::Brave => {
                tracing::debug!("[search] active engine = brave (BYO direct API)");
                let api_key = search.brave.api_key.clone();
                tools.push(Box::new(
                    crate::openhuman::integrations::BraveWebSearchTool::new(
                        api_key.clone(),
                        max_results,
                        timeout_secs,
                    ),
                ));
                tools.push(Box::new(
                    crate::openhuman::integrations::BraveNewsSearchTool::new(
                        api_key.clone(),
                        max_results,
                        timeout_secs,
                    ),
                ));
                tools.push(Box::new(
                    crate::openhuman::integrations::BraveImageSearchTool::new(
                        api_key.clone(),
                        max_results,
                        timeout_secs,
                    ),
                ));
                tools.push(Box::new(
                    crate::openhuman::integrations::BraveVideoSearchTool::new(
                        api_key,
                        max_results,
                        timeout_secs,
                    ),
                ));
            }
            SearchEngine::Querit => {
                tracing::debug!("[search] active engine = querit (BYO direct API)");
                tools.push(Box::new(
                    crate::openhuman::integrations::QueritSearchTool::new_web_search_tool(
                        search.querit.api_key.clone(),
                        None,
                        max_results,
                        timeout_secs,
                    ),
                ));
                tools.push(Box::new(
                    crate::openhuman::integrations::QueritSearchTool::new(
                        search.querit.api_key.clone(),
                        None,
                        max_results,
                        timeout_secs,
                    ),
                ));
            }
        }
    }

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
            tools.push(Box::new(
                crate::openhuman::integrations::ApifyRunActorTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ApifyGetRunStatusTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::ApifyGetRunResultsTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered apify tools");
        } else {
            tracing::debug!("[integrations] apify disabled — skipping");
        }
        if root_config.integrations.google_places.is_active() {
            tools.push(Box::new(
                crate::openhuman::integrations::GooglePlacesSearchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::GooglePlacesDetailsTool::new(Arc::clone(&client)),
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
        if root_config.integrations.tinyfish.is_active() {
            tools.push(Box::new(
                crate::openhuman::integrations::TinyFishSearchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::TinyFishFetchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::TinyFishAgentRunTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered tinyfish tools");
        } else {
            tracing::debug!("[integrations] tinyfish disabled — skipping");
        }
        if root_config.integrations.stock_prices.is_active() {
            tools.push(Box::new(
                crate::openhuman::integrations::StockQuoteTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockExchangeRateTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockOptionsTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockCryptoSeriesTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::openhuman::integrations::StockCommodityTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered stock_prices tools");
        } else {
            tracing::debug!("[integrations] stock_prices disabled — skipping");
        }
        if root_config.integrations.twilio.is_active() {
            tools.push(Box::new(
                crate::openhuman::integrations::TwilioCallTool::new(Arc::clone(&client)),
            ));
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
