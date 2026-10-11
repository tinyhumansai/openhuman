use super::*;
use crate::agent::host_runtime::{NativeRuntime, RuntimeAdapter};
use crate::agent::tools::DelegateTool;
use crate::config::{Config, DelegateAgentConfig};
use crate::security::{AuditLogger, SecurityPolicy};
use std::collections::HashMap;
use std::sync::Arc;
use tinyagents_harness::tools::{CurrentTimeTool, ResolveTimeTool};
#[cfg(feature = "mcp")]
use tinymcp::tools::{McpListServersTool, McpListToolsTool};
use tinytools::Tool;
use tinytools_std::detect_tools::DetectToolsTool;
use tinytools_std::filesystem::{
    ApplyPatchTool, CsvExportTool, EditFileTool, FileReadTool, FileWriteTool, GitOperationsTool,
    GlobTool, GrepTool, ImageInfoTool, ListFilesTool, ReadDiffTool, RunLinterTool, RunTestsTool,
};
use tinytools_std::network::{CurlTool, PushoverTool};

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
    let config = Arc::new(Config::default());
    vec![
        Box::new(ShellTool::with_config(
            security.clone(),
            runtime,
            audit,
            config,
        )),
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
    let shell: Box<dyn Tool> = Box::new(ShellTool::with_config(
        security.clone(),
        Arc::clone(&runtime),
        Arc::clone(&audit),
        Arc::clone(&config),
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
    ];
    tools.extend(crate::agent::registration::dialogue_tools(
        action_dir,
        root_config,
    ));
    tools.append(&mut vec![
        Box::new(CurrentTimeTool::new()),
        // Reversibility for native tool-output compaction (Stage 1a): when a
        // large result is compacted with a `retrieve_tool_output("<hash>")`
        // marker, this hands the original back from the CCR store on demand.
        Box::new(retrieve_tool_output_tool()),
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
    ]);
    tools.extend(crate::cron::tools::tools(config.clone(), security));
    #[cfg(feature = "flows")]
    tools.extend(crate::flows::registration::tools(config.clone(), security));
    #[cfg(feature = "web3")]
    tools.extend(crate::web3::wallet::registration::tools());
    tools.append(&mut vec![
        Box::new(ScheduleTool::new(security.clone(), root_config.clone())),
        Box::new(ProxyConfigTool::new(config.clone(), security.clone())),
        Box::new(UpdateCheckTool::new()),
        Box::new(UpdateApplyTool::new(security.clone())),
        Box::new(GitOperationsTool::new(
            security.clone(),
            action_dir.to_path_buf(),
        )),
        // Review loop for skill `coding` (and the workflow-run `critic`):
        // diff, lint and test the working tree in the action sandbox. They
        // were defined but never registered, so the belts naming them held
        // nothing. `Deferred`, so they cost no schema until found.
        Box::new(ReadDiffTool::new(action_dir.to_path_buf())),
        Box::new(RunLinterTool::new(action_dir.to_path_buf())),
        Box::new(RunTestsTool::new(action_dir.to_path_buf())),
        Box::new(PushoverTool::new(
            security.clone(),
            action_dir.to_path_buf(),
        )),
    ]);
    tools.push(Box::new(GmailUnsubscribeTool));
    #[cfg(feature = "skills")]
    tools.extend(crate::skills::registration::tools(config.clone()));
    tools.append(&mut vec![
        // Task & productivity tools (issue: agent-tool expansion).
        // Read/observe + bounded-write tools are registered here; the
        // destructive/overextending siblings (artifact_delete,
        // task_source_add/update/remove) are registered too but ship
        // default-OFF via `tools::user_filter` (their toggle IDs default off
        // in onboarding). The per-call permission ladder still gates them.
    ]);
    tools.extend(crate::agent::artifacts::tools::registered_tools(
        config.clone(),
    ));
    tools.extend(crate::integrations::task_sources::tools::registered_tools(
        config.clone(),
    ));

    // System & self-management: observability (default-ON) + service
    // lifecycle. doctor/health/cost/dashboard/security reads are default-ON.
    // service_status / daemon_host_prefs_get default-ON; the lifecycle
    // mutators ship default-OFF via `tools::user_filter` (service_lifecycle).
    tools.extend(crate::platform::doctor::tools::registered_tools(
        config.clone(),
    ));
    tools.extend(crate::platform::health::tools::registered_tools());
    tools.extend(crate::platform::cost::tools::registered_tools(
        config.clone(),
    ));
    tools.extend(crate::desktop::dashboard::tools::registered_tools(
        config.clone(),
    ));
    #[cfg(feature = "modules")]
    tools.extend(crate::desktop::control::tools::registered_tools(
        config.clone(),
    ));
    tools.extend(crate::security::tools::registered_tools(config.clone()));
    tools.extend(crate::platform::service::tools::registered_tools(
        config.clone(),
    ));

    // Config: read-only surface (default-ON). The config_update_* mutators
    // are deferred (their apply fns take non-Deserialize patch structs);
    // see config/tools.rs.
    tools.extend(crate::config::tools::registered_tools(config.clone()));
    tools.append(&mut vec![
        // Account & money. The billing / team / referral agent-tool families were
        // removed: money movement and team administration are dashboard
        // surfaces, not things an agent should reach for mid-turn, and their
        // controllers remain registered for the UI. `credentials` exposes only
        // non-secret reads.
    ]);
    tools.extend(crate::security::credentials::tools::registered_tools(
        config.clone(),
    ));
    tools.append(&mut vec![
        // MCP registry and workspace persona. Observe/connect/call tools
        // default-ON; MCP uninstall (mcp_manage), and persona/workspace writers
        // (workspace_manage) ship default-OFF via `tools::user_filter`. There
        // is no install tool: servers are declared by the user in mcp.json.
        //
        // MCP registry (dynamic, user-installed servers) — compiled out with
        // the `mcp` feature. Per-element attrs inside the `vec![]` mirror the
        // voice idiom used earlier in this same literal.
    ]);
    #[cfg(feature = "mcp")]
    tools.extend(crate::mcp::registry::tools::registered_tools(
        config.clone(),
    ));
    tools.extend(crate::config::workspace::tools::registered_tools(
        config.clone(),
    ));

    // The single `memory` tool (recall | fetch | learn | forget), registered
    // only while memory is on: with no usable engine (signed out, no CortexDB
    // key) the model is not offered a tool that can only fail.
    if crate::memory::engine::is_on(root_config) {
        tools.push(Box::new(crate::memory::MemoryTool::new(config.clone())));
        tracing::debug!("[tools::ops] registered memory tool");
    } else {
        tracing::debug!("[tools::ops] memory off; memory tool not registered");
    }

    // `juice_find` / `juice_extract` / `juice_summarize`: only while a handle can name them.
    tools.extend(crate::inference::tokenjuice::repl_tools_for(root_config));

    // Presentation generation (#2778), backed by the native Rust engine.
    // Always registered.
    #[cfg(feature = "documents")]
    tools.push(Box::new(PresentationTool::for_config(
        root_config,
        security.clone(),
    )));

    // Document generation (#4847, Problem 3). Native-Rust engine
    // (docx-rs backed) — no managed runtime, no subprocess — emitting a
    // real `.docx` through the same byte-agnostic artifact pipeline as
    // the presentation tool. Always registered; same constructor shape.
    #[cfg(feature = "documents")]
    tools.push(Box::new(DocumentTool::for_config(
        root_config,
        security.clone(),
    )));

    // Thread-level goal tools (Codex-style per-thread completion contract).
    // Visible only to agents that allowlist them (orchestrator). The target
    // thread is resolved from the ambient `thread_id`, so no thread arg is
    // taken. `goal_get`/`goal_set`/`goal_complete` — pause/resume/budget are
    // system-driven and have no model tool.
    tools.extend(crate::agent::goals::goal_tools(&root_config.workspace_dir));

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
    tools.push(Box::new(http_request_tool(
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
    tools.push(Box::new(crate::web3::x402::request_tool(
        security.clone(),
        http_config.allowed_domains.clone(),
    )));

    // Coding-harness baseline `web_fetch` (issue #1205) — single-purpose
    // GET-and-read primitive that reuses the same allowed-domains gate
    // as `http_request`. Use this for docs/READMEs; reach for
    // `http_request` only when you need richer HTTP semantics.
    tools.push(Box::new(web_fetch_tool(
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
            tools.push(Box::new(mcp_call_tool(
                Arc::clone(&mcp_registry),
                security.clone(),
            )));
            tracing::debug!(
                count = mcp_registry.list().len(),
                "[mcp_client] registered generic MCP bridge tools"
            );
            // And every cached server tool as its own `mcp_<server>_<tool>`,
            // deferred unless the server asks for direct exposure. Names
            // already taken keep their owner.
            let reserved: std::collections::HashSet<String> =
                tools.iter().map(|tool| tool.name().to_string()).collect();
            tools.extend(
                crate::tools::implementations::network::configured_server_tools(
                    root_config,
                    &mcp_registry,
                    security,
                    &reserved,
                ),
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

    // Hosting tools — deploy a workspace directory through the compiled
    // TinyHosts module. Registered only once a credential
    // actually resolves (`[hosting].api_key`, else the provider's environment
    // variables). Module loading stays lazy until a tool is invoked.
    // A tool that cannot work is worse than one that is absent,
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

    // Image metadata is always available for user-provided images.
    tools.push(Box::new(ImageInfoTool::new(security.clone())));

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
            tools.push(Box::new(
                crate::integrations::tools::GooglePlacesSearchTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::integrations::tools::GooglePlacesDetailsTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered google_places tools");
        } else {
            tracing::debug!("[integrations] google_places disabled — skipping");
        }
        // Web search providers (Exa, Gemini, TinyFish, ...) register through
        // the TinySearch module above, so `[search] enabled = false`
        // suppresses them too. `integrations.parallel` is parsed for old
        // config files and no longer does anything.
        if root_config.integrations.stock_prices.is_active() {
            tools.push(Box::new(crate::integrations::tools::StockQuoteTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(
                crate::integrations::tools::StockExchangeRateTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(crate::integrations::tools::StockOptionsTool::new(
                Arc::clone(&client),
            )));
            tools.push(Box::new(
                crate::integrations::tools::StockCryptoSeriesTool::new(Arc::clone(&client)),
            ));
            tools.push(Box::new(
                crate::integrations::tools::StockCommodityTool::new(Arc::clone(&client)),
            ));
            tracing::debug!("[integrations] registered stock_prices tools");
        } else {
            tracing::debug!("[integrations] stock_prices disabled — skipping");
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

    // Post-filters over the assembled list:
    //
    // 1. DomainSet (#4796): drop tools whose DomainGroup is disabled under the
    //    ambient CoreContext; no context, or `DomainSet::full()`, keeps every
    //    tool. Under `harness()` only the memory + threads families survive
    //    (see `tool_group` and its Platform-default caveat). In SaaS,
    //    `profiles::tools::admits` also applies the operator's host groups.
    let before = tools.len();
    let domains = crate::core::runtime::context::CoreContext::current().map(|c| c.domains());
    let mut tools: Vec<Box<dyn Tool>> = if let Some(set) = domains {
        tools
            .into_iter()
            .filter(|t| crate::profiles::tools::admits(t.name(), set.allows(tool_group(t.name()))))
            .collect()
    } else {
        // No ambient context (unit tests / pre-boot) ⇒ no domain filtering.
        tools
    };
    let after_domains = tools.len();

    // 2. ToolGroups: a group an embedder set to `Off` is not registered at all.
    //    `Advertised` and `Withheld` both keep the tool here — they differ only
    //    in whether its schema reaches the provider, which is decided later by
    //    `strip_packed_from_visible`. Same default-open rule as the filter
    //    above: with no ambient context every group is `Withheld`, so nothing
    //    is dropped and the desktop list is unchanged.
    {
        let groups = crate::tools::toolpacks::groups::current();
        tools.retain(|t| groups.mode_for_tool(t.name()) != crate::tools::toolpacks::GroupMode::Off);
    }

    log::debug!(
        "[tools::ops][post-filter] {before} assembled → {after_domains} after DomainSet → \
         {} after ToolGroups",
        tools.len()
    );

    // Append the two always-on pack tools. They resolve packed tools by name
    // out of the agent's live registry (bound after `Arc::new`), so they also
    // cover the `delegate_*` tools synthesised later by
    // `orchestrator_tools::collect_orchestrator_tools` — which never pass
    // through this function.
    crate::tools::toolpacks::append_pack_tools(&mut tools);
    // The lookup half of `ToolExposure::Deferred` is not registered here: the
    // tinyagents harness advertises its intrinsic `tool_search` bridge
    // (a found tool is then called by its own name) whenever a run has a
    // deferred tool (`tool::discover`), ranked by whatever `agent::tinyagents::discovery` installed. A host-registered
    // `tool_search` would shadow that bridge.
    tools
}

#[path = "ops_tool_groups.rs"]
mod tool_groups;
pub(crate) use tool_groups::tool_group;

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
