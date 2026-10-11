//! `CoreBuilder` → `CoreRuntime`: the embeddable composition surface.
//!
//! This is the first-class library API for hosting the OpenHuman core. It
//! splits the monolithic `run_server_inner` into two phases:
//!
//! 1. [`CoreBuilder::build`] — *initialization only*: register controllers, load
//!    the master key, seed the RPC bearer, initialize workspace-bound stores,
//!    and run the pure-registration part of [`bootstrap_core_runtime`]. No port
//!    is bound and `ServiceSet::none` / `ServiceSet::headless_api` start no
//!    background loops. After `build`, [`CoreRuntime::invoke`] can dispatch any
//!    RPC method in-process, and agent turns can run — so a harness-only embedder
//!    (`ServiceSet::none`) needs nothing more.
//! 2. *Transport + background services*: `openhuman_rpc::server::serve` binds
//!    the HTTP listener, mounts the router, fires the readiness signal, calls
//!    [`CoreRuntime::start_services`], and serves until shutdown. A runtime
//!    with no transport calls `start_services` itself.
//!
//! The host boot in `openhuman-rpc` (`host::{cli, desktop}`) and its
//! `run_server*` entry points build on this builder, so the desktop shell, the
//! standalone CLI, and any new embedder share one path.
//! See the pluggable-core work (`core::runtime`) for how this fits with
//! [`context`](crate::core::runtime::context) and `services`.

use std::sync::Arc;

use crate::config::Config;
use crate::core::runtime::context::CoreContext;
use crate::core::types::HostKind;

pub use super::domain_set::DomainSet;

/// Selects which background services and transports a [`CoreRuntime`] runs.
///
/// Each flag is independent. Presets cover the common hosts:
/// [`ServiceSet::desktop`] (everything — the Tauri shell / standalone CLI),
/// [`ServiceSet::headless_api`] (HTTP JSON-RPC only — single-core cloud), and
/// [`ServiceSet::none`] (no transport, no background work — library / harness).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ServiceSet {
    /// Bind the axum HTTP server and serve `POST /rpc` (+ the other core routes).
    pub rpc_http: bool,
    /// Mount the Socket.IO realtime layer on the HTTP server (requires `rpc_http`).
    pub socketio: bool,
    /// Spawn the cron scheduler (still gated at runtime by `config.cron.enabled`).
    pub cron: bool,
    /// Spawn realtime channel listeners (Telegram, Discord, …).
    pub channels: bool,
    /// Spawn login-gated services (local AI, voice, autocomplete).
    pub login_gated: bool,
    /// Spawn the periodic self-update checker.
    pub update_scheduler: bool,
    /// Start memory queue workers during runtime bootstrap.
    pub memory_queue: bool,
    /// Refresh the skill catalog during runtime bootstrap.
    pub skill_catalog_refresh: bool,
    /// Boot installed MCP servers and supervise reconnects during runtime bootstrap.
    pub mcp_boot: bool,
    /// Composio integration sync: periodic connection sync + one-shot memory-source reconcile.
    pub integrations: bool,
    /// Workspace memory-source periodic sync — repos, folders, RSS, web pages.
    pub memory_sync: bool,
}

impl ServiceSet {
    /// Everything on — the desktop shell and the standalone `openhuman-core run`.
    pub fn desktop() -> Self {
        Self {
            rpc_http: true,
            socketio: true,
            cron: true,
            channels: true,
            login_gated: true,
            update_scheduler: true,
            memory_queue: true,
            skill_catalog_refresh: true,
            mcp_boot: true,
            integrations: true,
            memory_sync: true,
        }
    }

    /// HTTP JSON-RPC only — a single-core cloud/server deployment. No Socket.IO,
    /// no cron/channels/login-gated services; the supervisor decides those per plan.
    pub fn headless_api() -> Self {
        Self {
            rpc_http: true,
            socketio: false,
            cron: false,
            channels: false,
            login_gated: false,
            update_scheduler: false,
            memory_queue: false,
            skill_catalog_refresh: false,
            mcp_boot: false,
            integrations: false,
            memory_sync: false,
        }
    }

