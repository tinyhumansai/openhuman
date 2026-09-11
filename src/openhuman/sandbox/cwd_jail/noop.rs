//! Fallback backend: no enforcement, just spawns.
//!
//! Used when no OS-level jail is available (unsupported platform, missing
//! kernel feature, etc.). Callers can still rely on application-layer
//! `validate_path_within_root` checks.

use std::process::{Child, Command};

use super::jail::{Jail, JailBackend};

/// The name `NoopBackend` reports, and the value callers compare against to
/// detect that no OS confinement is in force.
///
/// Single-sourced deliberately: `sandbox::ops::create_sandbox_backend` decides
/// whether the local sandbox handle reports `Ready` or `Inactive` by comparing
/// against this, so a rename here must not silently turn an unconfined host
/// back into a `Ready` report.
pub const NOOP_BACKEND_NAME: &str = "noop";

#[derive(Debug, Default)]
pub struct NoopBackend;

impl JailBackend for NoopBackend {
    fn name(&self) -> &'static str {
        NOOP_BACKEND_NAME
    }

    fn is_available(&self) -> bool {
        true
    }

    fn spawn(&self, _jail: &Jail, mut cmd: Command) -> std::io::Result<Child> {
        cmd.spawn()
    }
}

#[cfg(test)]
#[path = "noop_tests.rs"]
mod tests;
