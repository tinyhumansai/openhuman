//! Process-spawn helpers for local AI subsystems.
//!
//! On Windows every `Command::spawn` allocates a conhost for the child
//! before stdio inheritance is applied — so a `Stdio::null()` redirect
//! still flashes a console window. `apply_no_window` sets the
//! `CREATE_NO_WINDOW` (0x0800_0000) process-creation flag which tells
//! the OS to skip conhost allocation entirely.
//!
//! Mirrors the pattern established by #731 and #1338 for the Tauri-shell
//! side (see `app/src-tauri/src/core_process.rs` and `process_kill.rs`).
//! Without this every Ollama health-check / install attempt flashes a
//! console on Windows; on a fresh install without Ollama present the
//! flashes are continuous because the resolve-or-install loop retries.

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
pub(crate) fn apply_no_window(cmd: &mut tokio::process::Command) {
    use std::os::windows::process::CommandExt;
    cmd.creation_flags(CREATE_NO_WINDOW);
}

#[cfg(not(windows))]
pub(crate) fn apply_no_window(_cmd: &mut tokio::process::Command) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_no_window_is_callable_on_every_platform() {
        // The function is a no-op on non-Windows. On Windows it sets a
        // creation flag we cannot directly read back from
        // `tokio::process::Command`, so this test just guarantees the
        // helper compiles and is callable from generic code.
        let mut cmd = tokio::process::Command::new("does-not-need-to-exist");
        apply_no_window(&mut cmd);
    }
}
