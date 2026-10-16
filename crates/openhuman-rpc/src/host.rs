//! The shared host boot: one entry per host shape, so each host's `main`
//! becomes a few lines instead of a hand-assembled sequence.
//!
//! | Entry | Replaces | Builder |
//! |---|---|---|
//! | [`cli`] | `tinyhumans::install` → the server launcher → `run_core_from_args` | [`cli_builder`]: the `cli` preset, connected, with the server launcher and the `http_host` controllers |
//! | [`desktop`] | `tinyhumans::install` + the embedded server entry | [`desktop_builder`]: the `desktop` preset, connected, with the bearer, listener, services, server launcher and `http_host` controllers |
//! | [`tui`] | `tinyhumans::install` + `session_store::install_for_host` + `CoreBuilder(full, none)` | [`tui_builder`]: the `tui` preset, connected ([`tui`] adds the storage-backed session store) |
//!
//! Each `*_builder` returns a [`tinyhumans::RuntimeBuilder`] so a host can
//! adjust it (product identity, hooks, a different ranker) before handing it
//! to the matching `serve_*` / `build` call.
//!
//! "Connected" means [`tinyhumans::RuntimeBuilder::connect`]: the SDK
//! transport (process global and bound to the runtime), the hosted RPC
//! proxies and, with the `jev` feature, the Jev `tool_search` ranker.
//!
//! # Behavior notes
//!
//! - The desktop and CLI servers install the storage-backed session store for
//!   the life of the process (see `server::shims::build_and_serve`), exactly as
//!   the `run_server*` shims do; the TUI hands it to the builder, which
//!   restores the previous provider when its runtime drops at exit. Both
//!   resolve one storage URL (`OPENHUMAN_STORAGE_URL`, else `[storage] url`,
//!   else `sqlite:<workspace>/storage`).
//! - Builder seams follow embed's install/restore rules: the hosted and
//!   `http_host` controllers and the server launcher stay for the process; the
//!   Jev ranker is restored when the runtime drops. A desktop server that
//!   restarts in place re-installs it on the next build.
//! - The runtime does not start background services itself; [`desktop`]
//!   starts them through `server::serve`, as the shims always have.
//!
//! [`tinyhumans::RuntimeBuilder`]: openhuman_tinyhumans::RuntimeBuilder
//! [`tinyhumans::RuntimeBuilder::connect`]: openhuman_tinyhumans::RuntimeBuilder::connect

#[cfg(feature = "server")]
use std::sync::Arc;

#[cfg(feature = "server")]
use openhuman_tinyhumans::embed::{ServiceSet, TokenSource};
use openhuman_tinyhumans::RuntimeBuilder;
#[cfg(feature = "server")]
use tokio_util::sync::CancellationToken;

#[cfg(feature = "server")]
pub use crate::server::EmbeddedReadySignal;

#[path = "host/secret_storage.rs"]
mod secret_storage;

/// The CLI host's builder: the `cli` preset with this crate's server as the
/// `run` / `serve` launcher and the `http_host.*` controllers registered.
#[cfg(feature = "server")]
pub fn cli_builder() -> RuntimeBuilder {
    RuntimeBuilder::cli()
        .server_launcher(crate::server::cli::launch)
        .controller_extension(crate::http_host::extension())
}

/// Parse the side-effect-free embedding capability command.
#[cfg(any(feature = "server", test))]
fn embed_info(args: &[String]) -> anyhow::Result<Option<crate::embed::RuntimeInfo>> {
    if args.first().map(String::as_str) != Some("embed") {
        return Ok(None);
    }
    if !matches!(args.get(1).map(String::as_str), Some("info"))
        || args.iter().skip(2).any(|arg| arg != "--json")
    {
        anyhow::bail!("usage: openhuman-core embed info [--json]");
    }
    Ok(Some(crate::embed::RuntimeBuilder::standard().describe()))
}

/// Run the core's command-line dispatcher on `args` (without the binary
/// name) as the `openhuman-core` binary does: connected to the TinyHumans
/// backend, with this crate's server behind `run` / `serve`.
///
/// # Errors
///
/// The transport could not be built, a controller extension was refused, or
/// the dispatched command failed.
#[cfg(feature = "server")]
pub fn cli(args: &[String]) -> anyhow::Result<()> {
    // Config reads below (and in the dispatcher) need the enricher.
    openhuman_tinyhumans::register_hooks();
    // Introspection belongs above the core: never introduce a core -> embed dependency.
    if let Some(info) = embed_info(args)? {
        println!("{}", serde_json::to_string_pretty(&info)?);
        return Ok(());
    }

    log::debug!(
        "[rpc:host] cli command={} argc={}",
        args.first().map(String::as_str).unwrap_or("<none>"),
        args.len()
    );
    // The preflight reads the URL before the dispatcher loads `.env`
    // itself, so a URL supplied through the dotenv file must be loaded now.
    if let Err(error) = crate::core_host::core::cli::load_dotenv_for_cli() {
        log::warn!(
            "[rpc:host] cli: early dotenv load failed ({error}); a host setting supplied only \
         in that file will not be seen"
        );
    }
    secret_storage::install();
    let mut builder = cli_builder();
    if cli_command_uses_storage(args, |namespace| {
        // `subsystems` only renders status; it never touches stored state.
        namespace != "subsystems"
            && crate::core_host::core::all::cli_handler_for_namespace(namespace).is_some()
    }) {
        // A one-shot command reads the same backend the server would: the
        // resolved storage URL, opened on the core's storage runtime so the
        // backend outlives this call.
        let provider = crate::core_host::storage::block_on_anyhow(session_store_for_boot())?;
        log::debug!("[rpc:host] cli: storage-backed session store installed");
        builder = builder.session_store(provider);
    }
    builder.run_from_args(args)
}

