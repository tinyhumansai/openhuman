//! `openhuman subconscious` — CLI for testing and debugging the subconscious loop.
//!
//! Usage:
//!   openhuman subconscious tick [--workspace <path>] [--mode simple|aggressive] [--verbose]
//!   openhuman subconscious status [--workspace <path>]

use anyhow::{anyhow, Result};
use std::path::PathBuf;

pub fn run_subconscious_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "tick" => run_tick(&args[1..]),
        "status" => run_status(&args[1..]),
        other => Err(anyhow!(
            "unknown subconscious subcommand '{other}'. Run `openhuman subconscious --help`."
        )),
    }
}

// ── tick ────────────────────────────────────────────────────────────────────

struct TickFlags {
    workspace: Option<PathBuf>,
    mode: Option<String>,
    verbose: bool,
}

fn parse_tick_flags(args: &[String]) -> Result<TickFlags> {
    let mut workspace: Option<PathBuf> = None;
    let mut mode: Option<String> = None;
    let mut verbose = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace" | "-w" => {
                workspace = Some(PathBuf::from(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --workspace"))?,
                ));
                i += 2;
            }
            "--mode" | "-m" => {
                mode = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --mode"))?
                        .clone(),
                );
                i += 2;
            }
            "--verbose" | "-v" => {
                verbose = true;
                i += 1;
            }
            other => return Err(anyhow!("unknown flag '{other}'")),
        }
    }
    Ok(TickFlags {
        workspace,
        mode,
        verbose,
    })
}

fn run_tick(args: &[String]) -> Result<()> {
    let flags = parse_tick_flags(args)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| anyhow!("config load failed: {e}"))?;

        if let Some(ws) = &flags.workspace {
            config.workspace_dir = ws.clone();
        }

        if let Some(mode_str) = &flags.mode {
            config.heartbeat.subconscious_mode = match mode_str.as_str() {
                "simple" => crate::openhuman::config::schema::SubconsciousMode::Simple,
                "aggressive" => crate::openhuman::config::schema::SubconsciousMode::Aggressive,
                other => {
                    return Err(anyhow!(
                        "unknown mode '{other}', expected simple|aggressive"
                    ))
                }
            };
            config.heartbeat.enabled = true;
            config.heartbeat.inference_enabled = true;
        }

        // Ensure subconscious is enabled
        if !config.heartbeat.enabled || !config.heartbeat.inference_enabled {
            config.heartbeat.enabled = true;
            config.heartbeat.inference_enabled = true;
            if !config.heartbeat.subconscious_mode.is_enabled() {
                config.heartbeat.subconscious_mode =
                    crate::openhuman::config::schema::SubconsciousMode::Simple;
            }
        }

        let mode = config.heartbeat.effective_subconscious_mode();
        eprintln!(
            "[subconscious] mode={} workspace={}",
            mode.as_str(),
            config.workspace_dir.display()
        );

        // Init memory client
        let _ = crate::openhuman::memory::global::init(config.workspace_dir.clone());

        // Init scheduler gate so is_signed_out() works
        crate::openhuman::scheduler_gate::init_global(&config);

        // Seed signed_out from session token
        match crate::api::jwt::get_session_token(&config) {
            Ok(Some(_)) => {
                crate::openhuman::scheduler_gate::set_signed_out(false);
                eprintln!("[subconscious] session token found — provider available");
            }
            Ok(None) => {
                eprintln!("[subconscious] WARNING: no session token — cloud provider will fail");
                eprintln!("  hint: run `openhuman call auth store_session --token <JWT>` first");
            }
            Err(e) => {
                eprintln!("[subconscious] WARNING: session token read failed: {e}");
            }
        }

        // Check provider availability
        if let Some(reason) =
            crate::openhuman::subconscious::engine::subconscious_provider_unavailable_reason(
                &config,
            )
        {
            eprintln!("[subconscious] provider unavailable: {reason}");
            return Err(anyhow!("provider unavailable: {reason}"));
        }

        // Create engine and run tick. The engine pulls its own memory_diff /
        // context state from the workspace — no memory client to pass in.
        let engine = crate::openhuman::subconscious::SubconsciousEngine::new(&config);

        eprintln!("[subconscious] running tick...");
        let result = engine
            .tick()
            .await
            .map_err(|e| anyhow!("tick failed: {e}"))?;

        eprintln!(
            "[subconscious] tick complete: duration={}ms response_chars={}",
            result.duration_ms, result.response_chars,
        );

        if flags.verbose {
            // Print the world baseline the next tick will diff against.
            let baseline = crate::openhuman::subconscious::store::with_connection(
                &config.workspace_dir,
                crate::openhuman::subconscious::store::get_baseline_checkpoint_id,
            )
            .unwrap_or(None);
            match baseline {
                Some(id) => eprintln!("[subconscious] world baseline checkpoint: {id}"),
                None => eprintln!("[subconscious] no world baseline established yet"),
            }
        }

        Ok(())
    })
}

// ── status ─────────────────────────────────────────────────────────────────

fn run_status(args: &[String]) -> Result<()> {
    let workspace = parse_workspace_flag(args)?;

    let rt = tokio::runtime::Runtime::new()?;
    rt.block_on(async {
        let mut config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| anyhow!("config load failed: {e}"))?;
        if let Some(ws) = workspace {
            config.workspace_dir = ws;
        }

        let mode = config.heartbeat.effective_subconscious_mode();
        let provider_reason = if mode.is_enabled() {
            crate::openhuman::subconscious::engine::subconscious_provider_unavailable_reason(
                &config,
            )
        } else {
            None
        };

        let last_tick = crate::openhuman::subconscious::store::with_connection(
            &config.workspace_dir,
            crate::openhuman::subconscious::store::get_last_tick_at,
        )
        .ok();

        let status = serde_json::json!({
            "mode": mode.as_str(),
            "enabled": mode.is_enabled(),
            "provider_available": provider_reason.is_none(),
            "provider_unavailable_reason": provider_reason,
            "last_tick_at": last_tick.filter(|v| *v > 0.0),
            "interval_minutes": mode.default_interval_minutes().max(5),
        });

        println!("{}", serde_json::to_string_pretty(&status)?);
        Ok(())
    })
}

// ── helpers ────────────────────────────────────────────────────────────────

fn parse_workspace_flag(args: &[String]) -> Result<Option<PathBuf>> {
    let mut workspace: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace" | "-w" => {
                workspace = Some(PathBuf::from(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing --workspace"))?,
                ));
                i += 2;
            }
            other => return Err(anyhow!("unknown flag '{other}'")),
        }
    }
    Ok(workspace)
}

fn is_help(s: &str) -> bool {
    matches!(s, "--help" | "-h" | "help")
}

fn print_help() {
    eprintln!(
        "Usage: openhuman subconscious <command> [options]

Commands:
  tick          Run a single subconscious tick (synchronous, waits for completion)
  status        Show current subconscious engine status

Tick options:
  --mode <simple|aggressive>   Override the subconscious mode
  --workspace <path>           Override workspace directory
  --verbose, -v                Print the world baseline checkpoint after tick

Common options:
  --workspace <path>           Override workspace directory
"
    );
}