    /// No transport and no background services — for library / harness embedders
    /// that only drive the core through [`CoreRuntime::invoke`] and agent turns.
    pub fn none() -> Self {
        Self {
            rpc_http: false,
            socketio: false,
            cron: false,
            channels: false,
            login_gated: false,
            update_scheduler: false,
            memory_queue: false,
            skill_catalog_refresh: false,
            mcp_boot: false,
            integrations: false,
            memory_sync: false,
        }
    }

    /// A long-lived embedded host: no transport, but the background work such
    /// a session expects.
    ///
    /// Named for the shape, not a consumer — see [`DomainSet::embedded`].
    ///
    /// `rpc_http: false` is the payoff of embedding through the typed facade
    /// rather than HTTP — no port bound, no bearer-token handshake, no
    /// loopback listener. Flip it on only if the host also needs to serve external clients.
    ///
    /// `socketio` stays off because an embedded host reads state through the
    /// facade and the core event bus in-process; `channels` stays off because
    /// such a host owns its own harness and networking transports.
    pub fn embedded() -> Self {
        Self {
            rpc_http: false,
            socketio: false,
            cron: true,
            channels: false,
            login_gated: true,
            update_scheduler: false,
            memory_queue: true,
            skill_catalog_refresh: true,
            mcp_boot: false,
            integrations: false,
            memory_sync: true,
        }
    }
}

/// How the per-process RPC bearer token is seeded.
pub enum TokenSource {
    /// An in-memory bearer supplied by the embedder (the Tauri shell hands its
    /// `CoreProcessHandle.rpc_token` this way). Seeded via
    /// [`crate::core::auth::init_rpc_token_with_value`] — never crosses the
    /// process environment.
    Fixed(Arc<String>),
    /// Standalone fallback: read `OPENHUMAN_CORE_TOKEN` from the environment when
    /// present (operator config), otherwise generate a fresh token and write
    /// `{root}/core.token` (0o600 on Unix) so CLI callers can authenticate.
    EnvOrFile,
}

/// Builder for a [`CoreRuntime`]. Construct with [`CoreBuilder::new`], then
/// [`CoreBuilder::build`] to initialize the core.
pub struct CoreBuilder {
    host_kind: HostKind,
    token: TokenSource,
    services: ServiceSet,
    domains: DomainSet,
    tool_groups: crate::tools::toolpacks::ToolGroups,
    host: Option<String>,
    port: Option<u16>,
    config: Option<crate::config::Config>,
    backend_transport: Option<std::sync::Arc<dyn crate::backend::transport::BackendTransport>>,
}

impl CoreBuilder {
    /// Start a builder for the given host kind. Defaults: [`TokenSource::EnvOrFile`],
    /// [`ServiceSet::desktop`], and [`DomainSet::full`].
    pub fn new(host_kind: HostKind) -> Self {
        Self {
            host_kind,
            token: TokenSource::EnvOrFile,
            services: ServiceSet::desktop(),
            domains: DomainSet::full(),
            tool_groups: Default::default(),
            host: None,
            port: None,
            config: None,
            backend_transport: None,
        }
    }

    /// Bind the transport this core's handlers reach the hosted TinyHumans
    /// backend through (see [`crate::backend::transport`]).
    ///
    /// Optional: without it the core resolves the process-global transport
    /// installed with
    /// [`install_backend_transport`](crate::backend::transport::install_backend_transport),
    /// and with neither every backend-touching call degrades to a typed
    /// "backend unavailable" error while agents, memory, tools and RPC keep
    /// working. Library hosts that build one runtime per process prefer this
    /// builder form; the desktop shell and CLI, which boot the core through
    /// `openhuman_rpc::host`, install the global.
    pub fn backend_transport(
        mut self,
        transport: std::sync::Arc<dyn crate::backend::transport::BackendTransport>,
    ) -> Self {
        self.backend_transport = Some(transport);
        self
    }

