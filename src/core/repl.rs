//! Interactive REPL (read-eval-print loop) for the OpenHuman core.
//!
//! Drives the same code paths as the JSON-RPC server — no logic duplication.
//! See `docs/design-repl.md` for the full design document.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::io;
use std::path::PathBuf;
use std::time::Instant;

use rustyline::completion::{Completer, Pair};
use rustyline::error::ReadlineError;
use rustyline::highlight::Highlighter;
use rustyline::hint::Hinter;
use rustyline::history::FileHistory;
use rustyline::validate::Validator;
use rustyline::{CompletionType, Config, Editor, Helper};
use serde_json::{Map, Value};

use crate::core::all;
use crate::core::jsonrpc::{default_state, invoke_method, parse_json_params};
use crate::core::ControllerSchema;

// ---------------------------------------------------------------------------
// Public entry point
// ---------------------------------------------------------------------------

/// Run the REPL. Called from `cli.rs` when the user invokes `openhuman repl`.
pub fn run_repl(args: &[String]) -> anyhow::Result<()> {
    let opts = parse_repl_args(args)?;

    if opts.help {
        print_repl_usage();
        return Ok(());
    }

    crate::core::logging::init_for_cli_run(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    // Bootstrap the skill runtime (same as the server) so skill commands work.
    rt.block_on(async { crate::core::jsonrpc::bootstrap_skill_runtime().await });

    if let Some(expr) = &opts.eval {
        // --eval: run a single command, print result, exit.
        let mut state = ReplState::new(opts.verbose);
        state.json_mode = true; // machine-friendly by default for --eval
        let exit = rt.block_on(async { eval_line(&mut state, expr).await });
        std::process::exit(if exit == LoopAction::Exit { 1 } else { 0 });
    }

    if opts.batch {
        // --batch: read stdin line-by-line, raw JSON output, exit.
        let mut state = ReplState::new(opts.verbose);
        state.json_mode = true;
        let mut had_error = false;
        let stdin = io::stdin();
        let mut line = String::new();
        loop {
            line.clear();
            match stdin.read_line(&mut line) {
                Ok(0) => break,
                Ok(_) => {}
                Err(e) => {
                    eprintln!("error reading stdin: {e}");
                    had_error = true;
                    break;
                }
            }
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            let action = rt.block_on(async { eval_line(&mut state, trimmed).await });
            if action == LoopAction::Exit {
                had_error = true;
            }
        }
        std::process::exit(if had_error { 1 } else { 0 });
    }

    // Interactive mode.
    run_interactive(&rt, opts.verbose)
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

struct ReplOpts {
    verbose: bool,
    eval: Option<String>,
    batch: bool,
    help: bool,
}

fn parse_repl_args(args: &[String]) -> anyhow::Result<ReplOpts> {
    let mut opts = ReplOpts {
        verbose: false,
        eval: None,
        batch: false,
        help: false,
    };
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-v" | "--verbose" => {
                opts.verbose = true;
                i += 1;
            }
            "--eval" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --eval"))?;
                opts.eval = Some(val.clone());
                i += 2;
            }
            "--batch" => {
                opts.batch = true;
                i += 1;
            }
            "-h" | "--help" => {
                opts.help = true;
                i += 1;
            }
            other => return Err(anyhow::anyhow!("unknown repl arg: {other}")),
        }
    }
    Ok(opts)
}

fn print_repl_usage() {
    println!("Usage: openhuman repl [OPTIONS]");
    println!();
    println!("Options:");
    println!("  --verbose, -v        Enable debug logging");
    println!("  --eval '<command>'   Evaluate one command, print result, and exit");
    println!("  --batch              Read commands from stdin (one per line), raw JSON output");
    println!("  -h, --help           Show this help");
    println!();
    println!("Examples:");
    println!("  openhuman repl");
    println!("  openhuman repl --verbose");
    println!("  openhuman repl --eval 'health snapshot'");
    println!("  echo 'config get' | openhuman repl --batch");
}

// ---------------------------------------------------------------------------
// REPL state & toggles
// ---------------------------------------------------------------------------

struct ReplState {
    json_mode: bool,
    show_time: bool,
    verbose: bool,
}

impl ReplState {
    fn new(verbose: bool) -> Self {
        Self {
            json_mode: false,
            show_time: false,
            verbose,
        }
    }
}

// ---------------------------------------------------------------------------
// Interactive loop
// ---------------------------------------------------------------------------