/// Whether the CLI subcommand in `args` should open the configured storage
/// backend itself, following the dispatcher's own grammar
/// (`core::cli::parse_launch_options` then the subcommand match): only the
/// model/provider launch flags precede the command, and the first other token
/// is the command. `run` / `serve` open storage in their own server boot;
/// help, the moved TUI names and `sentry-test` never touch stored state; a
/// bare namespace only prints help unless it has a domain CLI handler
/// (`has_cli_handler`, e.g. `voice`), which runs. `help` counts only where
/// the dispatcher reads it (the command, the function slot or the slot after
/// it), never as an option value.
#[cfg(feature = "server")]
fn cli_command_uses_storage(args: &[String], has_cli_handler: impl Fn(&str) -> bool) -> bool {
    let is_help = |arg: &str| matches!(arg, "-h" | "--help" | "help");
    let mut rest = args.iter().map(String::as_str).peekable();
    while let Some(arg) = rest.peek().copied() {
        match arg {
            "--model" | "--model-id" | "-m" | "--provider" | "--provider-id" | "-p" => {
                rest.next();
                match rest.next() {
                    // The dispatcher rejects a missing or dash-led value with
                    // its own error; do not open storage ahead of it.
                    None => return false,
                    Some(value) if value.starts_with('-') => return false,
                    Some(_) => {}
                }
            }
            _ if arg.starts_with("--model=")
                || arg.starts_with("--model-id=")
                || arg.starts_with("--provider=")
                || arg.starts_with("--provider-id=") =>
            {
                rest.next();
            }
            _ => break,
        }
    }
    let Some(command) = rest.next() else {
        return false;
    };
    if is_help(command) {
        return false;
    }
    let tail: Vec<&str> = rest.collect();
    match command {
        "run" | "serve" | "tui" | "chat" | "sentry-test" => false,
        // The MCP server speaks stdio when given no function and runs agent
        // sessions, so it needs the backend.
        "mcp" | "mcp-server" => !tail.iter().any(|a| is_help(a)),
        // `call` and `agent` print help when given none, or on a help token
        // or flag anywhere in their own tails.
        "call" | "agent" => match tail.first() {
            None => false,
            Some(_) => !tail.iter().any(|a| is_help(a)),
        },
        // A namespace reads help only in the function slot and the slot
        // after it; later `--help` tokens are option values to its parser.
        namespace => match tail.as_slice() {
            [] => has_cli_handler(namespace),
            [function, ..] if is_help(function) => false,
            [_, slot, ..] if is_help(slot) => false,
            _ => true,
        },
    }
}

/// What the embedded desktop server binds and how it authenticates.
#[cfg(feature = "server")]
#[derive(Clone)]
pub struct DesktopOptions {
    /// Bind host; `None` falls back to `OPENHUMAN_CORE_HOST`, then loopback.
    pub host: Option<String>,
    /// Preferred port; `None` falls back to `OPENHUMAN_CORE_PORT`, then 7788.
    /// A stale listener of our own is taken over; see `server::serve`.
    pub port: Option<u16>,
    /// Serve Socket.IO alongside HTTP JSON-RPC (default `true`).
    pub socketio: bool,
    /// The per-launch bearer, handed over in memory. `None` keeps the
    /// env-or-file token.
    pub rpc_token: Option<Arc<String>>,
}

#[cfg(feature = "server")]
impl Default for DesktopOptions {
    fn default() -> Self {
        Self {
            host: None,
            port: None,
            socketio: true,
            rpc_token: None,
        }
    }
}

#[cfg(feature = "server")]
impl std::fmt::Debug for DesktopOptions {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Never print the bearer.
        f.debug_struct("DesktopOptions")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("socketio", &self.socketio)
            .field("rpc_token", &self.rpc_token.as_ref().map(|_| "<redacted>"))
            .finish()
    }
}

