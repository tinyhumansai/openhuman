//! Command-line interface for the OpenHuman core binary.
//!
//! This module handles argument parsing, subcommand dispatching, and help printing
//! for the CLI. It supports commands for running the server, making RPC calls,
//! and invoking domain-specific functionality across various namespaces.

use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;
use std::io::IsTerminal;

use crate::core::all;
use crate::core::jsonrpc::{default_state, invoke_method, parse_json_params};
use crate::core::logging::CliLogDefault;
use crate::core::{ControllerSchema, TypeSchema};

/// The ASCII banner displayed when the CLI starts.
const CLI_BANNER: &str = r#"

 ▗▄▖ ▄▄▄▄  ▗▞▀▚▖▄▄▄▄  ▗▖ ▗▖█  ▐▌▄▄▄▄  ▗▞▀▜▌▄▄▄▄
▐▌ ▐▌█   █ ▐▛▀▀▘█   █ ▐▌ ▐▌▀▄▄▞▘█ █ █ ▝▚▄▟▌█   █
▐▌ ▐▌█▄▄▄▀ ▝▚▄▄▖█   █ ▐▛▀▜▌     █   █      █   █
▝▚▄▞▘█                ▐▌ ▐▌
     ▀

Contribute & Star us on GitHub: https://github.com/tinyhumansai/openhuman

"#;

