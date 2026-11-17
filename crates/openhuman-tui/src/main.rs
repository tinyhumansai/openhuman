//! Binary entry point for the standalone terminal frontend; all logic lives in
//! the `openhuman_tui` library (see `lib.rs` and `README.md`).

fn main() {
    // Keep the guard alive for the entire terminal session. `init_for_tui`
    // installs the tracing layer, while this creates the client that receives
    // its events and installs Sentry's panic integration.
    let args = std::env::args().skip(1).collect::<Vec<_>>();
    let _sentry_guard = if args
        .iter()
        .any(|arg| matches!(arg.as_str(), "--demo" | "--bench" | "--no-telemetry"))
    {
        None
    } else {
        Some(openhuman_tui::init_crash_reporting())
    };
    if let Err(error) = openhuman_tui::run_from_cli(&args) {
        eprintln!("Error: {error:#}");
        std::process::exit(1);
    }
}
