//! Command-line interface for the OpenHuman core binary.
//!
//! This module handles argument parsing, subcommand dispatching, and help printing
//! for the CLI. It supports commands for running the server, making RPC calls,
//! starting a REPL, and invoking domain-specific functionality across various namespaces.

use anyhow::Result;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

use crate::core::all;
use crate::core::jsonrpc::{default_state, invoke_method, parse_json_params};
use crate::core::{ControllerSchema, TypeSchema};
use crate::openhuman::autocomplete::ops::{autocomplete_start_cli, AutocompleteStartCliOptions};

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
/// This is the entry point for CLI argument handling. It prints the banner,
/// checks for help requests, and dispatches to specific command handlers
/// like `run`, `call`, `repl`, `skills`, or namespace-based commands.
///
/// # Arguments
///
/// * `args` - A slice of strings containing the command-line arguments (excluding the binary name).
///
/// # Errors
///
/// Returns an error if the command fails or if an unknown command is provided.
pub fn run_from_cli_args(args: &[String]) -> Result<()> {
    // Print the welcome banner to stderr to keep stdout clean for JSON output.
    eprint!("{CLI_BANNER}");

    let grouped = grouped_schemas();
    if args.is_empty() || is_help(&args[0]) {
        print_general_help(&grouped);
        return Ok(());
    }

    // Match on the first argument to determine the subcommand.
    match args[0].as_str() {
        "run" | "serve" => run_server_command(&args[1..]),
        "call" => run_call_command(&args[1..]),
        "repl" | "shell" => crate::core::repl::run_repl(&args[1..]),
        "skills" => crate::core::skills_cli::run_skills_command(&args[1..]),
        "screen-intelligence" => {
            crate::core::screen_intelligence_cli::run_screen_intelligence_command(&args[1..])
        }
        "voice" | "dictate" => run_voice_server_command(&args[1..]),
        "text-input" => crate::core::text_input_cli::run_text_input_command(&args[1..]),
        namespace => run_namespace_command(namespace, &args[1..], &grouped),
    }
}

/// Handles the `run` subcommand to start the core HTTP/JSON-RPC server.
///
/// Parses flags for port, host, and optional Socket.IO support.
///
/// # Arguments
///
/// * `args` - Command-line arguments for the `run` command.
fn run_server_command(args: &[String]) -> Result<()> {
    let mut port: Option<u16> = None;
    let mut host: Option<String> = None;
    let mut socketio_enabled = true;
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
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman run [--host <addr>] [--port <u16>] [--jsonrpc-only] [-v|--verbose]");
                println!();
                println!(
                    "  --host <addr>    Bind address (default: 127.0.0.1 or OPENHUMAN_CORE_HOST)"
                );
                println!(
                    "  --port <u16>     Listen address port (default: 7788 or OPENHUMAN_CORE_PORT)"
                );
                println!("  --jsonrpc-only   HTTP JSON-RPC only; disable Socket.IO");
                println!("  -v, --verbose    Shorthand for RUST_LOG=debug when RUST_LOG is unset");
                println!();
                println!("Logging: set RUST_LOG (e.g. RUST_LOG=debug openhuman run). Default level is info.");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown run arg: {other}")),
        }
    }

    crate::core::logging::init_for_cli_run(verbose);

    // Initialize the Tokio runtime and start the server.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        crate::core::jsonrpc::run_server(host.as_deref(), port, socketio_enabled).await
    })?;
    Ok(())
}

/// Handles the `call` subcommand to invoke a JSON-RPC method directly from the CLI.
///
/// Useful for testing and automation.
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

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let value = rt
        .block_on(async { invoke_method(default_state(), &method, params).await })
        .map_err(anyhow::Error::msg)?;

    // Output the result as pretty-printed JSON.
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Handles the `voice` subcommand to run the standalone voice dictation server.
///
/// Listens for a hotkey, records audio, transcribes via whisper, and inserts
/// the result into the active text field.