/// Dispatches CLI commands based on arguments.
///
/// This is the entry point for CLI argument handling. It performs the following:
/// 1. Prints the ASCII welcome banner to stderr.
/// 2. Resolves and groups available controller schemas.
/// 3. Checks for global help requests.
/// 4. Matches the first argument to a subcommand or a domain namespace.
///
/// # Arguments
///
/// * `args` - A slice of strings containing the command-line arguments.
///
/// # Errors
///
/// Returns an error if the command fails, parameters are invalid, or if
/// the subcommand/namespace is unknown.
pub fn run_from_cli_args(args: &[String]) -> Result<()> {
    load_dotenv_for_cli()?;

    let launch = parse_launch_options(args)?;
    crate::openhuman::config::set_cli_inference_overrides(
        launch.provider.as_deref(),
        launch.model.as_deref(),
    );

    let host = crate::core::types::HostKind::detect_standalone();
    if launch.args == ["--tui"]
        || should_auto_launch_tui(
            &launch.args,
            std::io::stdin().is_terminal(),
            std::io::stdout().is_terminal(),
            host,
            cfg!(feature = "tui"),
        ) && !launch.no_tui
    {
        return run_tui_from_cli(&[]);
    }

    let args = launch.args;
    // Print the welcome banner to stderr to keep stdout clean for JSON output.
    // `mcp`/`mcp-server` speak JSON-RPC on stdout; `tui`/`chat` own the whole
    // terminal (alternate screen + raw mode) — a banner on either would corrupt
    // the stream / the UI, so both suppress it. The `matches!` is on the raw
    // string, so it stays valid even when the `tui` feature is compiled out.
    if !matches!(
        args.first().map(String::as_str),
        Some("mcp" | "mcp-server" | "tui" | "chat")
    ) {
        eprint!("{CLI_BANNER}");
    }

    let grouped = grouped_schemas();
    if args.is_empty() || is_help(&args[0]) {
        print_general_help(&grouped);
        return Ok(());
    }

    // Match on the first argument to determine the subcommand.
    match args[0].as_str() {
        "run" | "serve" => run_server_command(&args[1..]),
        "mcp" | "mcp-server" => crate::openhuman::mcp::server::run_stdio_from_cli(&args[1..]),
        // Keep the command present in slim builds so users get a build-fact
        // diagnostic rather than a misleading "unknown namespace" error.
        "tui" | "chat" => run_tui_from_cli(&args[1..]),
        "call" => run_call_command(&args[1..]),
        "tree-summarizer" => {
            crate::openhuman::memory::tree::tree_runtime::cli::run_tree_summarizer_command(
                &args[1..],
            )
        }
        "memory" => crate::core::memory_cli::run_memory_command(&args[1..]),
        "agent" => {
            log::debug!(
                "[cli] dispatching to agent subcommand, args={:?}",
                &args[1..]
            );
            crate::core::agent_cli::run_agent_command(&args[1..])
        }
        "sentry-test" => run_sentry_test_command(&args[1..]),
        // Generic namespace dispatcher: `openhuman <namespace> <function> ...`
        namespace => run_namespace_command(namespace, &args[1..], &grouped),
    }
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CliLaunchOptions {
    args: Vec<String>,
    model: Option<String>,
    provider: Option<String>,
    no_tui: bool,
}

/// Parse launch-wide flags before the subcommand, matching the familiar
/// `openhuman [global options] [command]` shape. Model/provider values are
/// transient process overrides; they never rewrite the user's config file.
fn parse_launch_options(args: &[String]) -> Result<CliLaunchOptions> {
    let mut parsed = CliLaunchOptions::default();
    let mut i = 0usize;

    while i < args.len() {
        let arg = args[i].as_str();
        let (target, inline_value) = match arg {
            "--model" | "--model-id" | "-m" => (Some("model"), None),
            "--provider" | "--provider-id" | "-p" => (Some("provider"), None),
            "--no-tui" => {
                parsed.no_tui = true;
                i += 1;
                continue;
            }
            _ if arg.starts_with("--model=") => (Some("model"), arg.split_once('=').map(|v| v.1)),
            _ if arg.starts_with("--model-id=") => {
                (Some("model"), arg.split_once('=').map(|v| v.1))
            }
            _ if arg.starts_with("--provider=") => {
                (Some("provider"), arg.split_once('=').map(|v| v.1))
            }
            _ if arg.starts_with("--provider-id=") => {
                (Some("provider"), arg.split_once('=').map(|v| v.1))
            }
            _ => break,
        };

        let value = match inline_value {
            Some(value) => value,
            None => {
                i += 1;
                let value = args
                    .get(i)
                    .map(String::as_str)
                    .ok_or_else(|| anyhow::anyhow!("missing value for {arg}"))?;
                if value.starts_with('-') {
                    return Err(anyhow::anyhow!("missing value for {arg}"));
                }
                value
            }
        };
        let value = value.trim();
        if value.is_empty() {
            return Err(anyhow::anyhow!("empty value for {arg}"));
        }
        match target {
            Some("model") => parsed.model = Some(value.to_string()),
            Some("provider") => parsed.provider = Some(value.to_string()),
            _ => unreachable!("launch option target is fixed above"),
        }
        i += 1;
    }

    parsed.args = args[i..].to_vec();
    Ok(parsed)
}

#[cfg(feature = "tui")]
fn run_tui_from_cli(args: &[String]) -> Result<()> {
    crate::tui::run_from_cli(args)
}

#[cfg(not(feature = "tui"))]
fn run_tui_from_cli(_args: &[String]) -> Result<()> {
    anyhow::bail!(
        "tui feature disabled at compile time; rebuild with `--features tui` \
         (or use a default-feature build) to enable `openhuman tui`"
    )
}

/// Pure launch policy for the bare `openhuman` command. Explicit subcommands
/// are never rewritten. Docker and redirected/CI sessions keep the headless
/// CLI behavior; `openhuman tui` remains an explicit override everywhere.
fn should_auto_launch_tui(
    args: &[String],
    stdin_is_terminal: bool,
    stdout_is_terminal: bool,
    host: crate::core::types::HostKind,
    tui_compiled: bool,
) -> bool {
    args.is_empty()
        && stdin_is_terminal
        && stdout_is_terminal
        && host == crate::core::types::HostKind::Cli
        && tui_compiled
}

/// Handles the `sentry-test` subcommand used to verify Sentry wiring end-to-end.
///
/// Captures an Error-level event against the currently initialized Sentry
/// client (see `sentry::init` in the binary entry point), flushes the client,
/// and prints the event UUID to stdout. Optional `--panic` flag additionally
/// triggers a panic so the panic integration is exercised too.
///
/// Requires a DSN resolvable at runtime — either via the
/// `OPENHUMAN_CORE_SENTRY_DSN` env var (or the legacy `OPENHUMAN_SENTRY_DSN`
/// alias) or baked into the binary at build time via `option_env!`. Absent a
/// DSN, the command exits non-zero with a diagnostic instead of silently
/// producing no telemetry.
///
/// Only compiled with the `crash-reporting` feature; the `#[cfg(not(...))]`
/// companion below returns a disabled-build error (mirrors the `mcp` CLI
/// precedent, where the subcommand arm + top-level help stay compiled and the
/// handler reports the build fact rather than a bogus "unknown command").
#[cfg(feature = "crash-reporting")]
fn run_sentry_test_command(args: &[String]) -> Result<()> {
    let mut message: Option<String> = None;
    let mut do_panic = false;
    let mut i = 0usize;

    while i < args.len() {
        match args[i].as_str() {
            "--message" => {
                message = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --message"))?
                        .clone(),
                );
                i += 2;
            }
            "--panic" => {
                do_panic = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman sentry-test [--message <text>] [--panic]");
                println!();
                println!("  --message <text>  Body of the Error-level event sent to Sentry");
                println!("                    (default: \"openhuman sentry-test ping\")");
                println!("  --panic           After capturing the event, trigger a panic so the");
                println!("                    panic integration reports it as a separate event.");
                println!();
                println!(
                    "Requires OPENHUMAN_CORE_SENTRY_DSN (or the legacy OPENHUMAN_SENTRY_DSN alias)"
                );
                println!("at runtime, or baked into the binary at build time via option_env!. On");
                println!("success, prints the event UUID to stdout.");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown sentry-test arg: {other}")),
        }
    }

    let client = sentry::Hub::current().client();
    let dsn_host = client
        .as_deref()
        .and_then(|c| c.dsn())
        .map(|d| d.host().to_string());

    match &dsn_host {
        Some(host) => eprintln!("[sentry-test] Sentry client active (dsn host: {host})"),
        None => {
            return Err(anyhow::anyhow!(
                "Sentry is not initialized in this binary — no DSN is resolvable. \
                 Set OPENHUMAN_CORE_SENTRY_DSN (or the legacy OPENHUMAN_SENTRY_DSN alias) \
                 in the environment (or rebuild with it defined at compile time) and try again."
            ));
        }
    }

    let msg = message.unwrap_or_else(|| "openhuman sentry-test ping".to_string());

    sentry::configure_scope(|scope| {
        scope.set_tag("test", "true");
        scope.set_tag("source", "sentry-test-cli");
    });

    let event_id = sentry::capture_message(&msg, sentry::Level::Error);

    if let Some(c) = client {
        if !c.flush(Some(std::time::Duration::from_secs(5))) {
            eprintln!(
                "[sentry-test] WARNING: flush timed out after 5s — event may not have reached Sentry."
            );
        }
    }

    println!("{event_id}");

    if do_panic {
        eprintln!(
            "[sentry-test] Triggering panic as requested — the panic integration should capture it."
        );
        panic!("openhuman sentry-test intentional panic");
    }

    Ok(())
}

/// Disabled-build stand-in for [`run_sentry_test_command`]. Same signature as
/// the `crash-reporting` version; reports that the probe is unavailable in a
/// build compiled without the feature rather than pretending to succeed.
#[cfg(not(feature = "crash-reporting"))]
fn run_sentry_test_command(_args: &[String]) -> Result<()> {
    Err(anyhow::anyhow!(
        "sentry-test unavailable: built without the crash-reporting feature — \
         rebuild with `--features crash-reporting`"
    ))
}

/// Loads key/value pairs from a `.env` file into the process environment.
///
/// This is used for all CLI entrypoints so direct namespace commands pick up
/// the same repo-local configuration as `run` / `serve`.
///
/// Precedence:
/// 1. Variables already set in the process environment are **not** overwritten.
/// 2. If `OPENHUMAN_DOTENV_PATH` is set, that file is loaded.
/// 3. Otherwise, it searches for `.env` in the current working directory.
pub(crate) fn load_dotenv_for_cli() -> Result<()> {
    match std::env::var("OPENHUMAN_DOTENV_PATH") {
        Ok(path) if !path.trim().is_empty() => {
            dotenvy::from_path(&path).map_err(|e| {
                anyhow::anyhow!("failed to load dotenv from OPENHUMAN_DOTENV_PATH={path}: {e}")
            })?;
        }
        _ => {
            let _ = dotenvy::dotenv();
        }
    }
    Ok(())
}

/// Handles the `run` subcommand to start the core HTTP/JSON-RPC server.
///
/// This command boots the main application server, including its JSON-RPC
/// endpoint, Socket.IO bridge, and background services (voice, vision, etc.).
///
/// # Arguments
///
/// * `args` - Command-line arguments for the `run` command (e.g., `--port`).
fn run_server_command(args: &[String]) -> Result<()> {
    let mut port: Option<u16> = None;
    let mut host: Option<String> = None;
    let mut socketio_enabled = true;
    let mut headless_api = false;
    let mut verbose = false;
    let log_scope = CliLogDefault::Global;
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
            "-h" | "--help" => {
                println!("Usage: openhuman run [--host <addr>] [--port <u16>] [--jsonrpc-only|--headless-api] [-v|--verbose]");
                println!();
                println!(
                    "  --host <addr>    Bind address (default: 127.0.0.1 or OPENHUMAN_CORE_HOST)"
                );
                println!(
                    "  --port <u16>     Listen address port (default: 7788 or OPENHUMAN_CORE_PORT)"
                );
                println!("  --jsonrpc-only   HTTP JSON-RPC only; disable Socket.IO");
                println!("  --headless-api   HTTP JSON-RPC only; disable all background services");
                println!("  -v, --verbose    Shorthand for RUST_LOG=debug when RUST_LOG is unset");
                println!();
                println!("Logging: set RUST_LOG (e.g. RUST_LOG=debug openhuman run). Default level is info.");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown run arg: {other}")),
        }
    }

    crate::core::logging::init_for_cli_run(verbose, log_scope);

    // Initialize the Tokio multi-threaded runtime.
    //
    // A single agent turn is a very large async state machine (system prompt +
    // hundreds of tool specs + the nested provider/tool loop), and delegating
    // to a sub-agent runs another full turn one level down. Even with the inner
    // sub-agent future boxed (`subagent_runner::ops`), that nesting overflows
    // tokio's default 2 MiB worker-thread stack and aborts the whole process
    // (SIGABRT: "thread 'tokio-rt-worker' has overflowed its stack"), taking
    // the JSON-RPC server down mid-request. Give workers a roomier stack.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(crate::core::runtime::MAX_BLOCKING_THREADS)
        .build()?;
    rt.block_on(async {
        if headless_api {
            crate::core::jsonrpc::run_server_headless(host.as_deref(), port).await
        } else {
            crate::core::jsonrpc::run_server(host.as_deref(), port, socketio_enabled).await
        }
    })?;
    Ok(())
}

