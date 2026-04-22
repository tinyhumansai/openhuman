//! `openhuman screen-intelligence` — standalone CLI for the screen intelligence loop.
//!
//! Boots **only** the screen intelligence engine (accessibility capture + local-AI
//! vision) without the full desktop app, Socket.IO, or skills runtime.  Useful for
//! testing the capture → save → vision-analysis pipeline from a terminal.
//!
//! Usage:
//!   openhuman screen-intelligence run       [--ttl <secs>] [--keep] [-v]
//!   openhuman screen-intelligence status    [-v]
//!   openhuman screen-intelligence capture   [--keep] [-v]
//!   openhuman screen-intelligence start     [--ttl <secs>] [-v]
//!   openhuman screen-intelligence stop      [-v]
//!   openhuman screen-intelligence doctor    [-v]
//!   openhuman screen-intelligence vision    [--limit <n>] [-v]

use anyhow::Result;
use std::sync::Arc;

mod capture;
mod doctor;
mod server;
mod session;

/// Entry point for `openhuman screen-intelligence <subcommand>`.
pub(crate) fn run_screen_intelligence_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "run" => server::run_server(&args[1..]),
        "status" => session::run_status(&args[1..]),
        "capture" => capture::run_capture(&args[1..]),
        "start" => session::run_start_session(&args[1..]),
        "stop" => session::run_stop_session(&args[1..]),
        "doctor" => doctor::run_doctor(&args[1..]),
        "vision" => capture::run_vision(&args[1..]),
        other => Err(anyhow::anyhow!(
            "unknown screen-intelligence subcommand '{other}'. Run `openhuman screen-intelligence --help`."
        )),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers (visible to sibling subcommand modules)
// ---------------------------------------------------------------------------

pub(super) struct CliOpts {
    pub verbose: bool,
    pub ttl_secs: u64,
    pub keep: bool,
    pub limit: usize,
    pub no_vision_model: bool,
}

pub(super) fn parse_opts(args: &[String]) -> Result<(CliOpts, Vec<String>)> {
    let mut verbose = false;
    let mut ttl_secs: u64 = 300;
    let mut keep = false;
    let mut limit: usize = 10;
    let mut no_vision_model = false;
    let mut rest = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--no-vision-model" | "--ocr-only" => {
                no_vision_model = true;
                i += 1;
            }
            "--ttl" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --ttl"))?;
                ttl_secs = val
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid --ttl: {e}"))?;
                i += 2;
            }
            "--limit" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --limit"))?;
                limit = val
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid --limit: {e}"))?;
                i += 2;
            }
            "--keep" => {
                keep = true;
                i += 1;
            }
            "-v" | "--verbose" => {
                verbose = true;
                i += 1;
            }
            "-h" | "--help" => {
                rest.push(args[i].clone());
                i += 1;
            }
            _ => {
                rest.push(args[i].clone());
                i += 1;
            }
        }
    }

    Ok((
        CliOpts {
            verbose,
            ttl_secs,
            keep,
            limit,
            no_vision_model,
        },
        rest,
    ))
}

/// Bootstrap the screen intelligence engine with config.
pub(super) async fn bootstrap_engine(
    verbose: bool,
) -> Result<Arc<crate::openhuman::screen_intelligence::AccessibilityEngine>> {
    bootstrap_engine_with_opts(verbose, false).await
}

/// Bootstrap with CLI overrides.
pub(super) async fn bootstrap_engine_with_opts(
    verbose: bool,
    no_vision_model: bool,
) -> Result<Arc<crate::openhuman::screen_intelligence::AccessibilityEngine>> {
    use crate::openhuman::config::Config;
    use crate::openhuman::screen_intelligence::global_engine;

    let mut config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("config load failed: {e}"))?;

    if no_vision_model {
        config.screen_intelligence.use_vision_model = false;
    }

    let engine = global_engine();
    let _ = engine
        .apply_config(config.screen_intelligence.clone())
        .await;

    if verbose {
        log::info!(
            "[screen-intelligence-cli] engine initialized, enabled={}, vision={}, use_vision_model={}, keep_screenshots={}, workspace={}",
            config.screen_intelligence.enabled,
            config.screen_intelligence.vision_enabled,
            config.screen_intelligence.use_vision_model,
            config.screen_intelligence.keep_screenshots,
            config.workspace_dir.display(),
        );
    }

    Ok(engine)
}

/// Quiet logging: only `warn` unless verbose (used for non-server subcommands).
pub(super) fn init_quiet_logging(verbose: bool) {
    if !verbose && std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    crate::core::logging::init_for_cli_run(verbose, crate::core::logging::CliLogDefault::Global);
}

pub(super) fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

fn print_help() {
    println!("openhuman screen-intelligence — standalone screen intelligence runtime\n");
    println!("Boots only the screen intelligence engine (accessibility capture + local-AI");
    println!("vision) without the full desktop app, Socket.IO, or skills runtime.\n");
    println!("Usage:");
    println!("  openhuman screen-intelligence run       [--ttl <secs>] [--no-vision-model] [-v]");
    println!("  openhuman screen-intelligence status     [-v]");
    println!("  openhuman screen-intelligence capture    [--keep] [-v]");
    println!("  openhuman screen-intelligence start      [--ttl <secs>] [--no-vision-model] [-v]");
    println!("  openhuman screen-intelligence stop       [-v]");
    println!("  openhuman screen-intelligence doctor     [-v]");
    println!("  openhuman screen-intelligence vision     [--limit <n>] [-v]");
    println!();
    println!("Subcommands:");
    println!("  run       Start the capture → vision → log loop (blocks until TTL/Ctrl+C)");
    println!("  status    Print current engine status (permissions, session, config)");
    println!("  capture   Take a single screenshot and print diagnostics");
    println!("  start     Start a capture + vision session (runs until TTL or Ctrl+C)");
    println!("  stop      Stop the active session");
    println!("  doctor    Check system readiness (permissions, vision config, platform)");
    println!("  vision    Print recent vision summaries from the active session");
    println!();
    println!("Common options:");
    println!("  --ttl <secs>        Session TTL (default: 300)");
    println!("  --limit <n>         Max vision summaries for 'vision' (default: 10)");
    println!("  --keep              Save screenshot to disk (for 'capture')");
    println!("  --no-vision-model   Skip vision LLM — use OCR + text LLM only");
    println!("  --ocr-only          Alias for --no-vision-model");
    println!("  -v, --verbose       Enable debug logging");
}
