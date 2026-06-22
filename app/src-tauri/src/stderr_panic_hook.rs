//! Broken-pipe-on-stderr crash hardening for the desktop GUI process.
//!
//! ## The bug (Sentry TAURI-RUST-F, message family
//! `panic: failed printing to stderr: The pipe is being closed. (os error 232)`)
//!
//! `std::io::stdio::print_to` (`library/std/src/io/stdio.rs`) turns a failed
//! stderr write into an unconditional `panic!("failed printing to stderr: {e}")`.
//! This panic is raised by the `eprint!` / `eprintln!` macros (and direct
//! `io::stderr().write*`) — NOT by the default panic printer, which ignores
//! stderr-write errors. So this is a *primary* panic from an explicit
//! diagnostic write hitting a dead stderr.
//!
//! In the GUI process (`openhuman::run`, the TAURI-RUST-F culprit) the
//! breakable handle is an inherited stderr *pipe*: the app is launched with its
//! stderr wired to a parent process (or a now-closed parent console). When that
//! parent end goes away, the next `eprintln!` anywhere in the `run()` call
//! graph fails with a broken-pipe errno (Windows `232` ERROR_NO_DATA / `109`
//! ERROR_BROKEN_PIPE, POSIX `32` EPIPE) and `print_to` panics on the main
//! thread — aborting an app the user never asked to close, over an external
//! condition (the parent going away).
//!
//! ## Why this is fixed at the write path, not in the panic hook
//!
//! A panic hook runs *during* unwinding; returning early from it does **not**
//! stop the unwind, and on the main thread the process still terminates. Worse,
//! an early return would skip the chained Sentry hook, hiding a crash that still
//! happens. A hook therefore cannot absorb this panic (Codex review, PR #3772).
//!
//! The real fix is to stop the write from erroring in the first place:
//! [`neutralize_broken_parent_stderr`] runs at GUI startup and, on Windows,
//! redirects an inherited stderr **pipe** to the `NUL` device. Subsequent
//! `eprintln!` writes then succeed (discarded) instead of EPIPE-panicking. GUI
//! diagnostics already flow to Sentry + file logging, not this pipe, so nothing
//! observable is lost — and console / file stderr (`FILE_TYPE_CHAR` /
//! `FILE_TYPE_DISK`, e.g. a dev console or a user `2> log.txt`) is left intact.
//!
//! [`install`] still installs a panic hook, but it **always** chains to the
//! previously-installed hook (Sentry's panic integration when called after
//! `sentry::init`). It never swallows a panic. For the broken-pipe family it
//! adds an informational breadcrumb so the context survives, but the panic is
//! still reported — defense-in-depth around the [`neutralize_broken_parent_stderr`]
//! source fix, never a substitute for it.

use std::panic::PanicHookInfo;

/// Extract the panic payload as a `&str` if it carries a string message.
///
/// Rust panic payloads are either `&'static str` (from `panic!("literal")`) or
/// `String` (from `panic!("{}", x)`). The stdlib stderr-write panic uses the
/// formatting form, so it is a `String`; we still check both to be safe.
fn payload_message<'a>(info: &'a PanicHookInfo<'_>) -> Option<&'a str> {
    let payload = info.payload();
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        Some(*s)
    } else {
        payload.downcast_ref::<String>().map(|s| s.as_str())
    }
}

/// Classify whether a panic message is the "stderr write failed because the
/// (parent) pipe is closed" case.
///
/// Used only to tag a diagnostic breadcrumb — the panic is reported either way.
/// We anchor on the stdlib's stable prefix `"failed printing to stderr"` (the
/// literal in `std::io::stdio::print_to`) AND a broken-pipe signal, matched
/// locale-independently via the embedded OS error code:
///   * `os error 232` — Windows `ERROR_NO_DATA` ("The pipe is being closed.")
///   * `os error 109` — Windows `ERROR_BROKEN_PIPE` ("The pipe has been ended.")
///   * `os error 32`  — POSIX `EPIPE` (defensive).
///
/// `std::io::Error`'s `Display` for an OS error always appends ` (os error N)`
/// regardless of system locale, so this matches the Chinese-locale variant
/// (`管道正在被关闭`) just as well as the English one.
pub(crate) fn is_broken_pipe_stderr_panic(message: &str) -> bool {
    if !message.contains("failed printing to stderr") {
        return false;
    }
    message.contains("(os error 232)")   // Windows ERROR_NO_DATA — "pipe is being closed"
        || message.contains("(os error 109)") // Windows ERROR_BROKEN_PIPE
        || message.contains("(os error 32)") // POSIX EPIPE (defensive)
}

