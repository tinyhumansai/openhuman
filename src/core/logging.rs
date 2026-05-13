//! Logging for `openhuman run` (and other CLI paths that need stderr output).
//!
//! Without initializing a subscriber, `log::` and `tracing::` macros are no-ops.
//!
//! Two entry points share the same formatter and `EnvFilter`:
//!   * [`init_for_cli_run`] — stderr only, used by `openhuman run` / CLI
//!     subcommands.
//!   * [`init_for_embedded`] — stderr + a daily-rotated file under
//!     `<data_dir>/logs/openhuman-YYYY-MM-DD.log`, used by the Tauri shell
//!     where stderr is invisible in packaged builds. Both shell `log::*`
//!     calls and core `tracing::*` calls funnel into the same file via
//!     [`tracing_log::LogTracer`].

use std::fmt;
use std::io::{self, IsTerminal};
use std::path::{Path, PathBuf};
use std::sync::{Once, OnceLock};

use nu_ansi_term::{Color, Style};
use tracing::{Event, Level};
use tracing_appender::non_blocking::WorkerGuard;
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

static INIT: Once = Once::new();

/// Holds the non-blocking writer guard for the file appender. Must live for
/// the entire process lifetime — dropping it stops the background flushing
/// thread and silently swallows pending log records.
static FILE_GUARD: OnceLock<WorkerGuard> = OnceLock::new();

/// Resolved path to the active log file directory. Populated by
/// [`init_for_embedded`] so UI commands (e.g. `reveal_logs_folder`) can find
/// it without re-deriving the data dir.
static LOG_DIR: OnceLock<PathBuf> = OnceLock::new();

/// Default `RUST_LOG` when it is unset: either global levels or only the inline autocomplete module tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliLogDefault {
    /// Typical server/CLI logging (`info`, or `debug` when `verbose`).
    Global,
    /// Silence other modules; only `openhuman_core::openhuman::autocomplete::*` emits logs.
    AutocompleteOnly,
}

/// Custom log formatter for the OpenHuman CLI.
///
/// It produces a clean, readable output on stderr:
/// `14:32:01 INF:jsonrpc: Listening on http://127.0.0.1:7788`
///
/// It supports ANSI colors if the output is a terminal and `NO_COLOR` is not set.
struct CleanCliFormat;

impl<S, N> FormatEvent<S, N> for CleanCliFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    /// Formats a single tracing event into a string and writes it to the writer.
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        // Use local time for log timestamps.
        let time = chrono::Local::now().format("%H:%M:%S");
        let level = level_tag(meta.level());
        let target = short_target(meta.target());

        // Check if the writer supports ANSI escape codes for coloring.
        if writer.has_ansi_escapes() {
            let time_styled = Style::new().dimmed().paint(time.to_string());
            write!(writer, "{time_styled}:")?;

            let tag = level.to_string();
            let level_styled = match *meta.level() {
                Level::ERROR => Style::new().fg(Color::Red).bold().paint(tag),
                Level::WARN => Style::new().fg(Color::Yellow).bold().paint(tag),
                Level::INFO => Style::new().fg(Color::Green).paint(tag),
                Level::DEBUG => Style::new().fg(Color::Cyan).paint(tag),
                Level::TRACE => Style::new().fg(Color::Magenta).dimmed().paint(tag),
            };
            write!(writer, "{level_styled}:")?;

            // Scope color: pick a neutral gray for the module name.
            let scope = target.to_string();
            let scope_styled = Style::new().fg(Color::Fixed(247)).paint(scope);
            write!(writer, "{scope_styled} ")?;
        } else {
            // Plain text fallback (e.g., when logging to a file or non-TTY).
            write!(writer, "{time}:{level}:{target} ")?;
        }

        // Write the actual log message and its fields.
        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

/// Returns a 3-letter uppercase tag for each log level.
fn level_tag(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "ERR",
        Level::WARN => "WRN",
        Level::INFO => "INF",
        Level::DEBUG => "DBG",
        Level::TRACE => "TRC",
    }
}

/// Shortens a Rust module path (e.g., `openhuman_core::rpc` -> `rpc`).
fn short_target(target: &str) -> &str {
    target.rsplit("::").next().unwrap_or(target)
}

