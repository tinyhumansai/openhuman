//! The `run` / `serve` subcommand: flag parsing and the hand-off to the
//! installed server launcher. Split from `cli.rs` to keep that file under its
//! line cap.

use anyhow::Result;

use crate::core::logging::CliLogDefault;

/// Handles the `run` subcommand to start the core HTTP/JSON-RPC server.
///
/// This command boots the main application server, including its JSON-RPC
/// endpoint, Socket.IO bridge, and background services (voice, vision, etc.).
///
/// # Arguments
///
/// * `args` - Command-line arguments for the `run` command (e.g., `--port`).
pub(super) fn run_server_command(
    args: &[String],
    host_boot: Option<crate::core::server_launcher::HostBoot>,
) -> Result<()> {
    let Some((request, verbose)) = parse_serve_args(
        args,
        std::env::var("OPENHUMAN_MODE").ok().as_deref(),
        host_boot,
    )?
    else {
        return Ok(());
    };
    let log_scope = CliLogDefault::Global;
    crate::core::logging::init_for_cli_run(verbose, log_scope);
    launch_server(request)
}

/// Parse the `run` / `serve` flags into the [`ServeRequest`] the launcher
/// receives, plus the `--verbose` switch. `None` means `--help` was printed.
///
/// Only flags the operator typed set a request field; everything else stays
/// unset so the launcher's builder (see `ServeRequest::host_boot`) keeps its
/// own value.
pub(super) fn parse_serve_args(
    args: &[String],
    env_mode: Option<&str>,
    host_boot: Option<crate::core::server_launcher::HostBoot>,
) -> Result<Option<(crate::core::server_launcher::ServeRequest, bool)>> {
    let mut port: Option<u16> = None;
    let mut host: Option<String> = None;
    let mut socketio_enabled = true;
    let mut headless_api = false;
    let mut mode_flag: Option<String> = None;
    let mut saas_config: Option<std::path::PathBuf> = None;
    let mut verbose = false;
    let mut i = 0usize;

    // Manual argument parsing loop for specific flags.
    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --port"))?;
                port = Some(
                    raw.parse::<u16>()
                        .map_err(|e| anyhow::anyhow!("invalid --port: {e}"))?,
                );
                i += 2;
            }
            "--host" => {
                host = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --host"))?
                        .clone(),
                );
                i += 2;
            }
            "--jsonrpc-only" => {
                socketio_enabled = false;
                i += 1;
            }
            "--headless-api" => {
                socketio_enabled = false;
                headless_api = true;
                i += 1;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "--mode" | "--saas-config" => {
                let value = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for {}", args[i]))?
                    .clone();
                if args[i] == "--mode" {
                    mode_flag = Some(value);
                } else {
                    saas_config = Some(value.into());
                }
                i += 2;
            }
            "-h" | "--help" => {
                println!("{}", crate::core::server_launcher::RUN_HELP);
                return Ok(None);
            }
            other => return Err(anyhow::anyhow!("unknown run arg: {other}")),
        }
    }

    let (mode, saas_config) =
        crate::core::server_launcher::resolve_mode(mode_flag.as_deref(), env_mode, saas_config)?;
    Ok(Some((
        crate::core::server_launcher::ServeRequest {
            host,
            port,
            socketio_enabled,
            headless_api,
            mode,
            saas_config,
            host_boot,
        },
        verbose,
    )))
}

/// Start the installed server launcher for `request` on a fresh tokio runtime.
fn launch_server(request: crate::core::server_launcher::ServeRequest) -> Result<()> {
    // Initialize the Tokio multi-threaded runtime.
    //
    // A single agent turn is a very large async state machine (system prompt +
    // hundreds of tool specs + the nested provider/tool loop), and delegating
    // to a sub-agent runs another full turn one level down. Even with the inner
    // sub-agent future boxed (`subagent_host::ops`), that nesting overflows
    // tokio's default 2 MiB worker-thread stack and aborts the whole process
    // (SIGABRT: "thread 'tokio-rt-worker' has overflowed its stack"), taking
    // the JSON-RPC server down mid-request. Give workers a roomier stack.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(crate::core::runtime::MAX_BLOCKING_THREADS)
        .build()?;
    let launcher = crate::core::server_launcher::installed_server_launcher().ok_or_else(|| {
        anyhow::anyhow!(
            "this binary has no JSON-RPC server linked in; the host must boot \
             through openhuman_rpc::host::cli or install a server launcher before \
             run_core_from_args"
        )
    })?;
    rt.block_on(launcher(request))?;
    Ok(())
}

#[cfg(test)]
#[path = "cli_serve_tests.rs"]
mod tests;
