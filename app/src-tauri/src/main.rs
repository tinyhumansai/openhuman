// Always run as the Windows GUI subsystem so launching the Tauri app — and
// every CEF helper subprocess re-exec'd from this binary — does not pop a
// console window. Without this, debug builds default to console-subsystem
// and each CEF role (renderer / GPU / utility) opens its own terminal.
// The `core` CLI subcommand path below re-attaches to the parent shell's
// console at runtime via AttachConsole, so command-line output still works.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

// ── Desktop (CEF) entry point ─────────────────────────────────────────────────
// On the CEF runtime, the main binary is re-exec'd as the renderer / GPU /
// utility helper subprocesses. The `cef_entry_point` macro short-circuits
// main() when CEF has passed `--type=<role>` in argv, routing straight into
// CEF's process dispatcher — our normal startup only runs for the browser
// process. The macro is a no-op relative to our own `core` subcommand
// multiplexing since that path never carries `--type=`.
#[tauri::cef_entry_point]
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some("core") {
        // CLI path: re-attach to the parent shell's console so eprintln!
        // output lands in the cmd/PowerShell window the user invoked us
        // from. No-op (and harmless) when launched without a parent
        // console, e.g. from Explorer.
        #[cfg(target_os = "windows")]
        attach_parent_console();

        if let Err(err) = openhuman::run_core_from_args(&args[2..]) {
            eprintln!("core process failed: {err}");
            std::process::exit(1);
        }
        return;
    }

    openhuman::run()
}

#[cfg(target_os = "windows")]
fn attach_parent_console() {
    use windows_sys::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
    // SAFETY: AttachConsole has no preconditions beyond a valid PID constant.
    // It returns 0 on failure (no parent console / already attached); both
    // outcomes are fine for our use — we just won't print anything in that
    // case, matching the GUI-launch behavior.
    unsafe {
        AttachConsole(ATTACH_PARENT_PROCESS);
    }
}
