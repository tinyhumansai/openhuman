// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

// On the CEF runtime, the main binary is re-exec'd as the renderer / GPU /
// utility helper subprocesses. The `cef_entry_point` macro short-circuits
// main() when CEF has passed `--type=<role>` in argv, routing straight into
// CEF's process dispatcher — our normal startup only runs for the browser
// process. The macro is a no-op relative to our own `core` subcommand
// multiplexing since that path never carries `--type=`.
#[cfg_attr(feature = "cef", tauri::cef_entry_point)]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("core") {
        if let Err(err) = openhuman::run_core_from_args(&args[2..]) {
            eprintln!("core process failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    openhuman::run()
}
