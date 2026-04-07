//! `openhuman screen-intelligence` — standalone CLI for the screen intelligence loop.
//!
//! Boots **only** the screen intelligence engine (accessibility capture + local-AI
//! vision) without the full desktop app, Socket.IO, or skills runtime.  Useful for
//! testing the capture → save → vision-analysis pipeline from a terminal.
//!
//! Usage:
//!   openhuman screen-intelligence run       [--port <u16>] [-v]
//!   openhuman screen-intelligence status    [-v]
//!   openhuman screen-intelligence capture   [--keep] [-v]
//!   openhuman screen-intelligence start     [--ttl <secs>] [-v]
//!   openhuman screen-intelligence stop      [-v]
//!   openhuman screen-intelligence doctor    [-v]
//!   openhuman screen-intelligence vision    [--limit <n>] [-v]

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
        "doctor" => run_doctor(&args[1..]),
        "vision" => run_vision(&args[1..]),
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
    limit: usize,
}

fn parse_opts(args: &[String]) -> Result<(CliOpts, Vec<String>)> {
    let mut port: u16 = 7797;
    let mut verbose = false;
    let mut ttl_secs: u64 = 300;
    let mut keep = false;
    let mut limit: usize = 10;
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
            port,
            verbose,
            ttl_secs,
            keep,
            limit,
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

        log::info!("[screen-intelligence-cli] ready — http://{bind_addr}/rpc (JSON-RPC 2.0)");

        eprintln!();
        eprintln!("  Screen Intelligence dev server listening on http://{bind_addr}");
        eprintln!();
        eprintln!("  Core:");
        eprintln!("    POST http://{bind_addr}/rpc            JSON-RPC 2.0 (screen_intelligence.*)");
        eprintln!("    GET  http://{bind_addr}/health         Health check");
        eprintln!();
        eprintln!("  REST convenience endpoints:");
        eprintln!("    GET  http://{bind_addr}/status         Full engine status");
        eprintln!("    GET  http://{bind_addr}/permissions    Permission state");
        eprintln!("    GET  http://{bind_addr}/session        Session status + features");
        eprintln!("    POST http://{bind_addr}/session/start  Start capture session");
        eprintln!("    POST http://{bind_addr}/session/stop   Stop session");
        eprintln!("    POST http://{bind_addr}/capture        Trigger manual capture");
        eprintln!("    POST http://{bind_addr}/capture/test   Standalone capture test");
        eprintln!("    GET  http://{bind_addr}/vision/recent  Recent vision summaries");
        eprintln!("    POST http://{bind_addr}/vision/flush   Analyze latest frame now");
        eprintln!("    GET  http://{bind_addr}/doctor         System readiness diagnostics");
        eprintln!("    GET  http://{bind_addr}/config         Current SI config");
        eprintln!("    GET  http://{bind_addr}/events         SSE status stream (2s interval)");
        eprintln!();
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
                            status
                                .session
                                .stop_reason
                                .unwrap_or_else(|| "unknown".into())
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

/// `openhuman screen-intelligence doctor` — diagnostic readiness check.
fn run_doctor(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence doctor [-v]");
        println!();
        println!("Check system readiness: permissions, platform support, vision config.");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let _engine = bootstrap_engine(opts.verbose).await?;

        let doctor_json =
            crate::openhuman::screen_intelligence::rpc::accessibility_doctor_cli_json().await?;

        let summary = &doctor_json["result"]["summary"];
        let recommendations = &doctor_json["result"]["recommendations"];
        let permissions = &doctor_json["result"]["permissions"];

        eprintln!("  Screen Intelligence Doctor");
        eprintln!("  ──────────────────────────");
        eprintln!();

        let check = |ok: bool| if ok { "✓" } else { "✗" };

        let platform_ok = summary["platform_supported"].as_bool().unwrap_or(false);
        let screen_ok = summary["screen_capture_ready"].as_bool().unwrap_or(false);
        let control_ok = summary["device_control_ready"].as_bool().unwrap_or(false);
        let input_ok = summary["input_monitoring_ready"].as_bool().unwrap_or(false);
        let overall_ok = summary["overall_ready"].as_bool().unwrap_or(false);

        eprintln!("  {} Platform supported", check(platform_ok));
        eprintln!("  {} Screen recording", check(screen_ok));
        eprintln!("  {} Accessibility (device control)", check(control_ok));
        eprintln!("  {} Input monitoring", check(input_ok));
        eprintln!();

        // Vision config check
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .ok();
        if let Some(ref cfg) = config {
            let si = &cfg.screen_intelligence;
            let la = &cfg.local_ai;
            eprintln!("  Config:");
            eprintln!("    enabled:           {}", si.enabled);
            eprintln!("    vision_enabled:    {}", si.vision_enabled);
            eprintln!("    baseline_fps:      {}", si.baseline_fps);
            eprintln!("    keep_screenshots:  {}", si.keep_screenshots);
            eprintln!("    local_ai.enabled:  {}", la.enabled);
            eprintln!("    local_ai.provider: {}", la.provider);
            if si.vision_enabled && !la.enabled {
                eprintln!(
                    "    ⚠  Vision is enabled but local_ai.enabled=false — vision analysis will fail"
                );
            }
        }

        eprintln!();
        if overall_ok {
            eprintln!("  ✓ Overall: READY");
        } else {
            eprintln!("  ✗ Overall: NOT READY");
            eprintln!();
            eprintln!("  Recommendations:");
            if let Some(recs) = recommendations.as_array() {
                for rec in recs {
                    if let Some(s) = rec.as_str() {
                        eprintln!("    • {s}");
                    }
                }
            }
        }
        eprintln!();

        // Also print machine-readable JSON
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "summary": summary,
                "permissions": permissions,
                "config": config.as_ref().map(|c| serde_json::json!({
                    "enabled": c.screen_intelligence.enabled,
                    "vision_enabled": c.screen_intelligence.vision_enabled,
                    "baseline_fps": c.screen_intelligence.baseline_fps,
                    "keep_screenshots": c.screen_intelligence.keep_screenshots,
                    "local_ai_enabled": c.local_ai.enabled,
                    "local_ai_provider": c.local_ai.provider,
                })),
            }))
            .unwrap_or_default()
        );
        Ok(())
    })
}

