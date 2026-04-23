//! `openhuman screen-intelligence run` — start the standalone capture + vision loop.

use anyhow::Result;

use super::{is_help, parse_opts};

/// Delegates to [`crate::openhuman::screen_intelligence::server::run_standalone`],
/// which boots the engine, starts a capture session, and blocks in a
/// monitoring loop logging captures and vision summaries until TTL or Ctrl+C.
pub(super) fn run_server(args: &[String]) -> Result<()> {
    let (opts, rest) = parse_opts(args)?;

    if rest.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence run [--ttl <secs>] [--keep] [--no-vision-model] [-v]");
        println!();
        println!("Start the screen intelligence capture + vision loop.");
        println!("Captures screenshots at baseline FPS, runs OCR and vision analysis,");
        println!("and logs summaries. Blocks until TTL expires or Ctrl+C.");
        println!();
        println!("  --ttl <secs>        Session duration (default: 300)");
        println!("  --keep              Keep screenshots on disk after vision processing");
        println!("  --no-vision-model   Skip the vision LLM — use OCR + text LLM only");
        println!("  --ocr-only          Alias for --no-vision-model");
        println!("  -v, --verbose       Enable debug logging");
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
        let mut config = crate::openhuman::config::Config::load_or_init()
            .await
            .map_err(|e| anyhow::anyhow!("config load failed: {e}"))?;

        if opts.no_vision_model {
            config.screen_intelligence.use_vision_model = false;
        }

        let keep_screenshots = opts.keep || config.screen_intelligence.keep_screenshots;

        let server_config = crate::openhuman::screen_intelligence::server::SiServerConfig {
            ttl_secs: opts.ttl_secs,
            log_interval_secs: 5,
            keep_screenshots,
        };

        let mode_label = if config.screen_intelligence.use_vision_model {
            format!("vision LLM ({})", config.local_ai.vision_model_id)
        } else {
            "OCR + text LLM (no vision model)".to_string()
        };

        eprintln!();
        eprintln!("  Screen Intelligence");
        eprintln!("  ───────────────────");
        eprintln!("  TTL:              {}s", opts.ttl_secs);
        eprintln!("  Mode:             {}", mode_label);
        eprintln!(
            "  FPS:              {}",
            config.screen_intelligence.baseline_fps
        );
        eprintln!("  Keep screenshots: {}", keep_screenshots);
        eprintln!();
        eprintln!("  Capturing → Vision → Log. Press Ctrl+C to stop.");
        eprintln!();

        crate::openhuman::screen_intelligence::server::run_standalone(config, server_config)
            .await
            .map_err(|e| anyhow::anyhow!("{e}"))
    })
}
