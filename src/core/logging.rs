//! Logging for `openhuman run` (and other CLI paths that need stderr output).
//!
//! Without initializing a subscriber, `log::` and `tracing::` macros are no-ops.

use std::fmt;
use std::io::{self, IsTerminal};
use std::sync::Once;

use nu_ansi_term::{Color, Style};
use tracing::{Event, Level};
use tracing_subscriber::fmt::format::{FormatEvent, FormatFields, Writer};
use tracing_subscriber::fmt::FmtContext;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::registry::LookupSpan;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::Layer;

static INIT: Once = Once::new();

/// Default `RUST_LOG` when it is unset: either global levels or only the inline autocomplete module tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CliLogDefault {
    /// Typical server/CLI logging (`info`, or `debug` when `verbose`).
    Global,
    /// Silence other modules; only `openhuman_core::openhuman::autocomplete::*` emits logs.
    AutocompleteOnly,
}

/// `14:32:01 <INFO> (jsonrpc) message…` — colors when stderr is a TTY.
struct CleanCliFormat;

impl<S, N> FormatEvent<S, N> for CleanCliFormat
where
    S: tracing::Subscriber + for<'a> LookupSpan<'a>,
    N: for<'a> FormatFields<'a> + 'static,
{
    fn format_event(
        &self,
        ctx: &FmtContext<'_, S, N>,
        mut writer: Writer<'_>,
        event: &Event<'_>,
    ) -> fmt::Result {
        let meta = event.metadata();
        let time = chrono::Local::now().format("%H:%M:%S");
        let level = level_tag(meta.level());
        let target = short_target(meta.target());

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

            let scope = target.to_string();
            let scope_styled = Style::new().fg(Color::Fixed(247)).paint(scope);
            write!(writer, "{scope_styled} ")?;
        } else {
            write!(writer, "{time}:{level}:{target} ")?;
        }

        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

fn level_tag(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "ERR",
        Level::WARN => "WRN",
        Level::INFO => "INF",
        Level::DEBUG => "DBG",
        Level::TRACE => "TRC",
    }
}

fn short_target(target: &str) -> &str {
    target.rsplit("::").next().unwrap_or(target)
}

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

/// Initialize `tracing` + bridge the `log` crate so existing `log::info!` calls appear.
///
/// - If `RUST_LOG` is unset: uses [`CliLogDefault`] and `verbose` to pick a default filter string.
/// - Safe to call once; subsequent calls are ignored.
pub fn init_for_cli_run(verbose: bool, default_scope: CliLogDefault) {
    INIT.call_once(|| {
        if std::env::var_os("RUST_LOG").is_none() {
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

        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            match default_scope {
                CliLogDefault::Global => {
                    tracing_subscriber::EnvFilter::new(if verbose { "debug" } else { "info" })
                }
                CliLogDefault::AutocompleteOnly => {
                    let level = if verbose { "trace" } else { "debug" };
                    tracing_subscriber::EnvFilter::new(format!(
                        "off,openhuman_core::openhuman::autocomplete={level}"
                    ))
                }
            }
        });

        let use_color = io::stderr().is_terminal();
        let file_constraints = parse_log_file_constraints();

        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(use_color)
            .event_format(CleanCliFormat)
            .with_filter(tracing_subscriber::filter::filter_fn(move |meta| {
                event_matches_file_constraints(meta, &file_constraints)
            }));

        let sentry_layer =
            sentry::integrations::tracing::layer().event_filter(|md: &tracing::Metadata<'_>| {
                match *md.level() {
                    Level::ERROR => sentry::integrations::tracing::EventFilter::Event,
                    Level::WARN | Level::INFO => {
                        sentry::integrations::tracing::EventFilter::Breadcrumb
                    }
                    _ => sentry::integrations::tracing::EventFilter::Ignore,
                }
            });

        let _ = tracing_subscriber::registry()
            .with(filter)
            .with(fmt_layer)
            .with(sentry_layer)
            .try_init();

        let _ = tracing_log::LogTracer::init();
    });
}