/// `openhuman screen-intelligence vision` — inspect recent vision summaries.
fn run_vision(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence vision [--limit <n>] [-v]");
        println!();
        println!("Print recent vision summaries from the active session.");
        println!();
        println!("  --limit <n>      Maximum summaries to show (default: 10)");
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
        let result = engine.vision_recent(Some(opts.limit)).await;

        if result.summaries.is_empty() {
            eprintln!("  No vision summaries available.");
            eprintln!("  Start a session first: openhuman screen-intelligence start");
        } else {
            eprintln!("  {} vision summary(ies):\n", result.summaries.len());
            for (i, s) in result.summaries.iter().enumerate() {
                let ts = chrono::DateTime::from_timestamp_millis(s.captured_at_ms)
                    .map(|dt| dt.format("%H:%M:%S").to_string())
                    .unwrap_or_else(|| "?".to_string());
                eprintln!(
                    "  [{}] {} — {} (confidence: {:.0}%)",
                    i + 1,
                    ts,
                    s.app_name.as_deref().unwrap_or("unknown"),
                    s.confidence * 100.0,
                );
                if !s.ui_state.is_empty() {
                    let truncated = if s.ui_state.len() > 120 {
                        format!("{}…", &s.ui_state[..120])
                    } else {
                        s.ui_state.clone()
                    };
                    eprintln!("       ui: {truncated}");
                }
                if !s.actionable_notes.is_empty() {
                    let truncated = if s.actionable_notes.len() > 120 {
                        format!("{}…", &s.actionable_notes[..120])
                    } else {
                        s.actionable_notes.clone()
                    };
                    eprintln!("       notes: {truncated}");
                }
                eprintln!();
            }
        }

        // Machine-readable output
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// HTTP router with convenience REST + JSON-RPC + CORS
// ---------------------------------------------------------------------------

fn build_router() -> axum::Router {
    use axum::routing::{get, post};
    use tower_http::cors::{Any, CorsLayer};

    let cors = CorsLayer::new()
        .allow_origin(Any)
        .allow_methods(Any)
        .allow_headers(Any);

    axum::Router::new()
        // Core
        .route("/health", get(health))
        .route("/rpc", post(rpc))
        // Convenience REST endpoints (read-only GETs + action POSTs)
        .route("/status", get(status_endpoint))
        .route("/permissions", get(permissions_endpoint))
        .route("/session", get(session_endpoint))
        .route("/session/start", post(session_start_endpoint))
        .route("/session/stop", post(session_stop_endpoint))
        .route("/capture", post(capture_endpoint))
        .route("/capture/test", post(capture_test_endpoint))
        .route("/vision/recent", get(vision_recent_endpoint))
        .route("/vision/flush", post(vision_flush_endpoint))
        .route("/doctor", get(doctor_endpoint))
        .route("/config", get(config_endpoint))
        // Watch: long-poll endpoint for status changes
        .route("/watch", get(watch_endpoint))
        .layer(cors)
}

async fn health() -> impl axum::response::IntoResponse {
    axum::Json(serde_json::json!({
        "ok": true,
        "mode": "screen-intelligence-dev",
        "endpoints": {
            "rpc": "POST /rpc — JSON-RPC 2.0 (screen_intelligence.* methods)",
            "status": "GET /status — full engine status",
            "permissions": "GET /permissions — permission state",
            "session": "GET /session — session status",
            "session_start": "POST /session/start — start session (body: StartSessionParams)",
            "session_stop": "POST /session/stop — stop session (body: { reason? })",
            "capture": "POST /capture — trigger manual capture",
            "capture_test": "POST /capture/test — standalone capture test",
            "vision_recent": "GET /vision/recent?limit=10 — recent vision summaries",
            "vision_flush": "POST /vision/flush — analyze latest frame now",
            "doctor": "GET /doctor — system readiness diagnostics",
            "config": "GET /config — current screen intelligence config",
            "events": "GET /events — SSE stream of session status updates"
        }
    }))
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

async fn permissions_endpoint() -> impl axum::response::IntoResponse {
    let engine = crate::openhuman::screen_intelligence::global_engine();
    let status = engine.status().await;
    axum::Json(serde_json::json!({
        "permissions": status.permissions,
        "platform_supported": status.platform_supported,
        "permission_check_process_path": status.permission_check_process_path,
    }))
}

async fn session_endpoint() -> impl axum::response::IntoResponse {
    let engine = crate::openhuman::screen_intelligence::global_engine();
    let status = engine.status().await;
    axum::Json(serde_json::json!({
        "session": status.session,
        "features": status.features,
        "foreground_context": status.foreground_context,
        "is_context_blocked": status.is_context_blocked,
    }))
}

async fn session_start_endpoint(
    axum::Json(params): axum::Json<crate::openhuman::screen_intelligence::StartSessionParams>,
) -> axum::response::Response {
    use axum::response::IntoResponse;

    match crate::openhuman::screen_intelligence::global_engine()
        .start_session(params)
        .await
    {
        Ok(session) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::to_value(&session).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::BAD_REQUEST,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn session_stop_endpoint(
    axum::Json(params): axum::Json<crate::openhuman::screen_intelligence::StopSessionParams>,
) -> impl axum::response::IntoResponse {
    let session = crate::openhuman::screen_intelligence::global_engine()
        .disable(params.reason)
        .await;
    axum::Json(serde_json::to_value(&session).unwrap_or_default())
}

async fn capture_endpoint() -> axum::response::Response {
    use axum::response::IntoResponse;

    match crate::openhuman::screen_intelligence::global_engine()
        .capture_now()
        .await
    {
        Ok(result) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::to_value(&result).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn capture_test_endpoint() -> impl axum::response::IntoResponse {
    let result = crate::openhuman::screen_intelligence::global_engine()
        .capture_test()
        .await;
    // Strip image_ref from response (too large for REST)
    let mut json = serde_json::to_value(&result).unwrap_or_default();
    if let Some(obj) = json.as_object_mut() {
        obj.remove("image_ref");
    }
    axum::Json(json)
}

async fn vision_recent_endpoint(
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
) -> impl axum::response::IntoResponse {
    let limit = query
        .get("limit")
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(10);
    let result = crate::openhuman::screen_intelligence::global_engine()
        .vision_recent(Some(limit))
        .await;
    axum::Json(serde_json::to_value(&result).unwrap_or_default())
}

async fn vision_flush_endpoint() -> axum::response::Response {
    use axum::response::IntoResponse;

    match crate::openhuman::screen_intelligence::global_engine()
        .vision_flush()
        .await
    {
        Ok(result) => (
            axum::http::StatusCode::OK,
            axum::Json(serde_json::to_value(&result).unwrap_or_default()),
        )
            .into_response(),
        Err(e) => (
            axum::http::StatusCode::INTERNAL_SERVER_ERROR,
            axum::Json(serde_json::json!({ "error": e })),
        )
            .into_response(),
    }
}

async fn doctor_endpoint() -> impl axum::response::IntoResponse {
    match crate::openhuman::screen_intelligence::rpc::accessibility_doctor_cli_json().await {
        Ok(json) => axum::Json(json),
        Err(e) => axum::Json(serde_json::json!({ "error": e })),
    }
}

async fn config_endpoint() -> impl axum::response::IntoResponse {
    let engine = crate::openhuman::screen_intelligence::global_engine();
    let status = engine.status().await;
    axum::Json(serde_json::json!({
        "config": status.config,
        "denylist": status.denylist,
    }))
}

/// SSE endpoint that streams session status every 2 seconds.
async fn sse_endpoint() -> axum::response::Sse<impl futures::Stream<Item = Result<axum::response::sse::Event, std::convert::Infallible>>> {
    use axum::response::sse::Event;
    use futures::stream;
    use std::time::Duration;

    let stream = stream::unfold((), |()| async {
        tokio::time::sleep(Duration::from_secs(2)).await;
        let engine = crate::openhuman::screen_intelligence::global_engine();
        let status = engine.status().await;
        let json = serde_json::to_string(&status).unwrap_or_else(|_| "{}".to_string());
        let event = Event::default().data(json).event("status");
        Some((Ok(event), ()))
    });

    axum::response::Sse::new(stream).keep_alive(
        axum::response::sse::KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("ping"),
    )
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
    println!("openhuman screen-intelligence — standalone screen intelligence runtime\n");
    println!("Boots only the screen intelligence engine (accessibility capture + local-AI");
    println!("vision) without the full desktop app, Socket.IO, or skills runtime.\n");
    println!("Usage:");
    println!("  openhuman screen-intelligence run       [--port <u16>] [-v]");
    println!("  openhuman screen-intelligence status     [-v]");
    println!("  openhuman screen-intelligence capture    [--keep] [-v]");
    println!("  openhuman screen-intelligence start      [--ttl <secs>] [-v]");
    println!("  openhuman screen-intelligence stop       [-v]");
    println!("  openhuman screen-intelligence doctor     [-v]");
    println!("  openhuman screen-intelligence vision     [--limit <n>] [-v]");
    println!();
    println!("Subcommands:");
    println!("  run       Start a dev server with JSON-RPC + REST + SSE endpoints");
    println!("  status    Print current engine status (permissions, session, config)");
    println!("  capture   Take a single screenshot and print diagnostics");
    println!("  start     Start a capture + vision session (runs until TTL or Ctrl+C)");
    println!("  stop      Stop the active session");
    println!("  doctor    Check system readiness (permissions, vision config, platform)");
    println!("  vision    Print recent vision summaries from the active session");
    println!();
    println!("Common options:");
    println!("  --port <u16>     Server port for 'run' (default: 7797)");
    println!("  --ttl <secs>     Session TTL for 'start' (default: 300)");
    println!("  --limit <n>      Max vision summaries for 'vision' (default: 10)");
    println!("  --keep           Save screenshot to disk (for 'capture')");
    println!("  -v, --verbose    Enable debug logging");
    println!();
    println!("The 'run' server exposes REST convenience endpoints at:");
    println!("  GET  /status, /permissions, /session, /vision/recent, /doctor, /config");
    println!("  POST /session/start, /session/stop, /capture, /capture/test, /vision/flush");
    println!("  GET  /events — SSE stream of session status updates (2s interval)");
}
