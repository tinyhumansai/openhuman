//! CLI entry point for the tabbed terminal UI (`openhuman` / `tui` / `chat`).
//!
//! Parses flags, initializes **file-only** logging (the TUI owns the terminal —
//! see `logging::init_for_tui`), boots the core in-process with no transport and
//! no background services, resolves the target thread, and hands off to the
//! event loop in [`super::app`].

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};

use openhuman_core::core::runtime::{
    CoreBuilder, CoreRuntime, DomainSet, ServiceSet, AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS,
};
use openhuman_core::core::types::HostKind;

/// Entry point for the `openhuman-tui` executable.
///
/// Flags:
///   * `--thread <id>` — attach to an existing thread.
///   * `--new` — force a brand-new thread (default when `--thread` is absent).
///   * `--last` / `--resume` — resume the newest thread, optionally opening the picker.
///   * `--no-alt-screen` — render in the current terminal buffer.
///   * `--provider <id>` / `--model <id>` — transient inference overrides.
///   * a positional prompt — send immediately after startup.
///   * `-v` / `--verbose` — debug-level file logging.
pub fn run_from_cli(args: &[String]) -> anyhow::Result<()> {
    openhuman_core::core::cli::load_dotenv_for_cli()?;
    openhuman_core::platform::service::apply_startup_restart_delay_from_env();
    openhuman_core::security::keyring::init_master_key().map_err(anyhow::Error::msg)?;

    let mut thread_id: Option<String> = None;
    let mut force_new = false;
    let mut verbose = false;
    let mut resume_picker = false;
    let mut use_last = false;
    let mut no_alt_screen = false;
    let mut provider: Option<String> = None;
    let mut model: Option<String> = None;
    let mut prompt_parts = Vec::new();

    let mut i = 0usize;
    while i < args.len() {
        match args[i].as_str() {
            "--thread" => {
                thread_id = Some(
                    args.get(i + 1)
                        .ok_or_else(|| anyhow::anyhow!("missing value for --thread"))?
                        .clone(),
                );
                i += 2;
            }
            "--new" => {
                force_new = true;
                i += 1;
            }
            "--resume" => {
                resume_picker = true;
                i += 1;
            }
            "--last" => {
                use_last = true;
                i += 1;
            }
            "--no-alt-screen" => {
                no_alt_screen = true;
                i += 1;
            }
            "--provider" | "--provider-id" | "-p" => {
                provider = Some(option_value(args, i, args[i].as_str())?);
                i += 2;
            }
            "--model" | "--model-id" | "-m" => {
                model = Some(option_value(args, i, args[i].as_str())?);
                i += 2;
            }
            arg if arg.starts_with("--provider=") || arg.starts_with("--provider-id=") => {
                provider = Some(inline_option_value(arg)?);
                i += 1;
            }
            arg if arg.starts_with("--model=") || arg.starts_with("--model-id=") => {
                model = Some(inline_option_value(arg)?);
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
            other if other.starts_with('-') => {
                return Err(anyhow::anyhow!("unknown tui arg: {other}"));
            }
            prompt => {
                prompt_parts.push(prompt.to_string());
                i += 1;
            }
        }
    }

    openhuman_core::core::cli::set_transient_inference_overrides(
        provider.as_deref(),
        model.as_deref(),
    );

    // File-only logging — never stderr while the TUI owns the terminal.
    let data_dir = resolve_data_dir();
    let log_dir = openhuman_core::core::logging::init_for_tui(&data_dir, verbose);
    log::info!(
        "[tui] starting tabbed terminal UI (thread={:?} new={} logs={:?})",
        thread_id,
        force_new,
        log_dir
    );

    // A chat turn is a large async state machine that can delegate to
    // sub-agents; give the tokio workers the same roomy stack the server uses
    // so a nested turn cannot overflow the default 2 MiB stack.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS)
        .build()?;
    let options = super::app::LaunchOptions {
        initial_prompt: (!prompt_parts.is_empty()).then(|| prompt_parts.join(" ")),
        resume_picker,
        no_alt_screen,
    };
    rt.block_on(async_main(
        thread_id,
        force_new,
        use_last || resume_picker,
        options,
    ))
}

fn option_value(args: &[String], index: usize, flag: &str) -> anyhow::Result<String> {
    let value = args
        .get(index + 1)
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| anyhow::anyhow!("missing value for {flag}"))?;
    nonempty_option_value(value, flag)
}

fn inline_option_value(arg: &str) -> anyhow::Result<String> {
    let (flag, value) = arg
        .split_once('=')
        .ok_or_else(|| anyhow::anyhow!("missing value for {arg}"))?;
    nonempty_option_value(value, flag)
}

fn nonempty_option_value(value: &str, flag: &str) -> anyhow::Result<String> {
    let value = value.trim();
    if value.is_empty() {
        return Err(anyhow::anyhow!("empty value for {flag}"));
    }
    Ok(value.to_string())
}

