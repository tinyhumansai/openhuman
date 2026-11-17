//! The port through which `openhuman-core run` / `serve` starts a server.
//!
//! The JSON-RPC server lives in `openhuman-rpc`, which sits above this crate,
//! so the CLI cannot call it directly. A host binary installs a launcher once
//! at startup (through `openhuman_rpc::host::cli`) before it hands
//! its arguments to [`run_core_from_args`](crate::run_core_from_args).
//! Without one, `run` / `serve` fail with an error that says so instead of
//! starting a core nothing can reach.

use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, OnceLock};

/// An opaque, host-owned boot description handed through the CLI to the
/// installed [`ServerLauncher`].
///
/// The CLI never boots a core itself: `run` / `serve` start whatever the
/// launcher builds, and the launcher lives in a layer above this crate. So a
/// host that wants its own configured builder (domains, services, token,
/// listener, host kind) to be what the CLI boots with wraps it here, passes it
/// to [`run_core_from_args_with`](crate::run_core_from_args_with), and its
/// launcher takes it back out with [`HostBoot::take`]. The core only carries
/// it; it never looks inside.
#[derive(Clone)]
pub struct HostBoot(Arc<Mutex<Option<Box<dyn Any + Send>>>>);

impl HostBoot {
    /// Wrap `value` for the launcher to take.
    pub fn new<T: Any + Send>(value: T) -> Self {
        Self(Arc::new(Mutex::new(Some(Box::new(value)))))
    }

    /// Take the wrapped value out, once. `None` if it was already taken or is
    /// not a `T` (a mismatched type is left in place).
    pub fn take<T: Any + Send>(&self) -> Option<T> {
        let mut slot = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match slot.take()?.downcast::<T>() {
            Ok(value) => Some(*value),
            Err(other) => {
                *slot = Some(other);
                None
            }
        }
    }
}

impl std::fmt::Debug for HostBoot {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("HostBoot(..)")
    }
}

impl PartialEq for HostBoot {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for HostBoot {}

/// What the `run` / `serve` subcommand asked for.
///
/// Precedence: a field the operator set explicitly on the command line
/// (`host`, `port`, `--jsonrpc-only`, `--headless-api`, `--mode`) wins over
/// the host's [`host_boot`](Self::host_boot) builder, which in turn wins over
/// the launcher's own preset. Unset flags (`None`, or the `socketio_enabled`
/// default) leave the builder's value alone.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServeRequest {
    /// Bind host, when the operator passed one.
    pub host: Option<String>,
    /// Bind port, when the operator passed one.
    pub port: Option<u16>,
    /// Serve Socket.IO alongside HTTP JSON-RPC (`--jsonrpc-only` clears it).
    pub socketio_enabled: bool,
    /// Request/response API only, with no background services.
    pub headless_api: bool,
    /// The operating mode (`--mode`).
    pub mode: crate::core::runtime::Mode,
    /// The operator's SaaS config file (`--saas-config`); required in SaaS mode.
    pub saas_config: Option<std::path::PathBuf>,
    /// The host's pre-configured builder, when it passed one to
    /// [`run_core_from_args_with`](crate::run_core_from_args_with).
    pub host_boot: Option<HostBoot>,
}

/// `openhuman-core run --help`.
pub const RUN_HELP: &str = "\
Usage: openhuman run [--host <addr>] [--port <u16>] [--jsonrpc-only|--headless-api]
                     [--mode single-user|saas] [--saas-config <file>] [-v|--verbose]

  --host <addr>          Bind address (default: 127.0.0.1 or OPENHUMAN_CORE_HOST)
  --port <u16>           Listen address port (default: 7788 or OPENHUMAN_CORE_PORT)
  --jsonrpc-only         HTTP JSON-RPC only; disable Socket.IO
  --headless-api         HTTP JSON-RPC only; disable all background services
  --mode <mode>          single-user (default) or saas (or OPENHUMAN_MODE=saas)
  --saas-config <file>   The operator's SaaS config; required with --mode saas
  -v, --verbose          Shorthand for RUST_LOG=debug when RUST_LOG is unset

Logging: set RUST_LOG (e.g. RUST_LOG=debug openhuman run). Default level is info.";

/// Resolve the operating mode from `--mode`, `OPENHUMAN_MODE` and
/// `--saas-config`.
///
/// The environment can only raise the mode to SaaS, never lower it: a
/// deployment that sets `OPENHUMAN_MODE=saas` cannot be talked back into
/// single-user by a flag. SaaS needs an operator config, and a config without
/// SaaS is a mistake worth refusing rather than ignoring.
pub fn resolve_mode(
    flag: Option<&str>,
    env: Option<&str>,
    saas_config: Option<std::path::PathBuf>,
) -> anyhow::Result<(crate::core::runtime::Mode, Option<std::path::PathBuf>)> {
    use crate::core::runtime::Mode;
    let parse = |raw: &str, source: &str| {
        raw.parse::<Mode>()
            .map_err(|e| anyhow::anyhow!("{source}: {e}"))
    };
    let from_flag = flag.map(|raw| parse(raw, "--mode")).transpose()?;
    let from_env = env
        .filter(|raw| !raw.trim().is_empty())
        .map(|raw| parse(raw, "OPENHUMAN_MODE"))
        .transpose()?;
    let mode = if from_env == Some(Mode::Saas) {
        Mode::Saas
    } else {
        from_flag.unwrap_or_default()
    };
    match (mode, &saas_config) {
        (Mode::Saas, None) => anyhow::bail!("--mode saas needs --saas-config <file>"),
        (Mode::SingleUser, Some(_)) => {
            anyhow::bail!("--saas-config is only meaningful with --mode saas")
        }
        _ => Ok((mode, saas_config)),
    }
}

/// Starts a server for a [`ServeRequest`] and resolves when it stops.
pub type ServerLauncher =
    fn(ServeRequest) -> Pin<Box<dyn Future<Output = anyhow::Result<()>> + Send>>;

static LAUNCHER: OnceLock<ServerLauncher> = OnceLock::new();

/// Install the launcher `run` / `serve` use. The first call wins; later calls
/// are ignored, so every host path can install without coordinating.
pub fn install_server_launcher(launcher: ServerLauncher) {
    if LAUNCHER.set(launcher).is_err() {
        log::debug!("[cli] server launcher already installed; keeping the first");
    }
}

/// The installed launcher, if a host installed one.
pub fn installed_server_launcher() -> Option<ServerLauncher> {
    LAUNCHER.get().copied()
}

#[cfg(test)]
#[path = "server_launcher_tests.rs"]
mod tests;