fn run_interactive(rt: &tokio::runtime::Runtime, verbose: bool) -> anyhow::Result<()> {
    let config = Config::builder()
        .completion_type(CompletionType::List)
        .auto_add_history(false) // we handle history manually for sensitive-line filtering
        .build();

    let helper = ReplHelper::new();
    let mut rl: Editor<ReplHelper, FileHistory> = Editor::with_config(config)?;
    rl.set_helper(Some(helper));

    // Load history from ~/.openhuman/repl_history (ignore errors on first run).
    let history_path = history_file_path();
    let _ = rl.load_history(&history_path);

    let mut state = ReplState::new(verbose);

    println!("OpenHuman interactive shell (v{})", env!("CARGO_PKG_VERSION"));
    println!("Type `help` for commands, `exit` or Ctrl-D to quit.\n");

    loop {
        let prompt = "openhuman> ";
        match rl.readline(prompt) {
            Ok(line) => {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }

                // Add to history unless it looks sensitive.
                if !should_skip_history(trimmed) {
                    let _ = rl.add_history_entry(trimmed);
                }

                let action = rt.block_on(async { eval_line(&mut state, trimmed).await });
                if action == LoopAction::Quit {
                    break;
                }
            }
            Err(ReadlineError::Interrupted) => {
                // Ctrl-C: clear line, continue.
                println!("^C");
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: exit.
                println!("exit");
                break;
            }
            Err(err) => {
                eprintln!("readline error: {err}");
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

// ---------------------------------------------------------------------------
// History file path
// ---------------------------------------------------------------------------

fn history_file_path() -> PathBuf {
    let base = std::env::var("OPENHUMAN_WORKSPACE")
        .map(PathBuf::from)
        .unwrap_or_else(|_| {
            dirs::home_dir()
                .unwrap_or_else(|| PathBuf::from("."))
                .join(".openhuman")
        });
    let _ = std::fs::create_dir_all(&base);
    base.join("repl_history")
}

// ---------------------------------------------------------------------------
// Sensitive-line filter (history exclusion)
// ---------------------------------------------------------------------------

const SENSITIVE_PATTERNS: &[&str] = &[
    "api_key",
    "token",
    "secret",
    "password",
    "plaintext",
    "mnemonic",
    "private_key",
];

fn should_skip_history(line: &str) -> bool {
    let lower = line.to_lowercase();
    SENSITIVE_PATTERNS.iter().any(|s| lower.contains(s))
}

// ---------------------------------------------------------------------------
// Command evaluation
// ---------------------------------------------------------------------------

#[derive(Debug, PartialEq, Eq)]
enum LoopAction {
    Continue,
    Quit,
    Exit, // for --eval error signalling
}

async fn eval_line(state: &mut ReplState, line: &str) -> LoopAction {
    let parts = shell_split(line);
    if parts.is_empty() {
        return LoopAction::Continue;
    }

    let first = parts[0].as_str();

    // Meta commands (dot-prefixed toggles).
    if first.starts_with('.') {
        handle_meta_command(state, first, &parts[1..]);
        return LoopAction::Continue;
    }

    match first {
        "exit" | "quit" => return LoopAction::Quit,
        "help" => {
            print_help();
            return LoopAction::Continue;
        }
        "namespaces" => {
            print_namespaces();
            return LoopAction::Continue;
        }
        "schema" => {
            print_schema(parts.get(1).map(|s| s.as_str()));
            return LoopAction::Continue;
        }
        "env" => {
            print_env();
            return LoopAction::Continue;
        }
        "call" => {
            return eval_raw_call(state, &parts[1..]).await;
        }
        _ => {}
    }

    // Namespace + function dispatch.
    eval_namespace_function(state, &parts).await
}

// ---------------------------------------------------------------------------
// Meta commands (.json, .verbose, .time)
// ---------------------------------------------------------------------------

fn handle_meta_command(state: &mut ReplState, cmd: &str, args: &[String]) {
    let toggle = args.first().map(|s| s.as_str());
    match cmd {
        ".json" => {
            state.json_mode = parse_toggle(toggle, state.json_mode);
            eprintln!("  json mode: {}", if state.json_mode { "on" } else { "off" });
        }
        ".verbose" => {
            let new_val = parse_toggle(toggle, state.verbose);
            state.verbose = new_val;
            if new_val {
                std::env::set_var("RUST_LOG", "debug");
            } else {
                std::env::set_var("RUST_LOG", "info");
            }
            eprintln!(
                "  verbose: {} (note: log filter change takes effect on next log init)",
                if new_val { "on" } else { "off" }
            );
        }
        ".time" => {
            state.show_time = parse_toggle(toggle, state.show_time);
            eprintln!(
                "  timing: {}",
                if state.show_time { "on" } else { "off" }
            );
        }
        other => {
            eprintln!("  unknown meta command: {other}");
            eprintln!("  available: .json, .verbose, .time");
        }
    }
}

fn parse_toggle(arg: Option<&str>, current: bool) -> bool {
    match arg {
        Some("on" | "true" | "1") => true,
        Some("off" | "false" | "0") => false,
        _ => !current, // toggle
    }
}

// ---------------------------------------------------------------------------
// `call <method> [json]` — raw JSON-RPC-style invocation
// ---------------------------------------------------------------------------

async fn eval_raw_call(state: &mut ReplState, args: &[String]) -> LoopAction {
    if args.is_empty() {
        eprintln!("  usage: call <method> [json-params]");
        eprintln!("  example: call openhuman.health_snapshot {{}}");
        return LoopAction::Continue;
    }

    let method = &args[0];
    let params_str = args.get(1).map(|s| s.as_str()).unwrap_or("{}");
    let params = match parse_json_params(params_str) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("  error: {e}");
            return LoopAction::Exit;
        }
    };

    invoke_and_print(state, method, params).await
}

// ---------------------------------------------------------------------------
// Namespace/function dispatch
// ---------------------------------------------------------------------------

async fn eval_namespace_function(state: &mut ReplState, parts: &[String]) -> LoopAction {
    let grouped = grouped_schemas();
    let namespace = parts[0].as_str();

    let Some(schemas) = grouped.get(namespace) else {
        eprintln!("  error: unknown command '{namespace}'");
        eprintln!("  hint: type `help` for available commands, or `namespaces` to list all");
        return LoopAction::Exit;
    };

    if parts.len() < 2 {
        // Print functions in this namespace.
        eprintln!("  functions in '{namespace}':");
        for s in schemas {
            eprintln!("    {} — {}", s.function, s.description);
        }
        return LoopAction::Continue;
    }

    let function = parts[1].as_str();
    let Some(schema) = schemas.iter().find(|s| s.function == function) else {
        eprintln!("  error: unknown function '{namespace} {function}'");
        eprintln!("  hint: type `{namespace}` to see available functions");
        return LoopAction::Exit;
    };

    // Parse --key value params from remaining args.
    let params = match parse_cli_params(schema, &parts[2..]) {
        Ok(p) => Value::Object(p),
        Err(e) => {
            eprintln!("  error: {e}");
            show_function_hint(schema);
            return LoopAction::Exit;
        }
    };

    let method = match all::rpc_method_from_parts(namespace, function) {
        Some(m) => m,
        None => {
            eprintln!("  error: no registered handler for '{namespace}.{function}'");
            return LoopAction::Exit;
        }
    };

    invoke_and_print(state, &method, params).await
}

fn show_function_hint(schema: &ControllerSchema) {
    if schema.inputs.is_empty() {
        return;
    }
    let params: Vec<String> = schema
        .inputs
        .iter()
        .map(|i| {
            if i.required {
                format!("--{} <value>", i.name)
            } else {
                format!("[--{} <value>]", i.name)
            }
        })
        .collect();
    eprintln!(
        "  usage: {} {} {}",
        schema.namespace,
        schema.function,
        params.join(" ")
    );
}

// ---------------------------------------------------------------------------
// Param parsing (reuses cli.rs logic via the schema)
// ---------------------------------------------------------------------------

fn parse_cli_params(
    schema: &ControllerSchema,
    args: &[String],
) -> Result<Map<String, Value>, String> {
    // If there's a single arg that looks like JSON, parse it directly.
    if args.len() == 1 && args[0].starts_with('{') {
        let val: Value =
            serde_json::from_str(&args[0]).map_err(|e| format!("invalid JSON: {e}"))?;
        return match val {
            Value::Object(map) => {
                all::validate_params(schema, &map)?;
                Ok(map)
            }
            _ => Err("expected JSON object".to_string()),
        };
    }

    // Otherwise parse --key value pairs.
    let mut out = Map::new();
    let mut i = 0;
    while i < args.len() {
        let raw = &args[i];
        if !raw.starts_with("--") {
            return Err(format!("expected --<param>, got '{raw}'"));
        }
        let key = raw.trim_start_matches("--").replace('-', "_");
        let Some(spec) = schema.inputs.iter().find(|input| input.name == key) else {
            return Err(format!("unknown param '--{key}' for {}.{}", schema.namespace, schema.function));
        };
        let raw_value = args
            .get(i + 1)
            .ok_or_else(|| format!("missing value for --{key}"))?;
        let value = crate::core::cli::parse_input_value_for_repl(&spec.ty, raw_value)?;
        out.insert(key, value);
        i += 2;
    }

    all::validate_params(schema, &out)?;
    Ok(out)
}

// ---------------------------------------------------------------------------
// Invoke + print result
// ---------------------------------------------------------------------------

async fn invoke_and_print(state: &mut ReplState, method: &str, params: Value) -> LoopAction {
    let started = Instant::now();

    match invoke_method(default_state(), method, params).await {
        Ok(value) => {
            print_value(state, &value);
            if state.show_time {
                eprintln!("  ({}ms)", started.elapsed().as_millis());
            }
            LoopAction::Continue
        }
        Err(e) => {
            eprintln!("  error: {e}");
            LoopAction::Exit
        }
    }
}

fn print_value(state: &ReplState, value: &Value) {
    if state.json_mode {
        match serde_json::to_string_pretty(value) {
            Ok(s) => println!("{s}"),
            Err(e) => eprintln!("  serialization error: {e}"),
        }
    } else {
        // Pretty mode: indented JSON with 2-space indent.
        match serde_json::to_string_pretty(value) {
            Ok(s) => {
                for line in s.lines() {
                    println!("  {line}");
                }
            }
            Err(e) => eprintln!("  serialization error: {e}"),
        }
    }
}

// ---------------------------------------------------------------------------
// Built-in commands: help, namespaces, schema, env
// ---------------------------------------------------------------------------

fn print_help() {
    println!("Commands:");
    println!("  <namespace> <function> [--param value ...]   Call any registered controller");
    println!("  <namespace>                                  List functions in a namespace");
    println!("  call <method> [json]                         Raw JSON-RPC method call");
    println!("  schema [namespace]                           Show controller schemas");
    println!("  namespaces                                   List all namespaces");
    println!("  env                                          Show workspace & runtime paths");
    println!("  .verbose on|off                              Toggle debug logging");
    println!("  .json on|off                                 Toggle raw JSON output");
    println!("  .time on|off                                 Toggle timing display");
    println!("  help                                         Show this help");
    println!("  exit | quit | Ctrl-D                         Exit");
}

fn print_namespaces() {
    let grouped = grouped_schemas();
    println!("  Namespaces ({} total):", grouped.len());
    for (ns, schemas) in &grouped {
        let desc = all::namespace_description(ns)
            .unwrap_or("(no description)");
        println!("    {ns:<24} {desc} ({} functions)", schemas.len());
    }
}

fn print_schema(namespace: Option<&str>) {
    let grouped = grouped_schemas();
    match namespace {
        Some(ns) => {
            if let Some(schemas) = grouped.get(ns) {
                for s in schemas {
                    println!("  {}.{}", s.namespace, s.function);
                    println!("    {}", s.description);
                    if !s.inputs.is_empty() {
                        println!("    inputs:");
                        for i in &s.inputs {
                            let req = if i.required { "*" } else { " " };
                            println!("      {req} --{:<20} {}", i.name, i.comment);
                        }
                    }
                    if !s.outputs.is_empty() {
                        println!("    outputs:");
                        for o in &s.outputs {
                            println!("        {:<20} {}", o.name, o.comment);
                        }
                    }
                    println!();
                }
            } else {
                eprintln!("  unknown namespace '{ns}'");
            }
        }
        None => {
            let all = all::all_controller_schemas();
            println!("  {} registered controllers across {} namespaces",
                all.len(), grouped.len());
            println!("  use `schema <namespace>` for details");
        }
    }
}

fn print_env() {
    let workspace = std::env::var("OPENHUMAN_WORKSPACE").unwrap_or_else(|_| {
        dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".openhuman")
            .to_string_lossy()
            .to_string()
    });
    let cwd = std::env::current_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|_| "(unknown)".to_string());

    println!("  workspace:   {workspace}");
    println!("  cwd:         {cwd}");
    println!("  version:     {}", env!("CARGO_PKG_VERSION"));
    println!(
        "  rust_log:    {}",
        std::env::var("RUST_LOG").unwrap_or_else(|_| "(unset)".to_string())
    );
}