fn run_voice_server_command(args: &[String]) -> Result<()> {
    use crate::openhuman::voice::hotkey::ActivationMode;
    use crate::openhuman::voice::server::{run_standalone, VoiceServerConfig};

    let mut hotkey: Option<String> = None;
    let mut mode: Option<String> = None;
    let mut skip_cleanup = false;
    let mut verbose = false;
    let mut i = 0usize;

    while i < args.len() {
        match args[i].as_str() {
            "--hotkey" => {
                hotkey = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --hotkey"))?
                        .clone(),
                );
                i += 2;
            }
            "--mode" => {
                mode = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --mode"))?
                        .clone(),
                );
                i += 2;
            }
            "--skip-cleanup" => {
                skip_cleanup = true;
                i += 1;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                println!("Usage: openhuman voice [--hotkey <combo>] [--mode <tap|push>] [--skip-cleanup] [-v]");
                println!();
                println!("  --hotkey <combo>   Key combination (default: fn)");
                println!(
                    "  --mode <tap|push>  Activation: tap to toggle, push to hold (default: push)"
                );
                println!("  --skip-cleanup     Skip LLM post-processing on transcriptions");
                println!("  -v, --verbose      Enable debug logging");
                println!();
                println!("Standalone voice dictation server. Press the hotkey to dictate,");
                println!("transcribed text is inserted into the active text field.");
                return Ok(());
            }
            other => return Err(anyhow::anyhow!("unknown voice arg: {other}")),
        }
    }

    crate::core::logging::init_for_cli_run(verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let mut config = crate::openhuman::config::Config::load_or_init()
            .await
            .unwrap_or_default();
        config.apply_env_overrides();

        let activation_mode = match mode.as_deref() {
            Some("tap") => ActivationMode::Tap,
            _ => ActivationMode::Push,
        };

        let server_config = VoiceServerConfig {
            hotkey: hotkey.unwrap_or_else(|| config.voice_server.hotkey.clone()),
            activation_mode,
            skip_cleanup,
            context: None,
            min_duration_secs: config.voice_server.min_duration_secs,
        };

        run_standalone(config, server_config)
            .await
            .map_err(anyhow::Error::msg)
    })?;

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
        print_namespace_help(namespace, schemas);
        return Ok(());
    }

    let function = args[0].as_str();
    let Some(schema) = schemas.iter().find(|s| s.function == function).cloned() else {
        return Err(anyhow::anyhow!(
            "unknown function '{namespace} {function}'. Run `openhuman {namespace} --help`."
        ));
    };

    // Special case for autocomplete start command which has its own CLI options.
    if namespace == "autocomplete" && function == "start" {
        if args.len() > 1 && is_help(&args[1]) {
            print_autocomplete_start_help();
            return Ok(());
        }
        let cli_options = parse_autocomplete_start_cli_options(&args[1..])?;
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        let value = rt
            .block_on(async { autocomplete_start_cli(cli_options).await })
            .map_err(anyhow::Error::msg)?;
        println!("{}", serde_json::to_string_pretty(&value)?);
        return Ok(());
    }

    if args.len() > 1 && is_help(&args[1]) {
        print_function_help(namespace, &schema);
        return Ok(());
    }

    // Generic parameter parsing and validation based on schema.
    let params = parse_function_params(&schema, &args[1..]).map_err(anyhow::Error::msg)?;
    let method = all::rpc_method_from_parts(namespace, function)
        .ok_or_else(|| anyhow::anyhow!("unregistered controller '{namespace}.{function}'"))?;

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;
    let value = rt
        .block_on(async { invoke_method(default_state(), &method, Value::Object(params)).await })
        .map_err(anyhow::Error::msg)?;

    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