/// Handles the `call` subcommand to invoke a JSON-RPC method directly from the CLI.
///
/// This is used for one-off commands and debugging, bypassing the HTTP transport
/// and calling the internal `invoke_method` directly.
///
/// # Arguments
///
/// * `args` - Command-line arguments specifying the method and parameters.
fn run_call_command(args: &[String]) -> Result<()> {
    let mut method: Option<String> = None;
    let mut params = "{}".to_string();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--method" => {
                method = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --method"))?
                        .clone(),
                );
                i += 2;
            }
            "--params" => {
                params = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --params"))?
                    .clone();
                i += 2;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman call --method <name> [--params '<json>']");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown call arg: {other}")),
        }
    }

    let method = method.ok_or_else(|| anyhow::anyhow!("--method is required"))?;
    let params = parse_json_params(&params).map_err(anyhow::Error::msg)?;

    // Raw calls bypass namespace parsing, but not the configured memory-driver
    // binding. Without this gate an absent capability could still reach a
    // destructive embedded handler because plain CLI invocations have no
    // ambient CoreContext to filter the registry.
    crate::core::cli_capability::ensure_capability_blocking(
        all::capability_for_rpc_method(&method).flatten(),
        &format!("openhuman call --method {method}"),
    )?;

    // `call` invokes a JSON-RPC method that may run an orchestrator turn
    // (e.g. `agent.chat`), so it needs the same roomy stack as the server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(crate::core::runtime::MAX_BLOCKING_THREADS)
        .build()?;
    let value = rt
        .block_on(async {
            // A raw call is its own process: the server publishes the module
            // host policy during boot, and a method that crosses the memory
            // module binding (since the round-2 migration, most of them)
            // fails with "the module host policy was never published"
            // without this — the same per-process publish the memory and
            // tree-summarizer subcommand families already do.
            // One line by design: `src/core/` is transport, and the load-
            // config-install-sink-publish-policy sequence lives with the
            // module host in the openhuman layer (review finding on #5932).
            #[cfg(feature = "modules")]
            crate::openhuman::modules::memory::publish_cli_boot_policy().await?;
            invoke_method(default_state(), &method, params).await
        })
        .map_err(anyhow::Error::msg)?;

    // Output the result as pretty-printed JSON to stdout.
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Dispatches commands that fall under a specific namespace (e.g., `openhuman <namespace> <function>`).
///
/// It looks up the function schema for validation and executes the request.
///
/// # Arguments
///
/// * `namespace` - The namespace for the command.
/// * `args` - Arguments for the function within the namespace.
/// * `grouped` - A map of available schemas grouped by namespace.
fn run_namespace_command(
    namespace: &str,
    args: &[String],
    grouped: &BTreeMap<String, Vec<ControllerSchema>>,
) -> Result<()> {
    let Some(schemas) = grouped.get(namespace) else {
        // Reachable only when `grouped` really was filtered — i.e. under
        // `run`/`serve`/TUI, which build a `CoreContext`. On a plain CLI
        // invocation there is no ambient context, so nothing is filtered and a
        // gated namespace is still present; the per-function gate below is what
        // fires there. Consult the UNFILTERED registry before reporting a typo:
        // silence reads as a mistyped command and sends the user off debugging
        // their own command line, which is exactly what `docs/specs/kernel.md`
        // §3.3 carves the CLI out of. Same reasoning as the retained `mcp` and
        // `tui` arms above. A namespace that does not exist at all yields `None`
        // and still reports unknown.
        crate::core::cli_capability::ensure_capability_blocking(
            all::sole_capability_for_namespace(namespace),
            &format!("openhuman {namespace}"),
        )?;
        return Err(anyhow::anyhow!(
            "unknown namespace '{namespace}'. Run `openhuman --help` to see available namespaces."
        ));
    };

    if args.is_empty() || is_help(&args[0]) {
        // If there's a domain-specific CLI handler for this namespace, use it as the default.
        if let Some(cli_handler) = all::cli_handler_for_namespace(namespace) {
            return cli_handler(args);
        }
        print_namespace_help(namespace, schemas);
        return Ok(());
    }

    let function = args[0].as_str();

    // Gate BEFORE resolving the schema, not in the not-found arm below.
    //
    // `grouped` comes from `all_controller_schemas()`, which filters through the
    // ambient `CoreContext` — and no plain CLI subcommand builds one, since
    // `DEFAULT_CONTEXT` is set only in `CoreContext::init` (reached by
    // `run`/`serve` and the TUI). So on a real `openhuman <ns> <fn>` invocation
    // *nothing* is filtered, a gated function is still found here, and a check
    // placed only in the not-found arm would never execute — the command would
    // simply run. Gating the resolved function instead makes this fire on the
    // path users actually take, and it stays correct under `run`/`serve` where
    // `grouped` genuinely is filtered.
    //
    // `capability_for_parts` consults the UNFILTERED registry and yields `None`
    // for a function registered nowhere, so a genuine typo short-circuits the
    // gate and falls through to the unknown-function message below. Keeping the
    // two distinguishable is the point: collapsing them would make real typos
    // harder to diagnose, which is the failure `docs/specs/kernel.md` §3.3
    // carves the CLI out of.
    crate::core::cli_capability::ensure_capability_blocking(
        all::capability_for_parts(namespace, function).flatten(),
        &format!("openhuman {namespace} {function}"),
    )?;

    let Some(schema) = schemas.iter().find(|s| s.function == function).cloned() else {
        return Err(anyhow::anyhow!(
            "unknown function '{namespace} {function}'. Run `openhuman {namespace} --help`."
        ));
    };

    if args.len() > 1 && is_help(&args[1]) {
        print_function_help(namespace, &schema);
        return Ok(());
    }

    // Generic parameter parsing and validation based on schema.
    let params = parse_function_params(&schema, &args[1..]).map_err(anyhow::Error::msg)?;
    let method = all::rpc_method_from_parts(namespace, function)
        .ok_or_else(|| anyhow::anyhow!("unregistered controller '{namespace}.{function}'"))?;

    // Same as the explicit `call` path above — any registered controller may
    // ultimately drive an orchestrator turn.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(crate::core::runtime::MAX_BLOCKING_THREADS)
        .build()?;
    let value = rt
        .block_on(async { invoke_method(default_state(), &method, Value::Object(params)).await })
        .map_err(anyhow::Error::msg)?;

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Parses command-line arguments into a JSON map based on a function's schema.
///
/// # Arguments
///
/// * `schema` - The schema defining expected inputs.
/// * `args` - The command-line arguments to parse.
///
/// # Errors
///
/// Returns an error if arguments are malformed, unknown, or fail validation.
fn parse_function_params(
    schema: &ControllerSchema,
    args: &[String],
) -> Result<Map<String, Value>, String> {
    let mut out = Map::new();
    let mut i = 0usize;

    while i < args.len() {
        let raw = &args[i];
        if !raw.starts_with("--") {
            return Err(format!("invalid arg '{raw}', expected --<param> <value>"));
        }
        let key = raw.trim_start_matches("--").replace('-', "_");
        let Some(spec) = schema.inputs.iter().find(|input| input.name == key) else {
            return Err(format!(
                "unknown param '{key}' for {}.{}",
                schema.namespace, schema.function
            ));
        };
        let raw_value = args
            .get(i + 1)
            .ok_or_else(|| format!("missing value for --{key}"))?;
        if raw_value.starts_with("--") {
            let next_key = raw_value.trim_start_matches("--").replace('-', "_");
            if schema.inputs.iter().any(|input| input.name == next_key) {
                return Err(format!("missing value for --{key}"));
            }
        }
        let value = parse_input_value(&spec.ty, raw_value)?;
        out.insert(key, value);
        i += 2;
    }

    all::validate_params(schema, &out)?;
    Ok(out)
}

/// Parses a raw string value into a JSON `Value` based on the target `TypeSchema`.
///
/// Supports basic types like string, bool, and numbers, as well as complex JSON
/// structures for advanced types.
///
/// # Arguments
///
/// * `ty` - The expected type schema.
/// * `raw` - The raw string value from the command line.
fn parse_input_value(ty: &TypeSchema, raw: &str) -> Result<Value, String> {
    match ty {
        TypeSchema::String => Ok(Value::String(raw.to_string())),
        TypeSchema::Bool => raw
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|e| format!("expected bool, got '{raw}': {e}")),
        TypeSchema::I64 => raw
            .parse::<i64>()
            .map(|n| Value::Number(n.into()))
            .map_err(|e| format!("expected i64, got '{raw}': {e}")),
        TypeSchema::U64 => raw
            .parse::<u64>()
            .map(|n| Value::Number(n.into()))
            .map_err(|e| format!("expected u64, got '{raw}': {e}")),
        TypeSchema::F64 => {
            let n = raw
                .parse::<f64>()
                .map_err(|e| format!("expected f64, got '{raw}': {e}"))?;
            serde_json::Number::from_f64(n)
                .map(Value::Number)
                .ok_or_else(|| format!("invalid f64 '{raw}'"))
        }
        TypeSchema::Option(inner) => parse_input_value(inner, raw),
        TypeSchema::Enum { .. } => Ok(Value::String(raw.to_string())),
        TypeSchema::Json
        | TypeSchema::Array(_)
        | TypeSchema::Map(_)
        | TypeSchema::Object { .. }
        | TypeSchema::Ref(_)
        | TypeSchema::Bytes => parse_json_params(raw),
    }
}