async fn async_main(
    thread_flag: Option<String>,
    force_new: bool,
    prefer_existing: bool,
    options: super::app::LaunchOptions,
) -> anyhow::Result<()> {
    // The core reaches the hosted backend (login, billing, integrations) only
    // through the transport `openhuman-tinyhumans` installs; bind it to the
    // runtime explicitly rather than relying on the process global.
    let backend_transport =
        openhuman_tinyhumans::install(openhuman_tinyhumans::InstallOptions::default())?;

    // In-process core: full domains (channel.web_chat needs DomainGroup::Channels,
    // so harness() is not enough), no RPC transport, no background services.
    let runtime = Arc::new(
        CoreBuilder::new(HostKind::detect_standalone())
            .domains(DomainSet::full())
            .services(ServiceSet::none())
            .backend_transport(backend_transport)
            .build()
            .await?,
    );
    log::info!("[tui] core built (DomainSet::full, ServiceSet::none)");

    // ServiceSet::none intentionally skips channel startup. The TUI is itself
    // an interactive surface, so bridge approval, plan-review, artifact, and
    // agent progress events onto the same in-process web-channel stream.
    openhuman_core::web_chat::register_approval_surface_subscriber();
    openhuman_core::web_chat::register_artifact_surface_subscriber();

    let client_id = format!("tui-{}", short_hex());
    let thread_id = resolve_thread(&runtime, thread_flag, force_new, prefer_existing).await?;
    log::info!("[tui] resolved thread={thread_id} client_id={client_id}");

    // Subscribe BEFORE the first turn so no streamed event is missed.
    let web_rx = openhuman_core::web_chat::subscribe_web_channel_events();

    super::app::run(runtime, client_id, thread_id, web_rx, options).await
}

/// Resolve the thread to open: the `--thread` id (unless `--new`), otherwise a
/// freshly created thread.
async fn resolve_thread(
    runtime: &CoreRuntime,
    thread_flag: Option<String>,
    force_new: bool,
    prefer_existing: bool,
) -> anyhow::Result<String> {
    if let (Some(id), false) = (thread_flag.as_ref(), force_new) {
        log::debug!("[tui] attaching to existing thread {id}");
        return Ok(id.clone());
    }

    if prefer_existing && !force_new {
        match runtime.invoke("openhuman.threads_list", json!({})).await {
            Ok(listed) => {
                if let Some(id) = super::cockpit::array_at(&listed, &["threads", "items"])
                    .first()
                    .and_then(|thread| thread.get("id").or_else(|| thread.get("thread_id")))
                    .and_then(Value::as_str)
                {
                    return Ok(id.to_string());
                }
            }
            Err(error) => {
                log::warn!("[tui] openhuman.threads_list failed: {error} — starting a new thread")
            }
        }
    }

    let created = runtime
        .invoke("openhuman.threads_create_new", json!({}))
        .await
        .map_err(|e| anyhow::anyhow!("openhuman.threads_create_new failed: {e}"))?;
    extract_thread_id(&created).ok_or_else(|| {
        anyhow::anyhow!("openhuman.threads_create_new returned no thread id: {created}")
    })
}

/// Pull a thread id out of a `threads.create_new` / `threads.list` response,
/// tolerant of the `Outcome` log-envelope wrapping (`{result, logs}`) and the
/// `ApiEnvelope` data wrapping (`{data, meta}`).
pub(super) fn extract_thread_id(value: &Value) -> Option<String> {
    // Unwrap the optional `{result, logs}` log envelope first.
    let inner = value.get("result").unwrap_or(value);
    // Then the optional `{data, meta}` ApiEnvelope.
    let payload = inner.get("data").unwrap_or(inner);
    payload
        .get("id")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// Resolve the OpenHuman data dir (host of `logs/`), mirroring the shell's
/// resolution: `OPENHUMAN_WORKSPACE` override, else `~/.openhuman`, else a temp
/// fallback. No `eprintln!` — the TUI is about to take the terminal.
fn resolve_data_dir() -> PathBuf {
    if let Ok(workspace) = std::env::var("OPENHUMAN_WORKSPACE") {
        if !workspace.is_empty() {
            return PathBuf::from(workspace);
        }
    }
    openhuman_core::config::default_root_openhuman_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("openhuman"))
}

/// 12 hex chars of randomness for the client stream id.
fn short_hex() -> String {
    let u = uuid::Uuid::new_v4();
    u.simple().to_string()[..12].to_string()
}

fn print_help() {
    println!("Usage: openhuman-tui [OPTIONS] [PROMPT]");
    println!();
    println!("Open the tabbed terminal UI for core logs, orchestrator chat, configuration,");
    println!("and account settings. Runs the core in-process — no server, no ports.");
    println!();
    println!("  --thread <id>   Attach to an existing conversation thread.");
    println!("  --new           Force a new thread (default when --thread is omitted).");
    println!("  --resume        Open the saved-thread picker (starts on the latest thread).");
    println!("  --last          Resume the most recent thread.");
    println!("  --no-alt-screen Draw in the current terminal buffer.");
    println!("  -p, --provider <id>  Override the provider for this TUI session.");
    println!("  -m, --model <id>     Override the model for this TUI session.");
    println!("  -v, --verbose   Debug-level logging (written to the log file, never the UI).");
    println!();
    println!("Keys: Ctrl+Tab or Alt+1-4 switch tabs · Enter send · Shift+Enter newline ·");
    println!("      / opens commands · Ctrl+C / Ctrl+D quit.");
}

#[cfg(test)]
#[path = "runner_tests.rs"]
mod tests;
