//! Firejail sandbox (Linux user-space sandboxing)
//!
//! Firejail is a SUID sandbox program that Linux applications use to sandbox themselves.

use crate::openhuman::security::traits::Sandbox;
use std::process::Command;

/// Firejail sandbox backend for Linux
#[derive(Debug, Clone, Default)]
pub struct FirejailSandbox;

impl FirejailSandbox {
    /// Create a new Firejail sandbox
    pub fn new() -> std::io::Result<Self> {
        if Self::is_installed() {
            Ok(Self)
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Firejail not found. Install firejail using your system package manager.",
            ))
        }
    }

    /// Probe if Firejail is available (for auto-detection)
    pub fn probe() -> std::io::Result<Self> {
        Self::new()
    }

    /// Check if firejail is installed
    fn is_installed() -> bool {
        Command::new("firejail")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

impl Sandbox for FirejailSandbox {
    fn wrap_command(&self, cmd: &mut Command) -> std::io::Result<()> {
        // Prepend firejail to the command
        let program = cmd.get_program().to_string_lossy().to_string();
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        // Build firejail wrapper with security flags
        let mut firejail_cmd = Command::new("firejail");
        firejail_cmd.args([
            "--private=home", // New home directory
            "--private-dev",  // Minimal /dev
            "--nosound",      // No audio
            "--no3d",         // No 3D acceleration
            "--novideo",      // No video devices
            "--nowheel",      // No input devices
            "--notv",         // No TV devices
            "--noprofile",    // Skip profile loading
            "--quiet",        // Suppress warnings
        ]);

        // Add the original command
        firejail_cmd.arg(&program);
        firejail_cmd.args(&args);

        // Replace the command
        *cmd = firejail_cmd;
        Ok(())
    }

    fn is_available(&self) -> bool {
        Self::is_installed()
    }

    fn name(&self) -> &str {
        "firejail"
    }

    fn description(&self) -> &str {
        "Linux user-space sandbox (requires firejail to be installed)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn firejail_sandbox_name() {
        assert_eq!(FirejailSandbox.name(), "firejail");
    }

    #[test]
    fn firejail_description_mentions_dependency() {
        let desc = FirejailSandbox.description();
        assert!(desc.contains("firejail"));
    }

    #[test]
    fn firejail_new_fails_if_not_installed() {
        // This will fail unless firejail is actually installed
        let result = FirejailSandbox::new();
        match result {
            Ok(_) => println!("Firejail is installed"),
            Err(e) => assert!(
                e.kind() == std::io::ErrorKind::NotFound
                    || e.kind() == std::io::ErrorKind::Unsupported
            ),
        }
    }

    #[test]
    fn firejail_wrap_command_prepends_firejail() {
        let sandbox = FirejailSandbox;
        let mut cmd = Command::new("echo");
        cmd.arg("test");

        // Note: wrap_command will fail if firejail isn't installed,
        // but we can still test the logic structure
        let _ = sandbox.wrap_command(&mut cmd);

        // After wrapping, the program should be firejail
        if sandbox.is_available() {
            assert_eq!(cmd.get_program().to_string_lossy(), "firejail");
        }
    }

    // ── §1.1 Sandbox isolation flag tests ──────────────────────

    #[test]
    fn firejail_wrap_command_includes_all_security_flags() {
        let sandbox = FirejailSandbox;
        let mut cmd = Command::new("echo");
        cmd.arg("test");
        sandbox.wrap_command(&mut cmd).unwrap();

        assert_eq!(
            cmd.get_program().to_string_lossy(),
            "firejail",
            "wrapped command should use firejail as program"
        );

        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        let expected_flags = [
            "--private=home",
            "--private-dev",
            "--nosound",
            "--no3d",
            "--novideo",
            "--nowheel",
            "--notv",
            "--noprofile",
            "--quiet",
        ];

        for flag in &expected_flags {
            assert!(
                args.contains(&flag.to_string()),
                "must include security flag: {flag}"
            );
        }
    }

    #[test]
    fn firejail_wrap_command_preserves_original_command() {
        let sandbox = FirejailSandbox;
        let mut cmd = Command::new("ls");
        cmd.arg("-la");
        cmd.arg("/workspace");
        sandbox.wrap_command(&mut cmd).unwrap();

        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        assert!(
            args.contains(&"ls".to_string()),
            "original program must be passed as argument"
        );
        assert!(
            args.contains(&"-la".to_string()),
            "original args must be preserved"
        );
        assert!(
            args.contains(&"/workspace".to_string()),
            "original args must be preserved"
        );
    }
}
