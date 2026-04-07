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
    verbose: bool,
    ttl_secs: u64,
    keep: bool,
    limit: usize,
}

fn parse_opts(args: &[String]) -> Result<(CliOpts, Vec<String>)> {
    let mut verbose = false;
    let mut ttl_secs: u64 = 300;
    let mut keep = false;
    let mut limit: usize = 10;
    let mut rest = Vec::new();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
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

/// `openhuman screen-intelligence run` — start the standalone capture + vision loop.
///
/// Delegates to [`crate::openhuman::screen_intelligence::server::run_standalone`],
/// which boots the engine, starts a capture session, and blocks in a
/// monitoring loop logging captures and vision summaries until TTL or Ctrl+C.
fn run_server(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence run [--ttl <secs>] [--keep] [-v]");
        println!();
        println!("Start the screen intelligence capture + vision loop.");
        println!("Captures screenshots at baseline FPS, sends to vision model,");
        println!("and logs summaries. Blocks until TTL expires or Ctrl+C.");
        println!();
        println!("  --ttl <secs>     Session duration (default: 300)");
        println!("  --keep           Keep screenshots on disk after vision processing");
        println!("  -v, --verbose    Enable debug logging");
        return Ok(());
    }

    crate::core::logging::init_for_cli_run(
        opts.verbose,
        crate::core::logging::CliLogDefault::Global,
    );

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| anyhow::anyhow!("config load failed: {e}"))?;

        let server_config = crate::openhuman::screen_intelligence::server::SiServerConfig {
            ttl_secs: opts.ttl_secs,
            log_interval_secs: 5,
            keep_screenshots: opts.keep,
        };

        eprintln!();
        eprintln!("  Screen Intelligence");
        eprintln!("  ───────────────────");
        eprintln!("  TTL:              {}s", opts.ttl_secs);
        eprintln!(
            "  Vision:           {}",
            config.screen_intelligence.vision_enabled
        );
        eprintln!("  Vision model:     {}", config.local_ai.vision_model_id);
        eprintln!(
            "  FPS:              {}",
            config.screen_intelligence.baseline_fps
        );
        eprintln!(
            "  Keep screenshots: {}",
            opts.keep || config.screen_intelligence.keep_screenshots
        );
        eprintln!();
        eprintln!("  Capturing → Vision → Log. Press Ctrl+C to stop.");
        eprintln!();

        crate::openhuman::screen_intelligence::server::run_standalone(config, server_config)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
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
    crate::core::logging::init_for_cli_run(
        opts.verbose,
        crate::core::logging::CliLogDefault::Global,
    );

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
                        let truncated = if summary.chars().count() > 100 {
                            format!("{}…", summary.chars().take(100).collect::<String>())
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
            crate::openhuman::screen_intelligence::rpc::accessibility_doctor_cli_json()
                .await
                .map_err(|e| anyhow::anyhow!("doctor check failed: {e}"))?;

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

        // Vision config check.
        let config = crate::openhuman::config::Config::load_or_init().await.ok();
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
                eprintln!("    ⚠  Vision is enabled but local_ai.enabled=false — vision analysis will fail");
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

        // Machine-readable JSON output.
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
                    let truncated = if s.ui_state.chars().count() > 120 {
                        format!("{}…", s.ui_state.chars().take(120).collect::<String>())
                    } else {
                        s.ui_state.clone()
                    };
                    eprintln!("       ui: {truncated}");
                }
                if !s.actionable_notes.is_empty() {
                    let truncated = if s.actionable_notes.chars().count() > 120 {
                        format!(
                            "{}…",
                            s.actionable_notes.chars().take(120).collect::<String>()
                        )
                    } else {
                        s.actionable_notes.clone()
                    };
                    eprintln!("       notes: {truncated}");
                }
                eprintln!();
            }
        }

        // Machine-readable output.
        println!(
            "{}",
            serde_json::to_string_pretty(&result).unwrap_or_default()
        );
        Ok(())
    })
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Quiet logging: only `warn` unless verbose (used for non-server subcommands).
fn init_quiet_logging(verbose: bool) {
    if !verbose && std::env::var_os("RUST_LOG").is_none() {
        std::env::set_var("RUST_LOG", "warn");
    }
    crate::core::logging::init_for_cli_run(verbose, crate::core::logging::CliLogDefault::Global);
}

fn is_help(value: &str) -> bool {
    matches!(value, "-h" | "--help" | "help")
}

fn print_help() {
    println!("openhuman screen-intelligence — standalone screen intelligence runtime\n");
    println!("Boots only the screen intelligence engine (accessibility capture + local-AI");
    println!("vision) without the full desktop app, Socket.IO, or skills runtime.\n");
    println!("Usage:");
    println!("  openhuman screen-intelligence run       [--ttl <secs>] [-v]");
    println!("  openhuman screen-intelligence status     [-v]");
    println!("  openhuman screen-intelligence capture    [--keep] [-v]");
    println!("  openhuman screen-intelligence start      [--ttl <secs>] [-v]");
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
    println!("  --ttl <secs>     Session TTL (default: 300)");
    println!("  --limit <n>      Max vision summaries for 'vision' (default: 10)");
    println!("  --keep           Save screenshot to disk (for 'capture')");
    println!("  -v, --verbose    Enable debug logging");
}
