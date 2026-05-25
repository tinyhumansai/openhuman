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