/// Parses a comma-separated list of file/module constraints from environment.
///
/// Used to filter logs to specific parts of the codebase.
fn parse_log_file_constraints() -> Vec<String> {
    std::env::var("OPENHUMAN_LOG_FILE_CONSTRAINTS")
        .ok()
        .map(|raw| {
            raw.split(',')
                .map(str::trim)
                .filter(|v| !v.is_empty())
                .map(ToOwned::to_owned)
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

/// Checks if a log event matches any of the configured file/module constraints.
fn event_matches_file_constraints(meta: &tracing::Metadata<'_>, constraints: &[String]) -> bool {
    if constraints.is_empty() {
        return true;
    }

    let file = meta.file().unwrap_or_default();
    let target = meta.target();
    constraints
        .iter()
        .any(|constraint| file.contains(constraint) || target.contains(constraint))
}

/// Initialize the global `tracing` subscriber and bridge the `log` crate.
///
/// This function:
/// 1. Determines the default log level based on `verbose` and `default_scope`.
/// 2. Sets up an `EnvFilter` from `RUST_LOG` or the defaults.
/// 3. Detects terminal capabilities for ANSI colors.
/// 4. Registers a formatting layer with [`CleanCliFormat`].
/// 5. Integrates Sentry for error tracking.
/// 6. Bridges legacy `log::info!` macros.
///
/// It is idempotent and will only initialize the subscriber once per process.
pub fn init_for_cli_run(verbose: bool, default_scope: CliLogDefault) {
    INIT.call_once(|| {
        seed_rust_log(verbose, default_scope);
        let filter = build_env_filter(verbose, default_scope);

        // Color resolution logic.
        let use_color = if std::env::var_os("NO_COLOR").is_some() {
            false
        } else if std::env::var_os("FORCE_COLOR").is_some()
            || std::env::var_os("CLICOLOR_FORCE").is_some()
        {
            true
        } else {
            // Auto-detect based on stderr terminal status.
            io::stderr().is_terminal()
        };

        let cli_constraints = parse_log_file_constraints();
        // Build the primary formatting layer.
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(use_color)
            .event_format(CleanCliFormat)
            .with_filter(tracing_subscriber::filter::filter_fn(move |meta| {
                event_matches_file_constraints(meta, &cli_constraints)
            }));

        // Register the subscriber with all layers.
        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(sentry_tracing_layer())
            .try_init();

        // Bridge the `log` crate.
        let _ = tracing_log::LogTracer::init();
    });
}

/// Initialize logging for the embedded core running inside the Tauri shell.
///
/// Installs:
///   * a stderr layer (for `tauri dev` / terminal launches), with ANSI when
///     attached to a TTY,
///   * a non-blocking, daily-rotated file appender at
///     `<data_dir>/logs/openhuman-YYYY-MM-DD.log` so packaged GUI builds —
///     where stderr is invisible — still produce a log users can share for
///     support,
///   * the Sentry breadcrumb/event layer,
///   * the `tracing_log::LogTracer` bridge so the Tauri shell's `log::*`
///     calls (currently routed through `env_logger`) flow into the same
///     file alongside core `tracing::*` events.
///
/// Idempotent (`Once`-guarded). Safe to call from `run()` multiple times
/// across re-execs; subsequent calls are no-ops. The first caller wins, so
/// the Tauri shell should call this before any CLI path could initialize a
/// stderr-only subscriber.
pub fn init_for_embedded(data_dir: &Path, verbose: bool) {
    INIT.call_once(|| {
        let scope = CliLogDefault::Global;
        seed_rust_log(verbose, scope);
        let filter = build_env_filter(verbose, scope);

        let logs_dir = data_dir.join("logs");
        // Build the file appender first, but keep the writer guard + path in
        // locals — only commit to `FILE_GUARD` / `LOG_DIR` after `try_init()`
        // succeeds. Otherwise a competing global subscriber would cause
        // `try_init` to return Err and `log_directory()` would still report a
        // path even though no file layer is attached. Errors are surfaced via
        // `eprintln!` (the tracing subscriber isn't installed yet here) using
        // the same `[logging]` prefix as the dir-creation diagnostic.
        let pending_file: Option<(_, tracing_appender::non_blocking::WorkerGuard, PathBuf)> =
            match std::fs::create_dir_all(&logs_dir) {
                Ok(()) => match tracing_appender::rolling::Builder::new()
                    .rotation(tracing_appender::rolling::Rotation::DAILY)
                    .filename_prefix("openhuman")
                    .filename_suffix("log")
                    .max_log_files(7)
                    .build(&logs_dir)
                {
                    Ok(appender) => {
                        let (writer, guard) = tracing_appender::non_blocking(appender);
                        Some((writer, guard, logs_dir.clone()))
                    }
                    Err(err) => {
                        eprintln!(
                            "[logging] failed to create file appender in {}: {err}",
                            logs_dir.display()
                        );
                        None
                    }
                },
                Err(err) => {
                    eprintln!(
                        "[logging] failed to create logs dir {}: {err}",
                        logs_dir.display()
                    );
                    None
                }
            };

        let file_layer = pending_file.as_ref().map(|(writer, _, _)| {
            let constraints = parse_log_file_constraints();
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .event_format(CleanCliFormat)
                .with_writer(writer.clone())
                .with_filter(tracing_subscriber::filter::filter_fn(move |meta| {
                    event_matches_file_constraints(meta, &constraints)
                }))
        });

        // Stderr layer: useful for `tauri dev` and CLI-style launches. ANSI
        // only when stderr is a real terminal.
        let stderr_constraints = parse_log_file_constraints();
        let stderr_layer = tracing_subscriber::fmt::layer()
            .with_ansi(io::stderr().is_terminal() && std::env::var_os("NO_COLOR").is_none())
            .event_format(CleanCliFormat)
            .with_filter(tracing_subscriber::filter::filter_fn(move |meta| {
                event_matches_file_constraints(meta, &stderr_constraints)
            }));

        match tracing_subscriber::registry()
            .with(filter)
            .with(stderr_layer)
            .with(file_layer)
            .with(sentry_tracing_layer())
            .try_init()
        {
            Ok(()) => {
                if let Some((_, guard, dir)) = pending_file {
                    let _ = FILE_GUARD.set(guard);
                    let _ = LOG_DIR.set(dir);
                }
            }
            Err(err) => {
                // Another global subscriber was already installed (rare —
                // typically a pre-existing CLI init in the same process).
                // Drop the writer guard so the background flushing thread
                // shuts down cleanly, and leave LOG_DIR unset so the UI
                // surfaces "logging not initialized" instead of pointing at
                // an empty directory.
                eprintln!("[logging] tracing subscriber init failed: {err}");
            }
        }

        let _ = tracing_log::LogTracer::init();
    });
}

/// Path to the active log directory (set by [`init_for_embedded`]). Returns
/// `None` if logging hasn't been initialized in embedded mode (e.g. bare
/// CLI runs).
pub fn log_directory() -> Option<&'static Path> {
    LOG_DIR.get().map(PathBuf::as_path)
}

fn seed_rust_log(verbose: bool, default_scope: CliLogDefault) {
    if std::env::var_os("RUST_LOG").is_some() {
        return;
    }
    let default = match default_scope {
        CliLogDefault::Global => {
            if verbose {
                "debug".to_string()
            } else {
                "info".to_string()
            }
        }
        CliLogDefault::AutocompleteOnly => {
            let level = if verbose { "trace" } else { "debug" };
            format!("off,openhuman_core::openhuman::autocomplete={level}")
        }
    };
    std::env::set_var("RUST_LOG", default);
}

fn build_env_filter(verbose: bool, default_scope: CliLogDefault) -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| match default_scope {
        CliLogDefault::Global => {
            tracing_subscriber::EnvFilter::new(if verbose { "debug" } else { "info" })
        }
        CliLogDefault::AutocompleteOnly => {
            let level = if verbose { "trace" } else { "debug" };
            tracing_subscriber::EnvFilter::new(format!(
                "off,openhuman_core::openhuman::autocomplete={level}"
            ))
        }
    })
}