/// Core panic-hook body, factored out so the always-chain contract is testable.
///
/// For the broken-pipe-on-stderr family it records an informational breadcrumb
/// for context, then — for **every** panic without exception — calls `chain`
/// (the previously-installed hook, i.e. Sentry's panic integration). It never
/// returns early and never swallows: this hook cannot stop the unwind, so its
/// only job is to make sure the crash is always reported. Prevention lives in
/// [`neutralize_broken_parent_stderr`].
fn handle_panic(info: &PanicHookInfo<'_>, chain: &dyn Fn(&PanicHookInfo<'_>)) {
    if let Some(message) = payload_message(info) {
        if is_broken_pipe_stderr_panic(message) {
            sentry::add_breadcrumb(sentry::Breadcrumb {
                category: Some("panic".into()),
                level: sentry::Level::Info,
                message: Some(
                    "broken-pipe stderr panic (parent stderr closed); \
                     neutralize_broken_parent_stderr should have prevented this"
                        .into(),
                ),
                ..Default::default()
            });
            log::debug!("[stderr-panic-guard] broken-pipe stderr panic observed: {message}");
        }
    }
    // ALWAYS chain — never hide a crash from Sentry.
    chain(info);
}

/// Install the panic hook. Chains to the previously-installed hook (Sentry's
/// panic integration when called after `sentry::init`) for *every* panic.
pub(crate) fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        handle_panic(info, &|i| previous(i));
    }));
}

