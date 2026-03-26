//! Landlock sandbox (Linux kernel 5.13+ LSM)
//!
//! Landlock provides unprivileged sandboxing through the Linux kernel.
//! This module uses the pure-Rust `landlock` crate for filesystem access control.

#[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
use landlock::{AccessFs, PathBeneath, PathFd, Ruleset, RulesetAttr, RulesetCreatedAttr};

use crate::openhuman::security::traits::Sandbox;
use std::path::Path;

/// Landlock sandbox backend for Linux
#[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
#[derive(Debug)]
pub struct LandlockSandbox {
    workspace_dir: Option<std::path::PathBuf>,
}

#[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
impl LandlockSandbox {
    /// Create a new Landlock sandbox with the given workspace directory
    pub fn new() -> std::io::Result<Self> {
        Self::with_workspace(None)
    }

    /// Create a Landlock sandbox with a specific workspace directory
    pub fn with_workspace(workspace_dir: Option<std::path::PathBuf>) -> std::io::Result<Self> {
        // Test if Landlock is available by trying to create a minimal ruleset
        let test_ruleset = Ruleset::default()
            .handle_access(AccessFs::ReadFile | AccessFs::WriteFile)
            .and_then(|ruleset| ruleset.create());

        match test_ruleset {
            Ok(_) => Ok(Self { workspace_dir }),
            Err(e) => {
                log::debug!("Landlock not available: {}", e);
                Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "Landlock not available",
                ))
            }
        }
    }

    /// Probe if Landlock is available (for auto-detection)
    pub fn probe() -> std::io::Result<Self> {
        Self::new()
    }

    /// Apply Landlock restrictions to the current process
    fn apply_restrictions(&self) -> std::io::Result<()> {
        let mut ruleset = Ruleset::default()
            .handle_access(
                AccessFs::ReadFile
                    | AccessFs::WriteFile
                    | AccessFs::ReadDir
                    | AccessFs::RemoveDir
                    | AccessFs::RemoveFile
                    | AccessFs::MakeChar
                    | AccessFs::MakeSock
                    | AccessFs::MakeFifo
                    | AccessFs::MakeBlock
                    | AccessFs::MakeReg
                    | AccessFs::MakeSym,
            )
            .and_then(|ruleset| ruleset.create())
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Allow workspace directory (read/write)
        if let Some(ref workspace) = self.workspace_dir {
            if workspace.exists() {
                let workspace_fd =
                    PathFd::new(workspace).map_err(|e| std::io::Error::other(e.to_string()))?;
                ruleset = ruleset
                    .add_rule(PathBeneath::new(
                        workspace_fd,
                        AccessFs::ReadFile | AccessFs::WriteFile | AccessFs::ReadDir,
                    ))
                    .map_err(|e| std::io::Error::other(e.to_string()))?;
            }
        }

        // Allow /tmp for general operations
        let tmp_fd =
            PathFd::new(Path::new("/tmp")).map_err(|e| std::io::Error::other(e.to_string()))?;
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                tmp_fd,
                AccessFs::ReadFile | AccessFs::WriteFile,
            ))
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Allow /usr and /bin for executing commands
        let usr_fd =
            PathFd::new(Path::new("/usr")).map_err(|e| std::io::Error::other(e.to_string()))?;
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                usr_fd,
                AccessFs::ReadFile | AccessFs::ReadDir,
            ))
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        let bin_fd =
            PathFd::new(Path::new("/bin")).map_err(|e| std::io::Error::other(e.to_string()))?;
        ruleset = ruleset
            .add_rule(PathBeneath::new(
                bin_fd,
                AccessFs::ReadFile | AccessFs::ReadDir,
            ))
            .map_err(|e| std::io::Error::other(e.to_string()))?;

        // Apply the ruleset
        match ruleset.restrict_self() {
            Ok(_) => {
                log::debug!("Landlock restrictions applied successfully");
                Ok(())
            }
            Err(e) => {
                log::warn!("Failed to apply Landlock restrictions: {}", e);
                Err(std::io::Error::other(e.to_string()))
            }
        }
    }
}

#[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
impl Sandbox for LandlockSandbox {
    fn wrap_command(&self, _cmd: &mut std::process::Command) -> std::io::Result<()> {
        // Apply Landlock restrictions before executing the command
        // Note: This affects the current process, not the child process
        // Child processes inherit the Landlock restrictions
        self.apply_restrictions()
    }

    fn is_available(&self) -> bool {
        // Try to create a minimal ruleset to verify availability
        Ruleset::default()
            .handle_access(AccessFs::ReadFile)
            .and_then(|ruleset| ruleset.create())
            .is_ok()
    }

    fn name(&self) -> &str {
        "landlock"
    }

    fn description(&self) -> &str {
        "Linux kernel LSM sandboxing (filesystem access control)"
    }
}

// Stub implementations for non-Linux or when feature is disabled
#[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
pub struct LandlockSandbox;

#[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
impl LandlockSandbox {
    pub fn new() -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Landlock is only supported on Linux with the sandbox-landlock feature",
        ))
    }

    pub fn with_workspace(_workspace_dir: Option<std::path::PathBuf>) -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Landlock is only supported on Linux",
        ))
    }

    pub fn probe() -> std::io::Result<Self> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Landlock is only supported on Linux",
        ))
    }
}

#[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
impl Sandbox for LandlockSandbox {
    fn wrap_command(&self, _cmd: &mut std::process::Command) -> std::io::Result<()> {
        Err(std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Landlock is only supported on Linux",
        ))
    }

    fn is_available(&self) -> bool {
        false
    }

    fn name(&self) -> &str {
        "landlock"
    }

    fn description(&self) -> &str {
        "Linux kernel LSM sandboxing (not available on this platform)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(feature = "sandbox-landlock", target_os = "linux"))]
    #[test]
    fn landlock_sandbox_name() {
        if let Ok(sandbox) = LandlockSandbox::new() {
            assert_eq!(sandbox.name(), "landlock");
        }
    }

    #[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
    #[test]
    fn landlock_not_available_on_non_linux() {
        assert!(!LandlockSandbox.is_available());
        assert_eq!(LandlockSandbox.name(), "landlock");
    }

    #[test]
    fn landlock_with_none_workspace() {
        // Should work even without a workspace directory
        let result = LandlockSandbox::with_workspace(None);
        // Result depends on platform and feature flag
        match result {
            Ok(sandbox) => assert!(sandbox.is_available()),
            Err(_) => assert!(!cfg!(all(
                feature = "sandbox-landlock",
                target_os = "linux"
            ))),
        }
    }

    // ── §1.1 Landlock stub tests ──────────────────────────────

    #[cfg(not(all(feature = "sandbox-landlock", target_os = "linux")))]
    #[test]
    fn landlock_stub_wrap_command_returns_unsupported() {
        let sandbox = LandlockSandbox;
        let mut cmd = std::process::Command::new("echo");
        let result = sandbox.wrap_command(&mut cmd);
        assert!(result.is_err());
        assert_eq!(result.unwrap_err().kind(), std::io::ErrorKind::Unsupported);
    }
}