fn sentry_tracing_layer<S>() -> impl Layer<S>
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
{
    sentry::integrations::tracing::layer().event_filter(|md: &tracing::Metadata<'_>| {
        // Events emitted from `report_error_message` are captured directly via
        // `sentry::capture_message` at the call site (see
        // `core::observability::REPORT_ERROR_TRACING_TARGET` for rationale).
        // Skip them here so we don't double-report.
        if md.target() == crate::core::observability::REPORT_ERROR_TRACING_TARGET {
            return sentry::integrations::tracing::EventFilter::Ignore;
        }
        match *md.level() {
            Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
            Level::WARN | Level::INFO => sentry::integrations::tracing::EventFilter::Breadcrumb,
            _ => sentry::integrations::tracing::EventFilter::Ignore,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Serialize tests that mutate `RUST_LOG` / `OPENHUMAN_LOG_FILE_CONSTRAINTS` —
    /// Cargo runs unit tests in parallel threads in the same process, so
    /// concurrent env-var writes would race.
    static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    fn with_clean_rust_log<R>(f: impl FnOnce() -> R) -> R {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var("RUST_LOG").ok();
        std::env::remove_var("RUST_LOG");
        let result = f();
        match prior {
            Some(v) => std::env::set_var("RUST_LOG", v),
            None => std::env::remove_var("RUST_LOG"),
        }
        result
    }

    #[test]
    fn level_tag_covers_all_levels() {
        assert_eq!(level_tag(&Level::ERROR), "ERR");
        assert_eq!(level_tag(&Level::WARN), "WRN");
        assert_eq!(level_tag(&Level::INFO), "INF");
        assert_eq!(level_tag(&Level::DEBUG), "DBG");
        assert_eq!(level_tag(&Level::TRACE), "TRC");
    }

    #[test]
    fn short_target_strips_module_path() {
        assert_eq!(short_target("openhuman_core::core::rpc"), "rpc");
        // Non-namespaced target stays as-is.
        assert_eq!(short_target("plain"), "plain");
    }

    #[test]
    fn seed_rust_log_global_uses_info_by_default() {
        with_clean_rust_log(|| {
            seed_rust_log(false, CliLogDefault::Global);
            assert_eq!(std::env::var("RUST_LOG").unwrap(), "info");
        });
    }

    #[test]
    fn seed_rust_log_global_uses_debug_when_verbose() {
        with_clean_rust_log(|| {
            seed_rust_log(true, CliLogDefault::Global);
            assert_eq!(std::env::var("RUST_LOG").unwrap(), "debug");
        });
    }

    #[test]
    fn seed_rust_log_autocomplete_scopes_to_module() {
        with_clean_rust_log(|| {
            seed_rust_log(false, CliLogDefault::AutocompleteOnly);
            assert_eq!(
                std::env::var("RUST_LOG").unwrap(),
                "off,openhuman_core::openhuman::autocomplete=debug"
            );
        });
        with_clean_rust_log(|| {
            seed_rust_log(true, CliLogDefault::AutocompleteOnly);
            assert_eq!(
                std::env::var("RUST_LOG").unwrap(),
                "off,openhuman_core::openhuman::autocomplete=trace"
            );
        });
    }

    #[test]
    fn seed_rust_log_respects_existing_value() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var("RUST_LOG").ok();
        std::env::set_var("RUST_LOG", "warn");
        seed_rust_log(true, CliLogDefault::Global);
        // Caller's existing setting must not be clobbered.
        assert_eq!(std::env::var("RUST_LOG").unwrap(), "warn");
        match prior {
            Some(v) => std::env::set_var("RUST_LOG", v),
            None => std::env::remove_var("RUST_LOG"),
        }
    }

    #[test]
    fn build_env_filter_returns_a_filter() {
        // Smoke test: shouldn't panic and should produce *some* filter regardless of inputs.
        let _ = build_env_filter(false, CliLogDefault::Global);
        let _ = build_env_filter(true, CliLogDefault::AutocompleteOnly);
    }

    #[test]
    fn parse_log_file_constraints_handles_csv_and_whitespace() {
        let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        let prior = std::env::var("OPENHUMAN_LOG_FILE_CONSTRAINTS").ok();
        std::env::set_var("OPENHUMAN_LOG_FILE_CONSTRAINTS", "rpc, , agent ,memory");
        let parsed = parse_log_file_constraints();
        assert_eq!(parsed, vec!["rpc", "agent", "memory"]);

        std::env::remove_var("OPENHUMAN_LOG_FILE_CONSTRAINTS");
        assert!(parse_log_file_constraints().is_empty());

        match prior {
            Some(v) => std::env::set_var("OPENHUMAN_LOG_FILE_CONSTRAINTS", v),
            None => std::env::remove_var("OPENHUMAN_LOG_FILE_CONSTRAINTS"),
        }
    }

    #[test]
    fn log_directory_is_none_before_init_for_embedded() {
        // In a fresh `cargo test` process where no test has called
        // `init_for_embedded`, `log_directory()` must return `None` so the
        // shell-side `reveal_logs_folder` command can surface a clear
        // error rather than launching against an empty path.
        if LOG_DIR.get().is_none() {
            assert!(log_directory().is_none());
        }
    }
}
