//! Sandbox trait for pluggable OS-level isolation

use std::process::Command;

/// Sandbox backend for OS-level isolation
pub trait Sandbox: Send + Sync {
    /// Wrap a command with sandbox protection
    fn wrap_command(&self, cmd: &mut Command) -> std::io::Result<()>;

    /// Check if this sandbox backend is available on the current platform
    fn is_available(&self) -> bool;

    /// Human-readable name of this sandbox backend
    fn name(&self) -> &str;

    /// Description of what this sandbox provides
    fn description(&self) -> &str;
}

/// No-op sandbox (always available, provides no additional isolation)
#[derive(Debug, Clone, Default)]
pub struct NoopSandbox;

impl Sandbox for NoopSandbox {
    fn wrap_command(&self, _cmd: &mut Command) -> std::io::Result<()> {
        // Pass through unchanged
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }

    fn name(&self) -> &str {
        "none"
    }

    fn description(&self) -> &str {
        "No sandboxing (application-layer security only)"
    }
}

#[cfg(test)]
#[path = "traits_tests.rs"]
mod tests;