    /// Choose which background services and transports this runtime runs.
    pub fn services(mut self, services: ServiceSet) -> Self {
        self.services = services;
        self
    }

    /// Choose which domain families exist at runtime (default [`DomainSet::full`]).
    /// `harness()` builds the embeddable agent core; `none()` disables every
    /// domain family while retaining transport built-ins and core infrastructure.
    pub fn domains(mut self, domains: DomainSet) -> Self {
        self.domains = domains;
        self
    }

    /// Choose how each tool group reaches the model (default: every group
    /// withheld behind `use_skill`, the desktop app's shape).
    ///
    /// The third narrowing axis, independent of both `services` and `domains`:
    /// `ServiceSet` picks the background services, `DomainSet` picks which
    /// families exist, and this picks how the tools of the families that do
    /// exist are disclosed — advertised on the wire, withheld behind the pack
    /// proxy, or not registered at all.
    ///
    /// ```no_run
    /// # use openhuman_core::core::runtime::CoreBuilder;
    /// # use openhuman_core::tools::toolpacks::{GroupMode, ToolGroups};
    /// # fn f(b: CoreBuilder) -> CoreBuilder {
    /// b.tool_groups(
    ///     ToolGroups::none()
    ///         .with("documents", GroupMode::Advertised)
    ///         .with("workflows", GroupMode::Withheld),
    /// )
    /// # }
    /// ```
    ///
    /// Narrowing only: a group set to `Advertised` whose tools are compiled
    /// out, or whose `DomainGroup` is off under `domains`, stays absent.
    pub fn tool_groups(mut self, tool_groups: crate::tools::toolpacks::ToolGroups) -> Self {
        self.tool_groups = tool_groups;
        self
    }

    /// Choose how the RPC bearer token is seeded.
    pub fn token(mut self, token: TokenSource) -> Self {
        self.token = token;
        self
    }

    /// Override the bind host (default: `OPENHUMAN_CORE_HOST` env or `127.0.0.1`).
    pub fn host(mut self, host: impl Into<String>) -> Self {
        self.host = Some(host.into());
        self
    }

    /// Override the bind port (default: `OPENHUMAN_CORE_PORT` env or `7788`).
    pub fn port(mut self, port: u16) -> Self {
        self.port = Some(port);
        self
    }

    /// Supply the [`Config`](crate::config::Config) outright instead
    /// of letting `build()` discover one from `config.toml` and the environment.
    ///
    /// Without this an embedder can only configure the core by setting
    /// environment variables before `build()` — process-global, order-dependent
    /// relative to a call it does not appear in, and silently wrong if a later
    /// caller in the same process wants different values. With it, every knob
    /// the core reads from config (workspace, action dir, autonomy tier, MCP
    /// servers, provider routes) is an ordinary struct field.
    ///
    /// The config is used **verbatim**: no `config.toml` read and no env
    /// overlay. Call
    /// [`apply_env_overrides`](crate::config::Config::apply_env_overrides)
    /// yourself first if you want the environment to participate.
    pub fn config(mut self, config: crate::config::Config) -> Self {
        self.config = Some(config);
        self
    }

    /// Root the core's state at `dir` — sessions, memory, attachments, skills.
    ///
    /// Sugar over [`config`](Self::config) for the common case of "same
    /// configuration, different workspace"; starts from the config already
    /// supplied, or [`Config::default`](Default::default) when none is.
    pub fn workspace(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        let dir = dir.into();
        let mut config = self.config.take().unwrap_or_default();
        config.workspace_dir = dir.clone();
        // Credential profiles and the file-backed keyring resolve from
        // `config_path`'s parent, not from `workspace_dir`. Rooting only the
        // workspace while leaving the default config path would keep sessions
        // and credentials in the previous config root even though this method
        // documents `dir` as rooting "core state" — so set a deterministic
        // config path beside the workspace, mirroring the harness's `Dir`
        // layout (`<root>/config.toml` next to `<root>/workspace`).
        config.config_path = dir.join("config.toml");
        self.config = Some(config);
        self
    }

