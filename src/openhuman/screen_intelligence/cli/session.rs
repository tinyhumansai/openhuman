//! Session lifecycle subcommands: `start`, `stop`, `status`.

use anyhow::Result;

use super::{
    bootstrap_engine, bootstrap_engine_with_opts, init_quiet_logging, is_help, parse_opts,
};

/// `openhuman screen-intelligence status` — print current engine status as JSON.
pub(super) fn run_status(args: &[String]) -> Result<()> {
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

/// `openhuman screen-intelligence start` — start a capture + vision session.
pub(super) fn run_start_session(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!(
            "Usage: openhuman screen-intelligence start [--ttl <secs>] [--no-vision-model] [-v]"
        );
        println!();
        println!("Start a screen intelligence capture session with vision analysis.");
        println!("The session runs until TTL expires or Ctrl+C is pressed.");
        println!();
        println!("  --ttl <secs>        Session duration (default: 300, max: 3600)");
        println!("  --no-vision-model   Skip the vision LLM — use OCR + text LLM only");
        println!("  --ocr-only          Alias for --no-vision-model");
        println!("  -v, --verbose       Enable debug logging");
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
        let engine = bootstrap_engine_with_opts(opts.verbose, opts.no_vision_model).await?;

        let params = crate::openhuman::screen_intelligence::StartSessionParams {
            consent: true,
            ttl_secs: Some(opts.ttl_secs),
            screen_monitoring: Some(true),
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
pub(super) fn run_stop_session(args: &[String]) -> Result<()> {
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
