//! Fallback backend: no enforcement, just spawns.
//!
//! Used when no OS-level jail is available (unsupported platform, missing
//! kernel feature, etc.). Callers can still rely on application-layer
//! `validate_path_within_root` checks.

use std::process::{Child, Command};

use super::jail::{Jail, JailBackend};

#[derive(Debug, Default)]
pub struct NoopBackend;

impl JailBackend for NoopBackend {
    fn name(&self) -> &'static str {
        "noop"
    }

    fn is_available(&self) -> bool {
        true
    }

    fn spawn(&self, _jail: &Jail, mut cmd: Command) -> std::io::Result<Child> {
        cmd.spawn()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Regression guard for #3235.
    ///
    /// On platforms where no OS-level jail backend is available
    /// (Landlock / Seatbelt / AppContainer absent or unsupported),
    /// `cwd_jail::detect::pick_backend()` returns `NoopBackend`. This
    /// test pins the NoopBackend contract — it must always report
    /// `is_available() == true` so it's a usable fallback, must
    /// identify as `"noop"` so operators can see in logs which backend
    /// is active, and `spawn` must be a passthrough that runs the
    /// command without enforcement. The in-Rust path hardening in
    /// `SecurityPolicy::is_path_string_allowed` /
    /// `is_workspace_internal_path` still applies; the noop name
    /// reflects "no OS-level isolation," not "no defense at all."
    #[test]
    fn noop_backend_identifies_as_noop_and_is_always_available() {
        let backend = NoopBackend;
        assert_eq!(backend.name(), "noop");
        assert!(
            backend.is_available(),
            "NoopBackend must always be available so it can serve as the \
             documented passthrough on unsupported platforms (see CLAUDE.md \
             'Sandbox execution backends')."
        );
    }

    #[test]
    fn noop_backend_spawn_runs_command_passthrough() {
        // On a POSIX host, `true` exits 0 immediately. The NoopBackend
        // must spawn it successfully without injecting any jail flags,
        // proving the passthrough contract.
        let backend = NoopBackend;
        let jail = Jail::new("/tmp", "noop-test-3235");
        let cmd = if cfg!(windows) {
            let mut c = Command::new("cmd");
            c.args(["/C", "exit 0"]);
            c
        } else {
            Command::new("true")
        };
        let child = backend
            .spawn(&jail, cmd)
            .expect("NoopBackend::spawn must passthrough-spawn the child");
        let output = child
            .wait_with_output()
            .expect("child must run to completion");
        assert!(
            output.status.success(),
            "passthrough child should succeed; got {:?}",
            output.status
        );
    }
}
