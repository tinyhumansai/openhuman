//! Process lifecycle helpers every host repeats: the tokio runtime, logging,
//! dotenv and launch overrides, the master key, and (behind
//! `crash-reporting`) the Sentry client options.
//!
//! These are the steps the desktop shell, the CLI and the TUI each perform
//! before they build a [`Runtime`](crate::Runtime). Each is a thin wrapper
//! over the core function that owns the behaviour, so hosts reach them
//! through embed instead of the core's internal module paths.

use std::path::{Path, PathBuf};

pub use openhuman_core::core::runtime::{AGENT_WORKER_STACK_BYTES, MAX_BLOCKING_THREADS};

/// Scoped ownership for host commands outside an agent turn. After dropping
/// the scoped future, await `wait()` before acknowledging cancellation.
pub use tinyagents_harness_tools::exec::timeout::ProcessCleanup as CommandCleanup;

/// Sentry options and event scrubbing shared by desktop and terminal hosts.
#[cfg(feature = "crash-reporting")]
#[cfg_attr(docsrs, doc(cfg(feature = "crash-reporting")))]
#[path = "process_sentry.rs"]
pub mod sentry;

/// A multi-threaded tokio runtime builder sized for agent turns.
///
/// A turn is a very deep async state machine and delegating to a sub-agent
/// nests another one, which overflows tokio's default 2 MiB worker stack;
/// every host builds its runtime with [`AGENT_WORKER_STACK_BYTES`] and caps
/// blocking threads at [`MAX_BLOCKING_THREADS`]. Returned unbuilt so a host
/// can still name its threads.
pub fn tokio_runtime_builder() -> tokio::runtime::Builder {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder
        .enable_all()
        .thread_stack_size(AGENT_WORKER_STACK_BYTES)
        .max_blocking_threads(MAX_BLOCKING_THREADS);
    builder
}

/// [`tokio_runtime_builder`], built.
pub fn tokio_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    log::debug!(
        "[embed][process] building tokio runtime stack_bytes={AGENT_WORKER_STACK_BYTES} \
         max_blocking={MAX_BLOCKING_THREADS}"
    );
    tokio_runtime_builder().build()
}

/// Logging for a host that embeds the core in a GUI process: the unified
/// subscriber, a daily-rotated file under `<data_dir>/logs/` (with the
/// `file-logging` feature), the Sentry layer, and the `log` bridge.
/// Idempotent; the first logging init in a process wins.
pub fn init_for_embedded(data_dir: &Path, verbose: bool) {
    openhuman_core::core::logging::init_for_embedded(data_dir, verbose);
}

/// File-only logging for a host that owns the terminal. Returns the log
/// directory when the file appender came up.
pub fn init_for_tui(data_dir: &Path, verbose: bool) -> Option<PathBuf> {
    openhuman_core::core::logging::init_for_tui(data_dir, verbose)
}

/// The active log directory, once [`init_for_embedded`] or [`init_for_tui`]
/// set one.
pub fn log_directory() -> Option<&'static Path> {
    openhuman_core::core::logging::log_directory()
}

/// Release the log file handle (so a data-directory reset can remove it on
/// Windows). `true` when a guard was dropped. File logging stays off until
/// the next launch.
pub fn shutdown_file_guard() -> bool {
    openhuman_core::core::logging::shutdown_file_guard()
}

/// The bounded in-memory log stream a terminal Logs tab renders.
pub fn tui_log_lines() -> Vec<String> {
    openhuman_core::core::logging::tui_log_lines()
}

/// Load `OPENHUMAN_DOTENV_PATH` or a `.env` in the working directory without
/// overwriting variables already set.
pub fn load_dotenv_for_cli() -> anyhow::Result<()> {
    openhuman_core::core::cli::load_dotenv_for_cli()
}

/// Process-local provider/model overrides from launch flags; never written
/// back to the user's config.
pub fn set_transient_inference_overrides(provider: Option<&str>, model: Option<&str>) {
    openhuman_core::core::cli::set_transient_inference_overrides(provider, model);
}

/// Honour a restart delay requested by the previous process (self-update and
/// restart flows) before booting.
pub fn apply_startup_restart_delay_from_env() {
    openhuman_core::platform::service::apply_startup_restart_delay_from_env();
}

/// Load the master key when encrypted-file storage is selected. The default
/// in-memory backend needs no key. Core boot also calls this helper.
pub fn init_master_key() -> anyhow::Result<()> {
    openhuman_core::security::secret_storage::init_master_key().map_err(|error| {
        log::warn!("[embed][process] master key init failed");
        anyhow::Error::msg(error)
    })
}

#[cfg(test)]
#[path = "process_tests.rs"]
mod tests;

/// Run a host command with stdin and a deadline, owning its process group.
/// Normal completion, timeout, and cancellation all reap the command. When
/// called inside a cancellable turn, its cleanup is also tracked by that turn.
/// Explicit command environment settings are preserved over `Turn::tool_env`.
pub async fn command_output(
    command: &mut tokio::process::Command,
    input: Vec<u8>,
    deadline: std::time::Duration,
) -> std::io::Result<std::process::Output> {
    let cleanup = tinyagents_harness_tools::exec::timeout::ProcessCleanup::default();
    let result = cleanup
        .scope(tokio::time::timeout(
            deadline,
            tinyagents_harness_tools::exec::timeout::output_with_input(command, Some(input)),
        ))
        .await;
    cleanup.wait().await;
    result
        .map_err(|_| std::io::Error::new(std::io::ErrorKind::TimedOut, "host command timed out"))?
}