    /// Set the agent's read/write root for acting tools (`action_dir`).
    ///
    /// Sugar over [`config`](Self::config), like [`workspace`](Self::workspace).
    /// Distinct from the workspace on purpose: the workspace holds internal
    /// state the agent must never write to, and `is_workspace_internal_path`
    /// enforces that separation fail-closed.
    pub fn action_dir(mut self, dir: impl Into<std::path::PathBuf>) -> Self {
        let mut config = self.config.take().unwrap_or_default();
        config.action_dir = dir.into();
        self.config = Some(config);
        self
    }

    /// Point the core's backend calls at `url` (`Config::api_url`).
    ///
    /// Sugar over [`config`](Self::config), like [`workspace`](Self::workspace).
    /// Worth having as its own method because the value reaches more than the
    /// obvious client: `/auth/me` session validation, the hosted-backend
    /// surfaces all resolve through it. A host that sets only one of
    /// those has the other two pointing at a different deployment, which fails
    /// as "backend rejected session token" rather than as a mismatch.
    pub fn backend_url(mut self, url: impl Into<String>) -> Self {
        let mut config = self.config.take().unwrap_or_default();
        config.api_url = Some(url.into());
        self.config = Some(config);
        self
    }

    /// Initialize the core: register controllers, load the master key, seed the
    /// RPC bearer, initialize workspace-bound stores, and run
    /// [`bootstrap_core_runtime`]. Binds no port and starts no transport.
    ///
    /// The init sequence itself is owned by [`CoreContext::init`] (Phase 2,
    /// Stage A).
    pub async fn build(self) -> anyhow::Result<CoreRuntime> {
        let (ctx, has_operator_token, config) = CoreContext::init_with_config(
            self.host_kind,
            &self.token,
            self.domains,
            self.tool_groups.clone(),
            self.config,
            self.backend_transport,
        )
        .await?;

        // Retired scheduled jobs must be pruned before `build()` exposes
        // in-process RPC or agent turns. Running this from `serve()` is too
        // late for embedders that only build and invoke.
        if let Some(cfg) = config.as_ref() {
            crate::core::runtime::services::run_legacy_migrations(cfg).await;
        }

        // Reap agent runs orphaned by a previous process (crash / restart /
        // deploy). Here, and not with the other boot-once jobs, because those
        // run from `serve()`: an embedder that only calls `build()` and then
        // `invoke()` never reaches them, and `openhuman.agent_runs_active` is
        // dispatchable the moment this returns. The core is a single in-process
        // runtime, so a run left Pending/Running/Interrupted in the durable
        // status store has no executor to advance it and would be listed as
        // active forever. Best-effort — a store that cannot be read logs and
        // reaps nothing rather than failing the build.
        if let Some(cfg) = config.as_ref() {
            crate::agent::tinyagents::reaper::reap_orphaned_runs(&cfg.workspace_dir).await;
        }

        Ok(CoreRuntime {
            ctx,
            config,
            services: self.services,
            has_operator_token,
            host: self.host,
            port: self.port,
            service_tasks: crate::core::runtime::services::ServiceTasks::default(),
        })
    }
}

/// A built, initialized core. Dispatch RPC in-process with [`CoreRuntime::invoke`],
/// start its background services with [`CoreRuntime::start_services`], or hand it
/// to `openhuman_rpc::server::serve` to run the selected transport as well.
pub struct CoreRuntime {
    ctx: Arc<CoreContext>,
    config: Option<Config>,
    services: ServiceSet,
    has_operator_token: bool,
    host: Option<String>,
    port: Option<u16>,
    /// The background services [`start_services`](Self::start_services)
    /// started; aborted by [`stop_services`](Self::stop_services) and when
    /// the runtime drops.
    service_tasks: crate::core::runtime::services::ServiceTasks,
}

impl CoreRuntime {
    /// The services/transports this runtime is configured to run.
    pub fn services(&self) -> ServiceSet {
        self.services
    }

