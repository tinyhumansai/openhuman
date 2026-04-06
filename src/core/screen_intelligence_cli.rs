//! `openhuman screen-intelligence` — standalone CLI for the screen intelligence loop.
//!
//! Boots **only** the screen intelligence engine (accessibility capture + local-AI
//! vision) without the full desktop app, Socket.IO, or skills runtime.  Useful for
//! testing the capture → save → vision-analysis pipeline from a terminal.
//!
//! Usage:
//!   openhuman screen-intelligence run       [--port <u16>] [-v]
//!   openhuman screen-intelligence status
//!   openhuman screen-intelligence capture   [--keep]
//!   openhuman screen-intelligence start     [--ttl <secs>] [-v]
//!   openhuman screen-intelligence stop

use anyhow::Result;
use std::sync::Arc;

/// Entry point for `openhuman screen-intelligence <subcommand>`.
pub fn run_screen_intelligence_command(args: &[String]) -> Result<()> {
    if args.is_empty() || is_help(&args[0]) {
        print_help();
        return Ok(());
    }

    match args[0].as_str() {
        "run" => run_server(&args[1..]),
        "status" => run_status(&args[1..]),
        "capture" => run_capture(&args[1..]),
        "start" => run_start_session(&args[1..]),
        "stop" => run_stop_session(&args[1..]),
        other => Err(anyhow::anyhow!(
            "unknown screen-intelligence subcommand '{other}'. Run `openhuman screen-intelligence --help`."
        )),
    }
}

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

struct CliOpts {
    port: u16,
    verbose: bool,
    ttl_secs: u64,
    keep: bool,
}

fn parse_opts(args: &[String]) -> Result<(CliOpts, Vec<String>)> {
    let mut port: u16 = 7797;
    let mut verbose = false;
    let mut ttl_secs: u64 = 300;
    let mut keep = false;
    let mut rest = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--port" => {
                let val = args
                    .get(i + 1)
                    .ok_or_else(|| anyhow::anyhow!("missing value for --port"))?;
                port = val
                    .parse()
                    .map_err(|e| anyhow::anyhow!("invalid --port: {e}"))?;
                i += 2;
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
            port,
            verbose,
            ttl_secs,
            keep,
        },
        rest,
    ))
}

/// Bootstrap the screen intelligence engine with config.
async fn bootstrap_engine(
    verbose: bool,
) -> Result<Arc<crate::openhuman::screen_intelligence::AccessibilityEngine>> {
    use crate::openhuman::config::Config;
    use crate::openhuman::screen_intelligence::global_engine;

    let config = Config::load_or_init()
        .await
        .map_err(|e| anyhow::anyhow!("config load failed: {e}"))?;

    let engine = global_engine();
    let _ = engine
        .apply_config(config.screen_intelligence.clone())
        .await;

    if verbose {
        log::info!(
            "[screen-intelligence-cli] engine initialized, enabled={}, vision={}, keep_screenshots={}, workspace={}",
            config.screen_intelligence.enabled,
            config.screen_intelligence.vision_enabled,
            config.screen_intelligence.keep_screenshots,
            config.workspace_dir.display(),
        );
    }

    Ok(engine)
}

// ---------------------------------------------------------------------------
// Subcommands
// ---------------------------------------------------------------------------

/// `openhuman screen-intelligence run` — start a minimal JSON-RPC server with the
/// screen intelligence engine, useful for integration testing.
fn run_server(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence run [--port <u16>] [-v]");
        println!();
        println!("Start a lightweight JSON-RPC server exposing screen intelligence RPC methods.");
        println!();
        println!("  --port <u16>     Listen port (default: 7797)");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    crate::core::logging::init_for_cli_run(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let _engine = bootstrap_engine(opts.verbose).await?;

        let app = build_router();

        let bind_addr = format!("127.0.0.1:{}", opts.port);
        let listener = tokio::net::TcpListener::bind(&bind_addr).await?;

        log::info!(
            "[screen-intelligence-cli] ready — http://{bind_addr}/rpc (JSON-RPC 2.0)"
        );

        eprintln!();
        eprintln!("  Screen intelligence dev server listening on http://{bind_addr}");
        eprintln!("  JSON-RPC endpoint: POST http://{bind_addr}/rpc");
        eprintln!("  Health check:      GET  http://{bind_addr}/health");
        eprintln!("  Press Ctrl+C to stop.");
        eprintln!();

        axum::serve(listener, app).await?;
        Ok(())
    })
}

