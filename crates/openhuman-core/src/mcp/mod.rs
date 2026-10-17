//! MCP family — the host half of Model Context Protocol support.
//!
//! MCP protocol and client behavior run in the TinyMCP module. This family is
//! the OpenHuman host adapter for configuration, product policy, lifecycle,
//! events, audit RPCs, and the local MCP server entry points.
//!
//! What is here is what belongs to this application.
//!
//! # Members
//!
//! - [`host`] — conversion from this application's configuration into the
//!   TinyMCP bus contract and host-owned proxy policy.
//! - [`registry`] — the `mcp_clients` RPC surface, the
//!   agent-facing tools, and the prompt-injection scan over remote tool
//!   definitions.
//! - [`audit`] — the RPC surface over the write-audit log.
//! - [`server`] — the `openhuman mcp` host adapter for stdio and HTTP entry
//!   points. The tool catalog, permission model, and agent dispatch stay here;
//!   TinyMCP owns protocol sessions and transport behavior.
//!
//! # Where the boundary fell
//!
//! Three things stayed on purpose, and each is host policy rather than
//! protocol:
//!
//! **Prompt-injection detection** over remote tool definitions. The detector,
//! its rules, and what a hit means belong to this application's threat model. A
//! module dropping tools by criteria of its own would be making a decision it
//! could not explain. The *lexical* half — control characters, prompt-template
//! fences, length caps — does live in the contract, applied by the display
//! accessors on every remote description.
//!
//! **Events.** `tinymcp` reports what happened in its return values. Turning
//! that into a `DomainEvent` happens here, where the vocabulary is known.
//!
//! **The proxy decision.** Whether a proxy applies to MCP traffic is decided by
//! this application's scope setting, per-service list and no-proxy list.
//! [`host::proxy_for_mcp`] consults them and hands `tinymcp` the answer.
//!
//! # Compile-time gate (`mcp` feature)
//!
//! `pub mod mcp;` is always compiled — the family root is a facade. `registry`
//! and `audit` keep their own gate and their `stub`, so a build without the
//! feature still serves `/rpc` without those namespaces.

/// Registers the host-side MCP subscriber and lifecycle integration.
///
/// Core startup calls this without knowing the MCP adapter details. This
/// remembers the process default configuration and registers cleanup for the
/// configured module client; opening the module remains lazy until a caller
/// needs it.
///
/// Idempotent: the subscriber and configured-client shutdown hook register at
/// most once, so the RPC domain enable point and boot-jobs path can both call
/// this function.
#[cfg(feature = "mcp")]
pub fn start(config: &crate::config::Config) {
    registry::bus::init();
    host::set_default_config(config);

    #[cfg(feature = "modules")]
    module_client::McpConfiguredClient::register_shutdown_hook();
}

#[cfg(feature = "mcp")]
pub(crate) mod module_client;

/// Brings this domain up — the build without it, where there is nothing to do.
#[cfg(not(feature = "mcp"))]
pub fn start(_config: &crate::config::Config) {}

/// Boots the domain from the runtime's startup path.
///
/// This is MCP's one entry in `start_boot_once_jobs`, alongside the
/// skill-catalog refresh. It asks TinyMCP to open the configured module object
/// and start its installed-server and reconnect work. The `Once` guard keeps
/// the host's poll task from spawning twice; `src/core` only knows it should
/// call one function.
#[cfg(feature = "mcp")]
pub fn start_boot_jobs(config: &crate::config::Config) {
    start(config);

    let cfg_for_mcp = config.clone();
    tokio::spawn(async move {
        registry::boot::spawn_installed_servers(&cfg_for_mcp).await;
    });
    // Installed servers re-cache their tool lists as they connect above; the
    // configured ones are listed here, so every server's cached tools are
    // refreshed once per app load. Between loads the cache is only replaced
    // or dropped by an MCP change (connect, credentials, disable, uninstall,
    // an edited definition).
    let cfg_for_cache = config.clone();
    tokio::spawn(async move {
        refresh_configured_tool_cache(&cfg_for_cache).await;
    });
    spawn_reconnect_supervisor();
}

