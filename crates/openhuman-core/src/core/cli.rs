//! Command-line interface for the OpenHuman core binary.
//!
//! This module handles argument parsing, subcommand dispatching, and help printing
//! for the CLI. It supports commands for running the server, making RPC calls,
//! and invoking domain-specific functionality across various namespaces.

use crate::core::runtime::DispatchCtx;
use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::config::schema::load::{set_current_dotenv_load, DotenvLoadProvenance};
use crate::core::all;
use crate::core::invoke::{default_state, invoke_method};
use crate::core::params::parse_json_params;
use crate::core::{ControllerSchema, TypeSchema};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[path = "cli/help.rs"]
mod help;
use help::{is_help, print_function_help, print_general_help, print_namespace_help};

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
    run_from_cli_args_with(args, None)
}

/// Whether the parsed invocation only asks the CLI to print help.
///
/// Hosts use this before startup work that may access user credentials so
/// `--help` remains available on a machine whose keyring is unavailable.
pub(crate) fn is_help_only(args: &[String]) -> bool {
    let Ok(launch) = parse_launch_options(args) else {
        return false;
    };
    let Some(command) = launch.args.first() else {
        return true;
    };
    if is_help(command) {
        return true;
    }
    let tail = &launch.args[1..];
    match command.as_str() {
        "call" | "agent" => tail.is_empty() || tail.iter().any(|arg| is_help(arg)),
        "mcp" | "mcp-server" | "run" | "serve" | "sentry-test" => {
            tail.iter().any(|arg| is_help(arg))
        }
        _namespace => {
            matches!(
                tail,
                [function, ..] if is_help(function)
            ) || matches!(tail, [_, slot, ..] if is_help(slot))
        }
    }
}