/// `openhuman screen-intelligence status` — print current engine status as JSON.
fn run_status(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence status [-v]");
        println!();
        println!("Print current screen intelligence engine status (permissions, session, config).");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_engine(opts.verbose).await?;
        let status = engine.status().await;
        println!(
            "{}",
            serde_json::to_string_pretty(&status).unwrap_or_else(|_| format!("{:?}", status))
        );
        Ok(())
    })
}

/// `openhuman screen-intelligence capture` — take a single screenshot and print info.
fn run_capture(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence capture [--keep] [-v]");
        println!();
        println!("Take a single screenshot, optionally save to workspace, and print diagnostics.");
        println!();
        println!("  --keep           Save the screenshot to {{workspace}}/screenshots/");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_engine(opts.verbose).await?;
        let result = engine.capture_test().await;

        if result.ok {
            eprintln!("  Capture: OK");
            eprintln!("  Mode:    {}", result.capture_mode);
            eprintln!("  Timing:  {}ms", result.timing_ms);
            if let Some(bytes) = result.bytes_estimate {
                eprintln!("  Size:    {} bytes", bytes);
            }
            if let Some(ctx) = &result.context {
                eprintln!(
                    "  App:     {}",
                    ctx.app_name.as_deref().unwrap_or("unknown")
                );
                eprintln!(
                    "  Window:  {}",
                    ctx.window_title.as_deref().unwrap_or("unknown")
                );
            }

            // Save to disk if --keep
            if opts.keep {
                if let Some(image_ref) = &result.image_ref {
                    let config = crate::openhuman::config::Config::load_or_init()
                        .await
                        .map_err(|e| anyhow::anyhow!("config load failed: {e}"))?;

                    let frame = crate::openhuman::screen_intelligence::CaptureFrame {
                        captured_at_ms: chrono::Utc::now().timestamp_millis(),
                        reason: "cli_capture".to_string(),
                        app_name: result
                            .context
                            .as_ref()
                            .and_then(|c| c.app_name.clone()),
                        window_title: result
                            .context
                            .as_ref()
                            .and_then(|c| c.window_title.clone()),
                        image_ref: Some(image_ref.clone()),
                    };

                    match crate::openhuman::screen_intelligence::AccessibilityEngine::save_screenshot_to_disk(
                        &config.workspace_dir,
                        &frame,
                    ) {
                        Ok(path) => {
                            eprintln!("  Saved:   {}", path.display());
                        }
                        Err(e) => {
                            eprintln!("  Save failed: {e}");
                        }
                    }
                }
            }
        } else {
            eprintln!("  Capture: FAILED");
            if let Some(err) = &result.error {
                eprintln!("  Error:   {err}");
            }
            std::process::exit(1);
        }

        // Also print as JSON for machine-readable output.
        let mut json_result = serde_json::to_value(&result).unwrap_or_default();
        // Strip image_ref from JSON output (too large for terminal).
        if let Some(obj) = json_result.as_object_mut() {
            obj.remove("image_ref");
        }
        println!(
            "{}",
            serde_json::to_string_pretty(&json_result).unwrap_or_default()
        );
        Ok(())
    })
}

/// `openhuman screen-intelligence start` — start a capture + vision session.
fn run_start_session(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence start [--ttl <secs>] [-v]");
        println!();
        println!("Start a screen intelligence capture session with vision analysis.");
        println!("The session runs until TTL expires or Ctrl+C is pressed.");
        println!();
        println!("  --ttl <secs>     Session duration (default: 300, max: 3600)");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    crate::core::logging::init_for_cli_run(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_engine(opts.verbose).await?;

        let params = crate::openhuman::screen_intelligence::StartSessionParams {
            consent: true,
            ttl_secs: Some(opts.ttl_secs),
            screen_monitoring: Some(true),
            device_control: Some(false),
            predictive_input: Some(false),
        };

        match engine.start_session(params).await {
            Ok(session) => {
                eprintln!("  Session started!");
                eprintln!("  TTL:           {}s", session.ttl_secs);
                eprintln!("  Vision:        {}", session.vision_enabled);
                eprintln!("  Panic hotkey:  {}", session.panic_hotkey);
                eprintln!();
                eprintln!("  Capturing screenshots and running vision analysis...");
                eprintln!("  Press Ctrl+C to stop.");
                eprintln!();

                // Print periodic status updates until the session ends.
                let mut tick = tokio::time::interval(std::time::Duration::from_secs(5));
                loop {
                    tick.tick().await;
                    let status = engine.status().await;
                    if !status.session.active {
                        eprintln!(
                            "\n  Session ended: {}",
                            status.session.stop_reason.unwrap_or_else(|| "unknown".into())
                        );
                        break;
                    }
                    eprintln!(
                        "  [{}] captures={} vision={} queue={} last_app={:?}",
                        chrono::Utc::now().format("%H:%M:%S"),
                        status.session.capture_count,
                        status.session.vision_state,
                        status.session.vision_queue_depth,
                        status.session.last_context.as_deref().unwrap_or("-"),
                    );
                    if let Some(summary) = &status.session.last_vision_summary {
                        let truncated = if summary.len() > 100 {
                            format!("{}…", &summary[..100])
                        } else {
                            summary.clone()
                        };
                        eprintln!("           notes: {truncated}");
                    }
                }
            }
            Err(e) => {
                eprintln!("  Failed to start session: {e}");
                std::process::exit(1);
            }
        }
        Ok(())
    })
}

