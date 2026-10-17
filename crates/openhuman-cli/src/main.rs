//! The `openhuman-core` binary.
//!
//! A host like the desktop app and the TUI: it depends on `openhuman-rpc`
//! alone and boots through the shared host entry
//! [`openhuman_rpc::host::cli`], which connects the TinyHumans backend,
//! registers the server launcher and the `http_host` controllers, and runs
//! the core's command-line dispatcher. This file only owns what is specific
//! to the process: SIGPIPE, the early `.env`, and (with `crash-reporting`)
//! the Sentry client, built from embed's shared `before_send` chain.

use openhuman_rpc::embed::process;

fn main() {
    restore_default_sigpipe();

    // Load `.env` before Sentry so a DSN defined only in the dotenv file is
    // visible at startup. Honors `OPENHUMAN_DOTENV_PATH` and never overwrites
    // variables already in the environment; the dispatcher loads it again
    // with the same rules, which is a no-op by then.
    if let Err(err) = process::load_dotenv_for_cli() {
        // Not fatal here: the dispatcher re-runs the load and reports it.
        log::debug!("[cli] early dotenv load failed: {err}");
    }

    // Local development only: link the module implementation crates in and
    // register them with the core's linked-module seam. They are served only
    // when the configuration asks (`OPENHUMAN_MODULE_SOURCE=linked`).
    #[cfg(feature = "linked-modules")]
    openhuman_modules_linked::install();

    // The guard must outlive everything, so it is bound in `main`.
    #[cfg(feature = "crash-reporting")]
    let _sentry_guard = init_sentry();

    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(err) = openhuman_rpc::host::cli(&args) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}

#[cfg(unix)]
fn restore_default_sigpipe() {
    // Rust ignores SIGPIPE at startup. That makes writes to a closed pipe
    // return EPIPE, which the print macros turn into a panic. CLI tools should
    // instead terminate quietly when a downstream reader such as `head` exits.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
}

#[cfg(not(unix))]
fn restore_default_sigpipe() {}

/// Start the Sentry client with embed's shared filter chain. The fallback
/// user id is the core's credential identity slot (the default of
/// [`SentryConfig::new`](process::sentry::SentryConfig::new)); the primary
/// source is the Sentry scope the core binds at login and server boot.
#[cfg(feature = "crash-reporting")]
fn init_sentry() -> process::sentry::sdk::ClientInitGuard {
    let config = process::sentry::SentryConfig::new(
        process::sentry::core_dsn(
            option_env!("OPENHUMAN_CORE_SENTRY_DSN"),
            option_env!("OPENHUMAN_SENTRY_DSN"),
        ),
        process::sentry::release_tag(
            env!("CARGO_PKG_VERSION"),
            option_env!("OPENHUMAN_BUILD_SHA"),
        ),
        process::sentry::resolve_environment(std::env::var("OPENHUMAN_APP_ENV").ok()),
    );
    log::debug!(
        "[cli] sentry init environment={} dsn_set={}",
        config.environment,
        config.dsn.is_some()
    );
    process::sentry::sdk::init(process::sentry::client_options(config))
}

#[cfg(all(test, feature = "crash-reporting"))]
#[path = "main_tests.rs"]
mod tests;