// ---------------------------------------------------------------------------
// Tab completion
// ---------------------------------------------------------------------------

struct ReplHelper {
    namespaces: Vec<String>,
    schemas: BTreeMap<String, Vec<ControllerSchema>>,
}

impl ReplHelper {
    fn new() -> Self {
        let schemas = grouped_schemas();
        let namespaces: Vec<String> = schemas.keys().cloned().collect();
        Self {
            namespaces,
            schemas,
        }
    }
}

impl Helper for ReplHelper {}
impl Validator for ReplHelper {}
impl Highlighter for ReplHelper {
    fn highlight_prompt<'b, 's: 'b, 'p: 'b>(
        &'s self,
        prompt: &'p str,
        _default: bool,
    ) -> Cow<'b, str> {
        Cow::Borrowed(prompt)
    }
}
impl Hinter for ReplHelper {
    type Hint = String;
    fn hint(&self, _line: &str, _pos: usize, _ctx: &rustyline::Context<'_>) -> Option<String> {
        None
    }
}

impl Completer for ReplHelper {
    type Candidate = Pair;

    fn complete(
        &self,
        line: &str,
        pos: usize,
        _ctx: &rustyline::Context<'_>,
    ) -> rustyline::Result<(usize, Vec<Pair>)> {
        let text = &line[..pos];
        let parts: Vec<&str> = text.split_whitespace().collect();

        // If the line ends with whitespace, we're completing the next word.
        let trailing_space = text.ends_with(' ') || text.ends_with('\t');

        match (parts.len(), trailing_space) {
            // Empty or first word being typed.
            (0, _) | (1, false) => {
                let prefix = parts.first().copied().unwrap_or("");
                let start = text.len() - prefix.len();
                let mut candidates: Vec<Pair> = Vec::new();

                // Built-in commands.
                for cmd in &[
                    "call", "help", "namespaces", "schema", "env", "exit", "quit",
                    ".json", ".verbose", ".time",
                ] {
                    if cmd.starts_with(prefix) {
                        candidates.push(Pair {
                            display: cmd.to_string(),
                            replacement: cmd.to_string(),
                        });
                    }
                }

                // Namespace names.
                for ns in &self.namespaces {
                    if ns.starts_with(prefix) {
                        candidates.push(Pair {
                            display: ns.clone(),
                            replacement: ns.clone(),
                        });
                    }
                }

                Ok((start, candidates))
            }

            // Second word: complete function names within the namespace.
            (1, true) | (2, false) => {
                let ns = parts[0];
                let prefix = if trailing_space {
                    ""
                } else {
                    parts.get(1).copied().unwrap_or("")
                };
                let start = text.len() - prefix.len();

                if let Some(schemas) = self.schemas.get(ns) {
                    let candidates: Vec<Pair> = schemas
                        .iter()
                        .filter(|s| s.function.starts_with(prefix))
                        .map(|s| Pair {
                            display: format!("{} — {}", s.function, s.description),
                            replacement: s.function.to_string(),
                        })
                        .collect();
                    Ok((start, candidates))
                } else {
                    Ok((pos, vec![]))
                }
            }

            // Third+ word: complete --param flags.
            _ => {
                let ns = parts[0];
                let func = parts.get(1).copied().unwrap_or("");
                let prefix = if trailing_space {
                    ""
                } else {
                    parts.last().copied().unwrap_or("")
                };

                if !prefix.is_empty() && !prefix.starts_with('-') {
                    return Ok((pos, vec![]));
                }

                let start = text.len() - prefix.len();
                let prefix_stripped = prefix.trim_start_matches('-');

                if let Some(schemas) = self.schemas.get(ns) {
                    if let Some(schema) = schemas.iter().find(|s| s.function == func) {
                        let candidates: Vec<Pair> = schema
                            .inputs
                            .iter()
                            .filter(|i| i.name.starts_with(prefix_stripped))
                            .map(|i| {
                                let flag = format!("--{}", i.name);
                                let req = if i.required { " (required)" } else { "" };
                                Pair {
                                    display: format!("{flag}{req} — {}", i.comment),
                                    replacement: flag,
                                }
                            })
                            .collect();
                        return Ok((start, candidates));
                    }
                }
                Ok((pos, vec![]))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Shell-like splitting (respects quoted strings and JSON braces)
// ---------------------------------------------------------------------------

fn shell_split(input: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut in_single_quote = false;
    let mut in_double_quote = false;
    let mut brace_depth: usize = 0;
    let mut bracket_depth: usize = 0;

    for ch in input.chars() {
        match ch {
            '\'' if !in_double_quote && brace_depth == 0 && bracket_depth == 0 => {
                in_single_quote = !in_single_quote;
            }
            '"' if !in_single_quote && brace_depth == 0 && bracket_depth == 0 => {
                in_double_quote = !in_double_quote;
            }
            '{' if !in_single_quote && !in_double_quote => {
                brace_depth += 1;
                current.push(ch);
            }
            '}' if !in_single_quote && !in_double_quote && brace_depth > 0 => {
                brace_depth -= 1;
                current.push(ch);
            }
            '[' if !in_single_quote && !in_double_quote => {
                bracket_depth += 1;
                current.push(ch);
            }
            ']' if !in_single_quote && !in_double_quote && bracket_depth > 0 => {
                bracket_depth -= 1;
                current.push(ch);
            }
            ' ' | '\t'
                if !in_single_quote
                    && !in_double_quote
                    && brace_depth == 0
                    && bracket_depth == 0 =>
            {
                if !current.is_empty() {
                    parts.push(std::mem::take(&mut current));
                }
            }
            _ => {
                current.push(ch);
            }
        }
    }
    if !current.is_empty() {
        parts.push(current);
    }
    parts
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn grouped_schemas() -> BTreeMap<String, Vec<ControllerSchema>> {
    let mut grouped: BTreeMap<String, Vec<ControllerSchema>> = BTreeMap::new();
    for schema in all::all_controller_schemas() {
        grouped
            .entry(schema.namespace.to_string())
            .or_default()
            .push(schema);
    }
    for schemas in grouped.values_mut() {
        schemas.sort_by_key(|s| s.function);
    }
    grouped
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- shell_split ---------------------------------------------------------

    #[test]
    fn shell_split_simple() {
        assert_eq!(shell_split("health snapshot"), vec!["health", "snapshot"]);
    }

    #[test]
    fn shell_split_with_flags() {
        assert_eq!(
            shell_split("config set --key value"),
            vec!["config", "set", "--key", "value"]
        );
    }

    #[test]
    fn shell_split_json_braces() {
        assert_eq!(
            shell_split(r#"call openhuman.health_snapshot {"verbose": true}"#),
            vec!["call", "openhuman.health_snapshot", r#"{"verbose": true}"#]
        );
    }

    #[test]
    fn shell_split_nested_json() {
        assert_eq!(
            shell_split(r#"call method {"a": {"b": 1}}"#),
            vec!["call", "method", r#"{"a": {"b": 1}}"#]
        );
    }

    #[test]
    fn shell_split_single_quotes() {
        assert_eq!(
            shell_split("call method 'hello world'"),
            vec!["call", "method", "hello world"]
        );
    }

    #[test]
    fn shell_split_double_quotes() {
        assert_eq!(
            shell_split(r#"call method "hello world""#),
            vec!["call", "method", "hello world"]
        );
    }

    #[test]
    fn shell_split_empty() {
        assert!(shell_split("").is_empty());
        assert!(shell_split("   ").is_empty());
    }

    // -- should_skip_history -------------------------------------------------

    #[test]
    fn skip_history_api_key() {
        assert!(should_skip_history("encrypt secret --plaintext my-api-key"));
    }

    #[test]
    fn skip_history_token() {
        assert!(should_skip_history(r#"call auth.store_session {"token": "abc"}"#));
    }

    #[test]
    fn skip_history_mnemonic() {
        assert!(should_skip_history("some command with mnemonic words"));
    }

    #[test]
    fn skip_history_private_key() {
        assert!(should_skip_history("import --private_key 0xabc"));
    }

    #[test]
    fn skip_history_safe_command() {
        assert!(!should_skip_history("health snapshot"));
    }

    #[test]
    fn skip_history_safe_config() {
        assert!(!should_skip_history("config get"));
    }

    // -- parse_toggle --------------------------------------------------------

    #[test]
    fn toggle_on() {
        assert!(parse_toggle(Some("on"), false));
        assert!(parse_toggle(Some("true"), false));
        assert!(parse_toggle(Some("1"), false));
    }

    #[test]
    fn toggle_off() {
        assert!(!parse_toggle(Some("off"), true));
        assert!(!parse_toggle(Some("false"), true));
        assert!(!parse_toggle(Some("0"), true));
    }

    #[test]
    fn toggle_flip() {
        assert!(parse_toggle(None, false));
        assert!(!parse_toggle(None, true));
    }
}