/// Lists every configured server and rewrites its cached tools.
#[cfg(feature = "mcp")]
pub(crate) async fn refresh_configured_tool_cache(config: &crate::config::Config) {
    let client = match module_client::McpConfiguredClient::for_config(config) {
        Ok(client) => client,
        Err(error) => {
            tracing::debug!(
                ?error,
                "[mcp] no module; configured tool cache not refreshed"
            );
            return;
        }
    };
    let client_config = host::client_config(config);
    let servers: Vec<String> = match client
        .call(&client_config, tinymcp_bus::names::methods::STATIC_LIST, ())
        .await
    {
        Ok(servers) => servers,
        Err(error) => {
            tracing::debug!(?error, "[mcp] static server list unavailable");
            return;
        }
    };
    for server in servers {
        let outcome: Result<Vec<tinymcp_bus::McpRemoteTool>, _> = client
            .call(
                &client_config,
                tinymcp_bus::names::methods::STATIC_LIST_TOOLS,
                (server.clone(),),
            )
            .await;
        match outcome {
            Ok(tools) => {
                tracing::debug!(
                    server = %server,
                    tools = tools.len(),
                    "[mcp] refreshed configured tool cache"
                );
                ui::discovery::log_configured_server(&server, &tools);
            }
            Err(error) => tracing::debug!(
                server = %server,
                ?error,
                "[mcp] configured tool cache not refreshed"
            ),
        }
    }
}

/// Spawns the reconnect supervisor exactly once per process.
///
/// The supervisor polls every configured module client the process has opened,
/// so the one task covers later workspace switches as well as the initial
/// workspace. It takes no configuration because it reads the client map each
/// tick rather than binding to one workspace.
#[cfg(feature = "mcp")]
fn spawn_reconnect_supervisor() {
    static SUPERVISOR_SPAWNED: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);
    static SUPERVISOR_TASK: std::sync::OnceLock<
        tokio::sync::Mutex<Option<tokio::task::JoinHandle<()>>>,
    > = std::sync::OnceLock::new();
    use std::sync::atomic::Ordering;

    if SUPERVISOR_SPAWNED.swap(true, Ordering::AcqRel) {
        return;
    }
    let task = tokio::spawn(async move {
        registry::supervisor::run().await;
    });
    let task_slot = SUPERVISOR_TASK.get_or_init(|| tokio::sync::Mutex::new(None));
    if let Ok(mut slot) = task_slot.try_lock() {
        *slot = Some(task);
    } else {
        task.abort();
        SUPERVISOR_SPAWNED.store(false, Ordering::Release);
        return;
    }
    crate::core::shutdown::register(|| async {
        if let Some(task) = SUPERVISOR_TASK.get() {
            if let Some(task) = task.lock().await.take() {
                task.abort();
                let _ = task.await;
            }
        }
        SUPERVISOR_SPAWNED.store(false, Ordering::Release);
    });
}

/// Boots the domain from the runtime's startup path — the build without it,
/// where there is nothing to do.
#[cfg(not(feature = "mcp"))]
pub fn start_boot_jobs(_config: &crate::config::Config) {}

/// Every `mcp_*` controller this build serves: the client registry and, with
/// the `mcp` feature, tool UI.
pub fn all_registered_controllers() -> Vec<crate::core::all::RegisteredController> {
    #[cfg_attr(not(feature = "mcp"), allow(unused_mut))]
    let mut controllers = registry::all_mcp_registry_registered_controllers();
    #[cfg(feature = "mcp")]
    controllers.extend(ui::all_mcp_ui_registered_controllers());
    controllers
}

pub mod audit;
// Ungated, like the transport below and for the same reason: `tinymcp` is an
// ordinary dependency, and the startup path calls `host::init` without a `cfg`
// of its own. Gating this would break the build with the domain turned off.
pub mod host;
pub mod registry;
pub mod server;
#[cfg(feature = "mcp")]
pub mod ui;

/// MCP wire-contract response types used by host adapters.
pub mod http_client {
    pub use tinymcp_bus::{
        AuthorizationServerMetadata, McpAuthChallenge, McpAuthorizationContext,
        McpInitializeResult, McpRemoteTool, McpServerToolResult, McpSseEvent,
        ProtectedResourceMetadata,
    };
}