    /// The initialized core context (host identity + resolved workspace).
    pub fn context(&self) -> &Arc<CoreContext> {
        &self.ctx
    }

    /// Dispatch an RPC method in-process — the same path the HTTP `/rpc` handler
    /// and the CLI use ([`crate::core::invoke::invoke_method`]). No network involved.
    pub async fn invoke(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.invoke_in(Arc::clone(&self.ctx), method, params).await
    }

    /// [`invoke`](Self::invoke) under a caller-supplied context instead of the
    /// runtime's own — typically a per-agent child from
    /// [`CoreContext::derive_with`], so the handler's config loader, DomainSet
    /// gate and tool-group filter all read that agent's overlay.
    pub async fn invoke_in(
        &self,
        ctx: Arc<CoreContext>,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        log::trace!("[core-runtime] invoke_in method={method}");
        CoreContext::scope(
            ctx,
            crate::core::invoke::invoke_method(
                crate::core::invoke::default_state(),
                method,
                params,
            ),
        )
        .await
    }

    /// Run an arbitrary future with `ctx` as the ambient [`CoreContext`] — the
    /// native (non-RPC) counterpart of [`invoke_in`](Self::invoke_in) for
    /// callers that reach a domain operation directly, such as an embedded
    /// agent turn. Bypasses the registered-RPC dispatch gate, so the caller is
    /// responsible for honouring `ctx.domains()` itself.
    pub async fn run_in<F: std::future::Future>(&self, ctx: Arc<CoreContext>, fut: F) -> F::Output {
        CoreContext::scope(ctx, fut).await
    }

    /// The host this runtime was built to bind, when the builder set one.
    pub fn host(&self) -> Option<&str> {
        self.host.as_deref()
    }

    /// The port this runtime was built to bind, when the builder set one.
    pub fn port(&self) -> Option<u16> {
        self.port
    }

    /// Whether the RPC bearer came from the operator (env or an in-memory
    /// handoff) rather than the self-generated `{workspace}/core.token`. A
    /// server must not bind a public address without one (#1919).
    pub fn has_operator_token(&self) -> bool {
        self.has_operator_token
    }

    /// Record that a transport bound its listener at `local_addr`.
    pub fn listener_bound(&self, local_addr: std::net::SocketAddr) {
        #[cfg(feature = "modules")]
        crate::desktop::control::set_listener_is_loopback(local_addr.ip().is_loopback());
        #[cfg(not(feature = "modules"))]
        let _ = local_addr;
    }

    /// Cleanup to run once a transport has stopped serving, whether it ended
    /// cleanly or with an error.
    ///
    /// Memory holds nothing to flush: turns are logged as they happen and
    /// queued background jobs are persisted. There is no local model runtime
    /// to stop either: the user runs Ollama / LM Studio / MLX themselves and
    /// OpenHuman never spawns it.
    pub async fn exit_cleanup(&self) {
        if let Err(error) = crate::integrations::composio::trigger_history::close_global().await {
            log::warn!("[composio][history] archive shutdown failed: {error}");
        }
        log::debug!("[core] shutdown: exit cleanup done (no owned local runtime to stop)");
    }

    /// Spawn each selected background service.
    ///
    /// A transport calls this once its listener is bound, so a failed bind
    /// never leaves pollers, one-shot jobs, MCP processes or socket
    /// reconnect work running without a live runtime. A runtime with no
    /// transport calls it directly.
    ///
    /// Idempotent while they run; the long-lived loops stop with
    /// [`stop_services`](Self::stop_services) or when this runtime drops.
    pub async fn start_services(&self) {
        crate::core::runtime::services::start_selected_services(
            &self.service_tasks,
            self.services,
            self.config.as_ref(),
            &self.ctx,
        )
        .await;
    }

    /// Stop the services [`start_services`](Self::start_services) started
    /// (they may be restarted). Also runs on drop.
    pub fn stop_services(&self) {
        self.service_tasks.stop();
    }
}

#[cfg(test)]
#[path = "builder_tests.rs"]
mod tests;