/// Parses CLI options specific to the `autocomplete start` command.
///
/// # Arguments
///
/// * `args` - CLI arguments for the autocomplete start command.
fn parse_autocomplete_start_cli_options(args: &[String]) -> Result<AutocompleteStartCliOptions> {
    let mut debounce_ms: Option<u64> = None;
    let mut serve = false;
    let mut spawn = false;
    let mut i = 0usize;

    while i < args.len() {
        match args[i].as_str() {
            "--debounce-ms" => {
                let raw = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --debounce-ms"))?;
                debounce_ms = Some(
                    raw.parse::<u64>()
                        .map_err(|e| anyhow::anyhow!("invalid --debounce-ms: {e}"))?,
                );
                i += 2;
            }
            "--serve" => {
                serve = true;
                i += 1;
            }
            "--spawn" => {
                spawn = true;
                i += 1;
            }
            other => return Err(anyhow::anyhow!("unknown autocomplete start arg: {other}")),
        }
    }

    // Ensure the user doesn't try to both foreground and background the process.
    if serve && spawn {
        return Err(anyhow::anyhow!(
            "--serve and --spawn are mutually exclusive"
        ));
    }

    Ok(AutocompleteStartCliOptions {
        debounce_ms,
        serve,
        spawn,
    })
}

/// Prints help information for the `autocomplete start` command.
fn print_autocomplete_start_help() {
    println!("Usage: openhuman autocomplete start [--debounce-ms <u64>] [--serve|--spawn]");
    println!();
    println!("  --debounce-ms <u64>  Override debounce in milliseconds.");
    println!("  --serve              Run autocomplete loop in the current foreground process.");
    println!("  --spawn              Spawn autocomplete loop as a background process.");
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
        let value = parse_input_value(&spec.ty, raw_value)?;
        out.insert(key, value);
        i += 2;
    }

    all::validate_params(schema, &out)?;
    Ok(out)
}

/// Re-exported alias for parsing input values, used by the REPL.
pub fn parse_input_value_for_repl(ty: &TypeSchema, raw: &str) -> Result<Value, String> {
    parse_input_value(ty, raw)
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
    println!("  openhuman run [--host <addr>] [--port <u16>] [--jsonrpc-only] [--verbose]");
    println!("  openhuman repl [--verbose] [--eval '<cmd>'] [--batch]");
    println!("  openhuman call --method <name> [--params '<json>']");
    println!("  openhuman skills <subcommand> [options]   (skill development runtime)");
    println!("  openhuman voice [--hotkey <combo>] [--mode <tap|push>]  (voice dictation server)");
    println!("  openhuman <namespace> <function> [--param value ...]\n");
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
mod tests {
    use super::{
        grouped_schemas, parse_autocomplete_start_cli_options, parse_function_params,
        parse_input_value,
    };
    use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

    #[test]
    fn grouped_schemas_contains_migrated_namespaces() {
        let grouped = grouped_schemas();
        assert!(grouped.contains_key("health"));
        assert!(grouped.contains_key("doctor"));
        assert!(grouped.contains_key("encrypt"));
        assert!(grouped.contains_key("decrypt"));
        assert!(grouped.contains_key("autocomplete"));
        assert!(grouped.contains_key("config"));
        assert!(grouped.contains_key("auth"));
        assert!(grouped.contains_key("service"));
        assert!(grouped.contains_key("migrate"));
        assert!(grouped.contains_key("local_ai"));
    }

    #[test]
    fn parse_autocomplete_start_cli_options_rejects_serve_and_spawn() {
        let args = vec!["--serve".to_string(), "--spawn".to_string()];
        let err = parse_autocomplete_start_cli_options(&args)
            .expect_err("must reject mutually exclusive flags");
        assert!(err.to_string().contains("mutually exclusive"));
    }

    #[test]
    fn parse_function_params_rejects_unknown_param() {
        let schema = ControllerSchema {
            namespace: "test",
            function: "echo",
            description: "test schema",
            inputs: vec![FieldSchema {
                name: "message",
                ty: TypeSchema::String,
                required: true,
                comment: "message text",
            }],
            outputs: vec![FieldSchema {
                name: "result",
                ty: TypeSchema::String,
                required: true,
                comment: "echo response",
            }],
        };
        let args = vec!["--unknown".to_string(), "value".to_string()];
        let err = parse_function_params(&schema, &args).expect_err("unknown param should fail");
        assert!(err.contains("unknown param"));
    }

    #[test]
    fn parse_input_value_rejects_invalid_bool() {
        let err = parse_input_value(&TypeSchema::Bool, "not-a-bool")
            .expect_err("invalid bool should fail");
        assert!(err.contains("expected bool"));
    }
}