/// Source-level fix for the broken-pipe-on-stderr abort (Windows).
///
/// If the process's stderr is an inherited **pipe** (`FILE_TYPE_PIPE`), redirect
/// `STD_ERROR_HANDLE` to the `NUL` device so later `eprintln!` writes can never
/// fail with a broken-pipe errno (and so never reach `print_to`'s `panic!`).
/// Console (`FILE_TYPE_CHAR`) and file (`FILE_TYPE_DISK`) stderr are left
/// untouched, preserving legitimate diagnostics. No-op on non-Windows and when
/// stderr is not a pipe.
///
/// Call this at the very start of GUI startup, before any `eprintln!` can fire.
#[cfg(target_os = "windows")]
pub(crate) fn neutralize_broken_parent_stderr() {
    use windows_sys::Win32::Foundation::{GENERIC_WRITE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{
        CreateFileW, GetFileType, FILE_SHARE_READ, FILE_SHARE_WRITE, FILE_TYPE_PIPE, OPEN_EXISTING,
    };
    use windows_sys::Win32::System::Console::{GetStdHandle, SetStdHandle, STD_ERROR_HANDLE};

    // SAFETY: all calls below are plain Win32 FFI with no preconditions beyond
    // valid constants. GetStdHandle returns a borrowed handle we do not close;
    // the NUL handle we open is intentionally leaked for the process lifetime
    // (it becomes the new stderr). Failures degrade to a no-op.
    unsafe {
        let stderr = GetStdHandle(STD_ERROR_HANDLE);
        if stderr == INVALID_HANDLE_VALUE || stderr.is_null() {
            return;
        }
        // Only redirect an inherited pipe — the breakable handle in the
        // TAURI-RUST-F GUI case. Leave a real console / file redirect alone.
        if GetFileType(stderr) != FILE_TYPE_PIPE {
            return;
        }

        // "NUL" wide string, NUL-terminated.
        let nul: [u16; 4] = [b'N' as u16, b'U' as u16, b'L' as u16, 0];
        let nul_handle = CreateFileW(
            nul.as_ptr(),
            GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE,
            std::ptr::null(),
            OPEN_EXISTING,
            0,
            std::ptr::null_mut(),
        );
        if nul_handle == INVALID_HANDLE_VALUE || nul_handle.is_null() {
            return;
        }
        // Redirect process stderr to NUL. Rust's std fetches STD_ERROR_HANDLE
        // per write on Windows, so this takes effect for all later eprintln!.
        SetStdHandle(STD_ERROR_HANDLE, nul_handle);
    }
}

/// No-op on non-Windows: the inherited-pipe broken-stderr abort is the Windows
/// GUI case (TAURI-RUST-F). POSIX broken-pipe is matched defensively in
/// [`is_broken_pipe_stderr_panic`] for breadcrumb tagging only.
#[cfg(not(target_os = "windows"))]
pub(crate) fn neutralize_broken_parent_stderr() {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    #[test]
    fn classifies_windows_pipe_closing_232_as_broken_pipe() {
        assert!(is_broken_pipe_stderr_panic(
            "failed printing to stderr: The pipe is being closed. (os error 232)"
        ));
    }

    #[test]
    fn classifies_chinese_locale_variant_via_os_error_code() {
        // Localized strerror, but the `(os error 232)` suffix is locale-stable.
        assert!(is_broken_pipe_stderr_panic(
            "failed printing to stderr: 管道正在被关闭。 (os error 232)"
        ));
    }

    #[test]
    fn classifies_windows_broken_pipe_109() {
        assert!(is_broken_pipe_stderr_panic(
            "failed printing to stderr: The pipe has been ended. (os error 109)"
        ));
    }

    #[test]
    fn classifies_posix_epipe_32() {
        assert!(is_broken_pipe_stderr_panic(
            "failed printing to stderr: Broken pipe (os error 32)"
        ));
    }

    #[test]
    fn does_not_classify_assertion_panic() {
        assert!(!is_broken_pipe_stderr_panic(
            "assertion `left == right` failed\n  left: 1\n right: 2"
        ));
    }

    #[test]
    fn does_not_classify_arbitrary_panic() {
        assert!(!is_broken_pipe_stderr_panic(
            "index out of bounds: the len is 0 but the index is 3"
        ));
    }

    #[test]
    fn does_not_classify_stdout_broken_pipe() {
        assert!(!is_broken_pipe_stderr_panic(
            "failed printing to stdout: The pipe is being closed. (os error 232)"
        ));
    }

    #[test]
    fn does_not_classify_stderr_panic_with_unrelated_error() {
        assert!(!is_broken_pipe_stderr_panic(
            "failed printing to stderr: Permission denied (os error 13)"
        ));
    }

    #[test]
    fn empty_message_is_not_broken_pipe() {
        assert!(!is_broken_pipe_stderr_panic(""));
    }

    // The contract the Codex review demanded: the hook must NEVER hide a crash
    // from Sentry. `handle_panic` must call `chain` for EVERY panic — including
    // the broken-pipe family it used to swallow.
    fn run_handle_panic_and_record_chain(panic_msg: &'static str) -> bool {
        let chained = Arc::new(AtomicBool::new(false));
        let chained_for_hook = Arc::clone(&chained);
        // Trigger a panic to obtain a real PanicHookInfo, observe it in our hook.
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let flag = Arc::clone(&chained_for_hook);
            handle_panic(info, &move |_| flag.store(true, Ordering::SeqCst));
        }));
        let _ = std::panic::catch_unwind(|| panic!("{panic_msg}"));
        std::panic::set_hook(prev);
        chained.load(Ordering::SeqCst)
    }

    // Combined into one test: each case installs the GLOBAL panic hook, so they
    // must run serially (cargo parallelises separate #[test] fns).
    #[test]
    fn every_panic_class_chains_to_previous_hook() {
        // The exact broken-pipe case that used to be swallowed must now reach
        // Sentry (Codex review, PR #3772) — and so must an ordinary panic.
        assert!(
            run_handle_panic_and_record_chain(
                "failed printing to stderr: The pipe is being closed. (os error 232)"
            ),
            "broken-pipe stderr panic must still chain to the previous (Sentry) hook"
        );
        assert!(
            run_handle_panic_and_record_chain("assertion `left == right` failed"),
            "ordinary panic must chain to the previous (Sentry) hook"
        );
    }
}