/// The desktop host's builder: the `desktop` preset with every background
/// service except the core update poller (Socket.IO per `options`), the
/// in-memory bearer and listener, this
/// crate's server launcher and the `http_host.*` controllers.
#[cfg(feature = "server")]
pub fn desktop_builder(options: &DesktopOptions) -> RuntimeBuilder {
    let mut services = ServiceSet::desktop();
    services.socketio = options.socketio;
    // The shell updates through the Tauri updater; releases publish core
    // archives for Linux only, so the core's own poller could only report a
    // missing asset here (Sentry TAURI-RUST-122R/122S/13B8/13B9).
    services.update_scheduler = false;
    let mut builder = RuntimeBuilder::desktop()
        .services(services)
        .server_launcher(crate::server::cli::launch)
        .controller_extension(crate::http_host::extension());
    if let Some(token) = options.rpc_token.clone() {
        builder = builder.token(TokenSource::Fixed(token));
    }
    if let Some(host) = options.host.clone() {
        builder = builder.listen_host(host);
    }
    if let Some(port) = options.port {
        builder = builder.listen_port(port);
    }
    builder
}

/// Boot the embedded desktop core and serve it until `shutdown_token` is
/// cancelled: [`desktop_builder`], connected, then [`serve_desktop`].
///
/// # Errors
///
/// The transport or runtime could not be built, or the listener failed.
#[cfg(feature = "server")]
pub async fn desktop(
    options: DesktopOptions,
    shutdown_token: CancellationToken,
    ready_tx: tokio::sync::oneshot::Sender<EmbeddedReadySignal>,
) -> anyhow::Result<()> {
    openhuman_tinyhumans::register_hooks();
    secret_storage::install();
    log::debug!("[rpc:host] desktop options={options:?}");
    serve_desktop(desktop_builder(&options), shutdown_token, ready_tx).await
}

/// Connect `builder` and serve it as the embedded desktop core: the session
/// store installed, the runtime built, background services started, the
/// listener bound (taking over a stale listener of our own on the preferred
/// port, else falling back), `ready_tx` signalled with the bound port, and
/// the server run until `shutdown_token` is cancelled.
///
/// # Errors
///
/// The transport or runtime could not be built, or the listener failed.
#[cfg(feature = "server")]
pub async fn serve_desktop(
    builder: RuntimeBuilder,
    shutdown_token: CancellationToken,
    ready_tx: tokio::sync::oneshot::Sender<EmbeddedReadySignal>,
) -> anyhow::Result<()> {
    let builder = builder.connect().map_err(|error| {
        log::warn!("[rpc:host] desktop: TinyHumans connection failed: {error}");
        anyhow::Error::new(error)
    })?;
    crate::server::shims::build_and_serve(builder, Some(ready_tx), Some(shutdown_token)).await
}

/// The terminal UI's builder: the `tui` preset (every domain, no transport,
/// no background services). [`tui`] adds the storage-backed session store.
#[cfg(feature = "session-store")]
pub fn tui_builder() -> RuntimeBuilder {
    RuntimeBuilder::tui()
}

/// Boot the TUI's in-process core, connected. The returned runtime owns the
/// core for the session; `Runtime::core_runtime` hands the TUI its
/// `CoreRuntime`.
///
/// Conversations live in the storage backend the host's configuration names
/// (`OPENHUMAN_STORAGE_URL` / `[storage] url`, else `sqlite:<workspace>/storage`;
/// [`crate::session_store::provider_for_host`]).
///
/// # Errors
///
/// A configured storage URL could not be opened, or the transport or runtime
/// could not be built.
#[cfg(feature = "session-store")]
pub async fn tui() -> anyhow::Result<openhuman_tinyhumans::embed::Runtime> {
    openhuman_tinyhumans::register_hooks();
    secret_storage::install();
    log::debug!("[rpc:host] tui: building connected runtime");
    let session_store = session_store_for_boot().await?;
    let runtime = tui_builder()
        .session_store(session_store)
        .build()
        .await
        .map_err(|error| {
            log::warn!("[rpc:host] tui: runtime build failed: {error}");
            anyhow::Error::new(error)
        })?;
    log::info!("[rpc:host] tui: core built (DomainSet::full, ServiceSet::none)");
    Ok(runtime)
}

/// The storage-backed session store a host opens *before* its runtime is
/// connected. Opening it loads the config (to resolve `[storage] url`), so the
/// TinyHumans config hooks go in first: without the enricher the load-time
/// migrations cannot tell a managed endpoint from a custom one and would
/// rewrite a legacy managed config.
#[cfg(feature = "session-store")]
async fn session_store_for_boot(
) -> anyhow::Result<std::sync::Arc<dyn tinyagents_session::port::SessionStoreProvider>> {
    openhuman_tinyhumans::register_hooks();
    crate::session_store::provider_for_host().await
}

#[cfg(test)]
#[path = "host_tests.rs"]
mod tests;
