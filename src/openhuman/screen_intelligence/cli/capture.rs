//! Capture + vision inspection subcommands: `capture`, `vision`.

use anyhow::Result;

use super::{bootstrap_engine, init_quiet_logging, is_help, parse_opts};

/// `openhuman screen-intelligence capture` — take a single screenshot and print info.
pub(super) fn run_capture(args: &[String]) -> Result<()> {
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
                let config = crate::openhuman::config::Config::load_or_init()
                    .await
                    .map_err(|e| anyhow::anyhow!("config load failed: {e}"))?;

                match crate::openhuman::screen_intelligence::AccessibilityEngine::save_capture_test_result(
                    &config.workspace_dir,
                    &result,
                    "cli_capture",
                ) {
                    Some(Ok(path)) => eprintln!("  Saved:   {}", path.display()),
                    Some(Err(e)) => eprintln!("  Save failed: {e}"),
                    None => {}
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

/// `openhuman screen-intelligence vision` — inspect recent vision summaries.
pub(super) fn run_vision(args: &[String]) -> Result<()> {
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