/// `openhuman screen-intelligence stop` — stop an active session.
fn run_stop_session(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence stop [-v]");
        println!();
        println!("Stop the active screen intelligence session.");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let engine = bootstrap_engine(opts.verbose).await?;
        let session = engine.stop_session(Some("cli_stop".to_string())).await;
        eprintln!(
            "  Session stopped: {}",
            session
                .stop_reason
                .unwrap_or_else(|| "no active session".into())
        );
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Minimal HTTP router
// ---------------------------------------------------------------------------

fn build_router() -> axum::Router {
    use axum::routing::{get, post};

    axum::Router::new()
        .route("/health", get(health))
        .route("/rpc", post(rpc))
        .route("/status", get(status_endpoint))
}

async fn health() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({ "ok": true, "mode": "screen-intelligence-dev" }))
}

async fn rpc(
    axum::Json(req): axum::Json<crate::core::types::RpcRequest>,
) -> axum::response::Response {
    use crate::core::types::{RpcError, RpcFailure, RpcSuccess};
    use axum::response::IntoResponse;

    let id = req.id.clone();
    let state = crate::core::jsonrpc::default_state();

    match crate::core::jsonrpc::invoke_method(state, req.method.as_str(), req.params).await {
        Ok(value) => (
            axum::http::StatusCode::OK,
            axum::Json(RpcSuccess {
                jsonrpc: "2.0",
                id,
                result: value,
            }),
        )
            .into_response(),
        Err(message) => (
            axum::http::StatusCode::OK,
            axum::Json(RpcFailure {
                jsonrpc: "2.0",
                id,
                error: RpcError {
                    code: -32000,
                    message,
                    data: None,
                },
            }),
        )
            .into_response(),
    }
}

async fn status_endpoint() -> impl axum::response::IntoResponse {
    let engine = crate::openhuman::screen_intelligence::global_engine();
    let status = engine.status().await;
    axum::Json(serde_json::to_value(&status).unwrap_or_default())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn init_quiet_logging(verbose: bool) {
    if !verbose && std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    crate::core::logging::init_for_cli_run(verbose);
}

fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

fn print_help() {
    println!("openhuman screen-intelligence — screen intelligence runtime\n");
    println!("Usage:");
    println!("  openhuman screen-intelligence run       [--port <u16>] [-v]");
    println!("  openhuman screen-intelligence status     [-v]");
    println!("  openhuman screen-intelligence capture    [--keep] [-v]");
    println!("  openhuman screen-intelligence start      [--ttl <secs>] [-v]");
    println!("  openhuman screen-intelligence stop       [-v]");
    println!();
    println!("Subcommands:");
    println!("  run       Start a lightweight JSON-RPC server with screen intelligence");
    println!("  status    Print current engine status (permissions, session, config)");
    println!("  capture   Take a single screenshot and print diagnostics");
    println!("  start     Start a capture + vision session (runs until TTL or Ctrl+C)");
    println!("  stop      Stop the active session");
    println!();
    println!("Common options:");
    println!("  --port <u16>     Server port for 'run' (default: 7797)");
    println!("  --ttl <secs>     Session TTL for 'start' (default: 300)");
    println!("  --keep           Save screenshot to disk (for 'capture')");
    println!("  -v, --verbose    Enable debug logging");
}
