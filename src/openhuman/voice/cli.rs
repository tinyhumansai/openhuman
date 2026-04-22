//! Voice CLI adapter — domain-owned.
//!
//! Handles the `openhuman voice` / `openhuman dictate` subcommand which runs a
//! long-lived, blocking standalone dictation server (hotkey → record →
//! transcribe → insert). This flow doesn't fit the request/response controller
//! registry pattern because it blocks forever on the hotkey listener, so the
//! adapter lives here inside the voice domain rather than in `src/core/cli.rs`.

use anyhow::{anyhow, Result};

use crate::core::logging::{init_for_cli_run, CliLogDefault};
use crate::openhuman::voice::hotkey::ActivationMode;
use crate::openhuman::voice::server::{run_standalone, VoiceServerConfig};

/// Parse and execute the `openhuman voice` / `openhuman dictate` subcommand.
///
/// Supported flags:
///   --hotkey <combo>   Key combination (default from config, usually `fn`)
///   --mode <tap|push>  Activation mode (default push)
///   --skip-cleanup     Skip LLM post-processing on transcriptions
///   -v / --verbose     Enable debug logging
///   -h / --help        Print usage
pub(crate) fn run_standalone_subcommand(args: &[String]) -> Result<()> {
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
                        .ok_or_else(|| anyhow!("missing value for --hotkey"))?
                        .clone(),
                );
                i += 2;
            }
            "--mode" => {
                mode = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow!("missing value for --mode"))?
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
                print_help();
                return Ok(());
            }
            other => return Err(anyhow!("unknown voice arg: {other}")),
        }
    }

    log::debug!(
        "[voice-cli] starting standalone server hotkey={:?} mode={:?} skip_cleanup={} verbose={}",
        hotkey,
        mode,
        skip_cleanup,
        verbose
    );

    init_for_cli_run(verbose, CliLogDefault::Global);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let mut config = match crate::openhuman::config::Config::load_or_init().await {
            Ok(cfg) => cfg,
            Err(e) => {
                log::warn!("[voice-cli] config load failed, using defaults: {e}");
                crate::openhuman::config::Config::default()
            }
        };
        config.apply_env_overrides();

        let activation_mode = match mode.as_deref() {
            Some("tap") => ActivationMode::Tap,
            Some("push") | None => ActivationMode::Push,
            Some(other) => return Err(anyhow!("invalid --mode '{other}', expected tap|push")),
        };

        let server_config = VoiceServerConfig {
            hotkey: hotkey.unwrap_or_else(|| config.voice_server.hotkey.clone()),
            activation_mode,
            skip_cleanup,
            context: None,
            min_duration_secs: config.voice_server.min_duration_secs,
            silence_threshold: config.voice_server.silence_threshold,
            custom_dictionary: config.voice_server.custom_dictionary.clone(),
        };

        run_standalone(config, server_config)
            .await
            .map_err(anyhow::Error::msg)
    })?;

    Ok(())
}

fn print_help() {
    println!("Usage: openhuman voice [--hotkey <combo>] [--mode <tap|push>] [--skip-cleanup] [-v]");
    println!();
    println!("  --hotkey <combo>   Key combination (default: fn)");
    println!("  --mode <tap|push>  Activation: tap to toggle, push to hold (default: push)");
    println!("  --skip-cleanup     Skip LLM post-processing on transcriptions");
    println!("  -v, --verbose      Enable debug logging");
    println!();
    println!("Standalone voice dictation server. Press the hotkey to dictate,");
    println!("transcribed text is inserted into the active text field.");
}