/// [`run_from_cli_args`] with a host-supplied boot description for `run` /
/// `serve`; see [`crate::run_core_from_args_with`].
pub fn run_from_cli_args_with(
    args: &[String],
    host_boot: Option<crate::core::server_launcher::HostBoot>,
) -> Result<()> {
    load_dotenv_for_cli()?;

    let launch = parse_launch_options(args)?;
    crate::config::set_cli_inference_overrides(launch.provider.as_deref(), launch.model.as_deref());

    let args = launch.args;
    // Print the welcome banner to stderr to keep stdout clean for JSON output.
    // `mcp`/`mcp-server` speak JSON-RPC on stdout. The historical `tui`/`chat`
    // names emit only their concise migration diagnostic, so suppress the
    // banner there as well.
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
        "run" | "serve" => super::cli_serve::run_server_command(&args[1..], host_boot),
        "mcp" | "mcp-server" => crate::mcp::server::run_stdio_from_cli(&args[1..]),
        // Keep the historical names as a migration diagnostic now that the
        // terminal frontend is its own workspace crate and executable.
        "tui" | "chat" => run_moved_tui_diagnostic(),
        "call" => run_call_command(&args[1..]),
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

/// Apply process-local provider/model overrides for a standalone frontend.
///
/// The values affect config loaded later in this process and are never written
/// back to the user's config file. This keeps independently packaged frontends
/// such as `openhuman-tui` compatible with the core CLI's launch flags.
#[doc(hidden)]
pub fn set_transient_inference_overrides(provider: Option<&str>, model: Option<&str>) {
    crate::config::set_cli_inference_overrides(provider, model);
}

#[derive(Debug, Default, PartialEq, Eq)]
struct CliLaunchOptions {
    args: Vec<String>,
    model: Option<String>,
    provider: Option<String>,
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

/// Parse the same leading model/provider flags as the CLI and return the
/// remaining command arguments without applying those transient overrides.
///
/// Hosts that route an inspection command around the normal CLI boot path use
/// this to preserve the launch-argument grammar while keeping the selected
/// inspection configuration unchanged.
pub fn parse_cli_args_after_launch_options(args: &[String]) -> Result<Vec<String>> {
    parse_launch_options(args).map(|options| options.args)
}

fn run_moved_tui_diagnostic() -> Result<()> {
    anyhow::bail!(
        "the terminal UI is now the separate `openhuman-tui` executable; \
         run `openhuman-tui --help` for usage"
    )
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
pub fn load_dotenv_for_cli() -> Result<()> {
    let previous_dotenv_load = crate::config::schema::load::current_dotenv_load();
    let process_vars = std::env::vars_os()
        .map(|(name, _)| name.to_string_lossy().into_owned())
        .collect::<HashSet<_>>();
    match std::env::var("OPENHUMAN_DOTENV_PATH") {
        Ok(path) if !path.trim().is_empty() => {
            let path = PathBuf::from(&path);
            dotenvy::from_path(&path).map_err(|e| {
                anyhow::anyhow!(
                    "failed to load dotenv from OPENHUMAN_DOTENV_PATH={}: {e}",
                    path.display()
                )
            })?;
            if let Ok(variables) = dotenv_variable_names(&path) {
                record_dotenv_load(
                    path,
                    variables,
                    &process_vars,
                    previous_dotenv_load.as_ref(),
                );
            }
        }
        _ => {
            if let Ok(path) = dotenvy::dotenv() {
                if let Ok(variables) = dotenv_variable_names(&path) {
                    record_dotenv_load(
                        path,
                        variables,
                        &process_vars,
                        previous_dotenv_load.as_ref(),
                    );
                }
            }
        }
    }
    Ok(())
}

fn dotenv_variable_names(path: &Path) -> Result<Vec<String>> {
    dotenvy::from_path_iter(path)?
        .map(|entry| entry.map(|(name, _)| name).map_err(anyhow::Error::from))
        .collect()
}

fn record_dotenv_load(
    path: PathBuf,
    variables: Vec<String>,
    process_vars: &HashSet<String>,
    previous: Option<&DotenvLoadProvenance>,
) {
    let mut load = DotenvLoadProvenance::from_variables(
        path,
        variables,
        process_vars.iter().map(String::as_str),
    );
    load.preserve_previous_origin(previous, process_vars);
    set_current_dotenv_load(Some(load));
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
    let mut params = None;
    let mut params_stdin = false;

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
                if params_stdin {
                    return Err(anyhow::anyhow!(
                        "--params and --params-stdin are mutually exclusive"
                    ));
                }
                params = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --params"))?
                        .clone(),
                );
                i += 2;
            }
            "--params-stdin" => {
                if params.is_some() {
                    return Err(anyhow::anyhow!(
                        "--params and --params-stdin are mutually exclusive"
                    ));
                }
                params_stdin = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!(
                    "Usage: openhuman call --method <name> [--params '<json>' | --params-stdin]"
                );
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown call arg: {other}")),
        }
    }

    let method = method.ok_or_else(|| anyhow::anyhow!("--method is required"))?;
    if params_stdin {
        let mut stdin_params = String::new();
        std::io::Read::read_to_string(&mut std::io::stdin(), &mut stdin_params)
            .map_err(|e| anyhow::anyhow!("failed to read --params-stdin: {e}"))?;
        params = Some(stdin_params);
    }
    let params =
        parse_json_params(params.as_deref().unwrap_or("{}")).map_err(anyhow::Error::msg)?;

    // `call` invokes a JSON-RPC method that may run an orchestrator turn
    // (e.g. `agent.chat`), so it needs the same roomy stack as the server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(crate::core::runtime::AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(crate::core::runtime::MAX_BLOCKING_THREADS)
        .build()?;
    let value = rt
        .block_on(async {
            let headless_agent_chat = crate::core::legacy_aliases::resolve_legacy(&method)
                == crate::agent_runs::INFERENCE_AGENT_CHAT;
            let dispatch = if headless_agent_chat {
                let config = crate::config::load_config_with_timeout()
                    .await
                    .map_err(|error| error.to_string())?;
                if crate::agent_registry::host_definition::AgentDefinitionRegistry::global()
                    .is_none()
                {
                    crate::agent_registry::host_definition::AgentDefinitionRegistry::init_global(
                        &config.workspace_dir,
                    )
                    .map_err(|error| error.to_string())?;
                }
                crate::security::credentials::seed_api_key_from_env(&config).await;

                // A one-shot CLI call does not boot the long-lived server
                // runtime, but the agent turn still needs a real owner context
                // for scoped persistence and tools that cross task boundaries.
                let core = match crate::core::runtime::CoreContext::default_context() {
                    Some(core) => core,
                    None => {
                        let tool_groups = if std::env::var_os("OPENHUMAN_E2E").is_some() {
                            crate::tools::toolpacks::ToolGroups::advertised()
                        } else {
                            crate::tools::toolpacks::ToolGroups::default()
                        };
                        crate::core::runtime::CoreContext::init_with_config(
                            crate::core::types::HostKind::detect_standalone(),
                            &crate::core::runtime::TokenSource::EnvOrFile,
                            crate::core::runtime::DomainSet::full(),
                            tool_groups,
                            Some(config),
                        )
                        .await
                        .map_err(|error| error.to_string())?
                        .0
                    }
                };
                DispatchCtx::new(core)
            } else {
                DispatchCtx::process_default()
            };
            let dispatch = if headless_agent_chat {
                dispatch.with_headless_agent_chat()
            } else {
                dispatch
            };
            invoke_method(dispatch, default_state(), &method, params).await
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
        .block_on(async {
            invoke_method(
                DispatchCtx::process_default(),
                default_state(),
                &method,
                Value::Object(params),
            )
            .await
        })
        .map_err(anyhow::Error::msg)?;

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Parses namespace-style command-line arguments into a JSON map based on a
/// function's schema, without dispatching the command or starting a runtime.
///
/// # Arguments
///
/// * `schema` - The schema defining expected inputs.
/// * `args` - The command-line arguments to parse.
///
/// # Errors
///
/// Returns an error if arguments are malformed, unknown, or fail validation.
pub fn parse_function_params(
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
        TypeSchema::BoundedU64 { min, max } => {
            let n = raw
                .parse::<u64>()
                .map_err(|e| format!("expected unsigned integer, got '{raw}': {e}"))?;
            // Name the crossed limit, matching `validate_params`.
            if n > *max {
                Err(format!("expected unsigned integer <= {max}, got '{raw}'"))
            } else if n < *min {
                Err(format!("expected unsigned integer >= {min}, got '{raw}'"))
            } else {
                Ok(Value::Number(n.into()))
            }
        }
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

#[cfg(test)]
#[path = "cli_tests.rs"]
mod tests;