/// Aggregates all registered controller schemas and groups them by namespace.
fn grouped_schemas() -> BTreeMap<String, Vec<ControllerSchema>> {
    let mut grouped: BTreeMap<String, Vec<ControllerSchema>> = BTreeMap::new();
    for schema in all::all_controller_schemas() {
        grouped
            .entry(schema.namespace.to_string())
            .or_default()
            .push(schema);
    }
    // Sort functions within each namespace for consistent help output.
    for schemas in grouped.values_mut() {
        schemas.sort_by_key(|s| s.function);
    }
    grouped
}

/// Prints the general help message listing available commands and namespaces.
fn print_general_help(grouped: &BTreeMap<String, Vec<ControllerSchema>>) {
    println!("OpenHuman core CLI\n");
    println!("Usage:");
    println!("  openhuman [OPTIONS]                     (tabbed terminal UI on interactive hosts)");
    println!("  openhuman run [--host <addr>] [--port <u16>] [--jsonrpc-only] [--verbose]");
    println!("  openhuman call --method <name> [--params '<json>']");
    println!(
        "  openhuman mcp [-v|--verbose]              (stdio MCP server; read-only memory tools)"
    );
    println!(
        "  openhuman tui [--thread <id>|--new]        (force tabbed terminal UI, alias: chat)"
    );
    println!("  openhuman skills <subcommand> [options]   (skill development runtime)");
    println!("  openhuman agent <subcommand> [options]    (inspect agent definitions & prompts)");
    println!("  openhuman voice [--hotkey <combo>] [--mode <tap|push>]  (voice dictation server)");
    println!("  openhuman tree-summarizer <subcommand> [options]  (summary tree CLI)");
    println!("  openhuman sentry-test [--message <text>] [--panic]  (verify Sentry wiring)");
    println!("  openhuman <namespace> <function> [--param value ...]\n");
    println!("Global options (place before the command):");
    println!("  -m, --model <id>       Override the model for this CLI session");
    println!("  -p, --provider <id>    Override the provider id or slug for this CLI session");
    println!("      --no-tui           Do not auto-open the terminal UI");
    println!("                        (aliases: --model-id, --provider-id)\n");
    println!("Available namespaces:");
    for namespace in grouped.keys() {
        let description = all::namespace_description(namespace.as_str())
            .unwrap_or("No namespace description available.");
        println!("  {namespace} - {description}");
    }
    println!("\nUse `openhuman <namespace> --help` to see functions.");
}

/// Prints help for a specific namespace, listing its functions.
fn print_namespace_help(namespace: &str, schemas: &[ControllerSchema]) {
    println!("Namespace: {namespace}\n");
    if let Some(description) = all::namespace_description(namespace) {
        println!("{description}\n");
    }
    println!("Functions:");
    for schema in schemas {
        println!("  {} - {}", schema.function, schema.description);
    }
    println!("\nUse `openhuman {namespace} <function> --help` for parameters.");
}

/// Prints detailed help for a specific function, including its parameters and description.
fn print_function_help(namespace: &str, schema: &ControllerSchema) {
    println!("{} {}\n", namespace, schema.function);
    println!("{}", schema.description);
    println!("\nParameters:");
    if schema.inputs.is_empty() {
        println!("  none");
    } else {
        for input in &schema.inputs {
            let required = if input.required {
                "required"
            } else {
                "optional"
            };
            println!("  --{} ({}) - {}", input.name, required, input.comment);
        }
    }
}

/// Checks if a string represents a help flag.
fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
