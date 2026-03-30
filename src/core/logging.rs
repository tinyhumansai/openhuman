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
use tracing_subscriber::registry::LookupSpan;

static INIT: Once = Once::new();

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
            write!(writer, "{time_styled} ")?;

            let tag = format!("<{level}>");
            let level_styled = match *meta.level() {
                Level::ERROR => Style::new().fg(Color::Red).bold().paint(tag),
                Level::WARN => Style::new().fg(Color::Yellow).bold().paint(tag),
                Level::INFO => Style::new().fg(Color::Green).paint(tag),
                Level::DEBUG => Style::new().fg(Color::Cyan).paint(tag),
                Level::TRACE => Style::new().fg(Color::Magenta).dimmed().paint(tag),
            };
            write!(writer, "{level_styled} ")?;

            let scope = format!("({target})");
            let scope_styled = Style::new().fg(Color::Fixed(247)).paint(scope);
            write!(writer, "{scope_styled} ")?;
        } else {
            write!(writer, "{time} <{level}> ({target}) ")?;
        }

        ctx.field_format().format_fields(writer.by_ref(), event)?;
        writeln!(writer)
    }
}

fn level_tag(level: &Level) -> &'static str {
    match *level {
        Level::ERROR => "ERROR",
        Level::WARN => "WARN",
        Level::INFO => "INFO",
        Level::DEBUG => "DEBUG",
        Level::TRACE => "TRACE",
    }
}

fn short_target(target: &str) -> &str {
    target.rsplit("::").next().unwrap_or(target)
}

/// Initialize `tracing` + bridge the `log` crate so existing `log::info!` calls appear.
///
/// - If `RUST_LOG` is unset: uses `info`, or `debug` when `verbose` is true.
/// - Safe to call once; subsequent calls are ignored.
pub fn init_for_cli_run(verbose: bool) {
    INIT.call_once(|| {
        if std::env::var_os("RUST_LOG").is_none() {
            std::env::set_var("RUST_LOG", if verbose { "debug" } else { "info" });
        }

        let filter = tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
            tracing_subscriber::EnvFilter::new(if verbose { "debug" } else { "info" })
        });

        let use_color = io::stderr().is_terminal();

        let _ = tracing_subscriber::fmt()
            .with_ansi(use_color)
            .with_env_filter(filter)
            .event_format(CleanCliFormat)
            .try_init();

        let _ = tracing_log::LogTracer::init();
    });
}
