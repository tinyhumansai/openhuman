//! Surgical panic-hook guard for the "broken pipe on stderr" abort.
//!
//! ## The bug (Sentry TAURI-RUST-F, message family
//! `panic: failed printing to stderr: The pipe is being closed. (os error 232)`)
//!
//! On Windows the desktop binary calls `AttachConsole(ATTACH_PARENT_PROCESS)`
//! (see `main.rs`) for the `core` / `mcp` CLI subcommands so that `eprintln!`
//! output lands in the parent terminal. The GUI app keeps running long after
//! the user closes that parent terminal. Once the parent console pipe is gone,
//! the **next** stdlib write to stderr fails, and `std::io::stdio::print_to`
//! turns that I/O error into an unconditional
//! `panic!("failed printing to stderr: {e}")` (library/std/src/io/stdio.rs).
//! With no panic hook taming it, that panic aborts the whole process — a crash
//! the user never asked for, triggered purely by an *external* condition (the
//! parent console going away).
//!
//! ## Why a panic hook (and not a blanket swallow)
//!
//! The broken-pipe *trigger* is genuinely unpreventable: we cannot stop the
//! user's terminal from exiting, and any `eprintln!`/`log` write afterwards can
//! hit the dead pipe. But the *process abort* in response is preventable — a
//! failed diagnostic write should never take the app down.
//!
//! This hook is deliberately **surgical**: it recognises only the
//! broken-pipe-on-stderr panic and swallows it (downgraded to a debug log /
//! Sentry breadcrumb). **Every other panic** — assertions, real logic bugs,
//! the `assertion left == right` family that is *also* bucketed under
//! TAURI-RUST-F — is chained through to the previously-installed hook, which is
//! Sentry's panic integration (installed by `sentry::init`). Those keep
//! reaching Sentry exactly as before.
//!
//! `install` MUST be called *after* `sentry::init`, so `take_hook()` captures
//! Sentry's hook as the chain target.

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

/// Classify whether a panic message is the benign "stderr write failed because
/// the (parent console) pipe is closed" case that we want to swallow.
///
/// We anchor the match on the stdlib's stable prefix
/// `"failed printing to stderr"` (the literal in `std::io::stdio::print_to`)
/// AND a broken-pipe signal. The broken-pipe signal is matched
/// locale-independently via the embedded OS error code:
///   * `os error 232` — Windows `ERROR_NO_DATA` ("The pipe is being closed.")
///   * `os error 109` — Windows `ERROR_BROKEN_PIPE` ("The pipe has been ended.")
///   * `os error 32`  — POSIX `EPIPE` (defensive; the Windows console path is
///     the live one but a closed stderr can EPIPE on Unix too).
///
/// `std::io::Error`'s `Display` for an OS error always appends ` (os error N)`
/// regardless of system locale, so this matches the Chinese-locale variant
/// (`管道正在被关闭`) just as well as the English one — we never depend on the
/// localized strerror text.
///
/// Returns `false` for any other panic (assertions, real bugs, etc.) so they
/// chain through to Sentry untouched.
pub(crate) fn is_broken_pipe_stderr_panic(message: &str) -> bool {
    if !message.contains("failed printing to stderr") {
        return false;
    }
    message.contains("(os error 232)")   // Windows ERROR_NO_DATA — "pipe is being closed"
        || message.contains("(os error 109)") // Windows ERROR_BROKEN_PIPE
        || message.contains("(os error 32)") // POSIX EPIPE (defensive)
}

/// Install the surgical broken-pipe panic guard.
///
/// Chains to the previously-installed hook (Sentry's panic integration when
/// called after `sentry::init`) for every panic that is NOT the broken-pipe
/// stderr case, preserving full Sentry panic reporting for real defects.
pub(crate) fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info: &PanicHookInfo<'_>| {
        if let Some(message) = payload_message(info) {
            if is_broken_pipe_stderr_panic(message) {
                // Swallow: the parent console pipe is gone, a diagnostic write
                // failed, and aborting the GUI over it is the actual defect.
                // Downgrade to a breadcrumb + debug log; do NOT chain to the
                // Sentry hook (which would capture+report it as a crash).
                sentry::add_breadcrumb(sentry::Breadcrumb {
                    category: Some("panic".into()),
                    level: sentry::Level::Info,
                    message: Some(
                        "swallowed broken-pipe stderr panic (parent console closed)".into(),
                    ),
                    ..Default::default()
                });
                log::debug!("[stderr-panic-guard] swallowed broken-pipe stderr panic: {message}");
                return;
            }
        }
        // Real panic (assertion, logic bug, anything else): hand off to the
        // previous hook so Sentry's panic integration still fires.
        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::is_broken_pipe_stderr_panic;

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
        // The `assertion left == right` family also lands under TAURI-RUST-F;
        // it MUST chain through to Sentry, so it must NOT be classified here.
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
        // Only stderr writes are part of this bug; a stdout write panic should
        // still surface (different path / not our AttachConsole case).
        assert!(!is_broken_pipe_stderr_panic(
            "failed printing to stdout: The pipe is being closed. (os error 232)"
        ));
    }

    #[test]
    fn does_not_classify_stderr_panic_with_unrelated_error() {
        // A stderr-write failure for a *different* reason is not the
        // parent-console-closed case; let it through rather than over-swallow.
        assert!(!is_broken_pipe_stderr_panic(
            "failed printing to stderr: Permission denied (os error 13)"
        ));
    }

    #[test]
    fn empty_message_is_not_broken_pipe() {
        assert!(!is_broken_pipe_stderr_panic(""));
    }
}
