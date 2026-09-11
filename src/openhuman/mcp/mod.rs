//! MCP family — the host half of Model Context Protocol support.
//!
//! The client moved out to `tinymcp`: both transports, the static
//! config-declared server set, the dynamic registry with its store, supervisor
//! and browser sign-in, and the write-audit log all live there now.
//!
//! What is here is what belongs to this application.
//!
//! # Members
//!
//! - [`host`] — the one `tinymcp` service this process holds, and the
//!   conversion from this application's configuration into what it takes.
//! - [`registry`] — the `mcp_clients` and `mcp_setup` RPC surface, the
//!   agent-facing tools, and the prompt-injection scan over remote tool
//!   definitions.
//! - [`audit`] — the RPC surface over the write-audit log.
//! - [`server`] — the `openhuman mcp` stdio and HTTP server that exposes this
//!   application's own tools to external MCP hosts. This is the *server* side
//!   and did not move: it is bound to the tool registry, the permission model
//!   and the agent turn machinery, none of which a client library should know
//!   about.
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

/// Brings this domain up: its lifecycle subscriber and its service.
///
/// The one entry point core startup calls. It exists so that `src/core/` — which
/// is transport, and carries no business logic — does not have to know that this
/// domain has a service, that opening one can fail, or what to do when it does.
///
/// Never fails. MCP being unavailable must not stop the core coming up, and
/// every caller in the domain already handles an absent service, so a failure is
/// logged here and the process continues without it.
///
/// Idempotent: the subscriber registers once and the service opens once, so the
/// two startup paths that call this — the RPC domain enable point and the
/// boot-jobs path, which are gated separately — can both call it.
#[cfg(feature = "mcp")]
pub fn start(config: &crate::openhuman::config::Config) {
    registry::bus::init();

    if let Err(error) = host::init(config) {
        log::warn!("[mcp] the service could not be opened; continuing without it: {error}");
    }
}

/// Brings this domain up — the build without it, where there is nothing to do.
#[cfg(not(feature = "mcp"))]
pub fn start(_config: &crate::openhuman::config::Config) {}

/// Boots the domain from the runtime's startup path.
///
/// This is MCP's one entry in `start_boot_once_jobs`, the way `harness_init`
/// and the skill-catalog refresh each have theirs. The orchestration here —
/// that the service must exist before installed servers can dial it, and that
/// the reconnect supervisor must run until the process ends — is this domain's
/// own, and the `Once` guard on the supervisor keeps a repeated boot from
/// spawning a second one. `src/core` only knows it should call one function.
#[cfg(feature = "mcp")]
pub fn start_boot_jobs(config: &crate::openhuman::config::Config) {
    start(config);

    let cfg_for_mcp = config.clone();
    tokio::spawn(async move {
        registry::boot::spawn_installed_servers(&cfg_for_mcp).await;
    });
    spawn_reconnect_supervisor();
}

/// Spawns the reconnect supervisor exactly once per process.
///
/// The supervisor walks every workspace host the process has opened, so the
/// single task covers a host opened after boot — a workspace switch, a second
/// embedder — as well as the one booted against. It takes no configuration
/// because it reads the host map each tick rather than binding to one
/// workspace.
#[cfg(feature = "mcp")]
fn spawn_reconnect_supervisor() {
    static SUPERVISOR_SPAWNED: std::sync::Once = std::sync::Once::new();
    SUPERVISOR_SPAWNED.call_once(|| {
        tokio::spawn(async move {
            registry::supervisor::run().await;
        });
    });
}

/// Boots the domain from the runtime's startup path — the build without it,
/// where there is nothing to do.
#[cfg(not(feature = "mcp"))]
pub fn start_boot_jobs(_config: &crate::openhuman::config::Config) {}

pub mod audit;
// Ungated, like the transport below and for the same reason: `tinymcp` is an
// ordinary dependency, and the startup path calls `host::init` without a `cfg`
// of its own. Gating this would break the build with the domain turned off.
pub mod host;
pub mod registry;
pub mod server;

/// The Streamable HTTP transport, from the wire contract's implementation.
///
/// Re-exported under the path this module used to define it at. The bespoke
/// documentation tool and the observability classifier both name these, and
/// neither is gated — so this is not either, exactly as before the extraction.
pub mod http_client {
    pub use tinymcp::transport::http::{McpHttpClient, McpHttpClientBuilder};
    /// The transport's error, re-exported so a caller can inspect a failure
    /// structurally rather than by its rendered text.
    ///
    /// The variant that motivates it is [`McpError::Unauthorized`], whose
    /// `resource_metadata` is what separates a server that wants OAuth from one
    /// that wants a static credential — the difference between offering a sign-in
    /// path and asking for a token. That field is deliberately absent from the
    /// message, so a caller wanting it must have the type.
    ///
    /// Note this crate's own `core::observability` classifier does **not** use
    /// it: it reads rendered text, because it classifies failures that have
    /// already crossed an RPC boundary and arrive as strings. This re-export is
    /// for an in-process consumer, which holds the real error and should not be
    /// reduced to matching on wording that upstream is free to reword.
    pub use tinymcp::Error as McpError;
    pub use tinymcp::{redact_endpoint, render_tool_result};
    pub use tinymcp_bus::{
        AuthorizationServerMetadata, McpAuthChallenge, McpAuthorizationContext,
        McpInitializeResult, McpRemoteTool, McpServerToolResult, McpSseEvent,
        ProtectedResourceMetadata,
    };
}

/// The statically declared server set, from the wire contract's
/// implementation.
#[cfg(feature = "mcp")]
pub mod config_servers {
    pub use tinymcp::transport::stdio::McpStdioClient;
    pub use tinymcp::{
        McpRegistrySource, McpServerDefinition, McpServerRegistry, McpTransportClient,
    };
    /// The auth shape a *definition in this registry* carries.
    ///
    /// Distinct from `config::McpAuthConfig`, which is the shape this
    /// application's own TOML declares — the two were one type before the
    /// extraction. A caller reading `McpServerDefinition::auth` needs this one,
    /// and without the re-export cannot name it at all.
    pub use tinymcp_bus::McpAuthConfig as McpDefinitionAuth;
}
