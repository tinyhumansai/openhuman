//! Docker sandbox (container isolation)

use crate::openhuman::security::traits::Sandbox;
use std::process::Command;

/// Docker sandbox backend
#[derive(Debug, Clone)]
pub struct DockerSandbox {
    image: String,
}

impl Default for DockerSandbox {
    fn default() -> Self {
        Self {
            image: "alpine:latest".to_string(),
        }
    }
}

impl DockerSandbox {
    pub fn new() -> std::io::Result<Self> {
        if Self::is_installed() {
            Ok(Self::default())
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Docker not found",
            ))
        }
    }

    pub fn with_image(image: String) -> std::io::Result<Self> {
        if Self::is_installed() {
            Ok(Self { image })
        } else {
            Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                "Docker not found",
            ))
        }
    }

    pub fn probe() -> std::io::Result<Self> {
        Self::new()
    }

    fn is_installed() -> bool {
        Command::new("docker")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }
}

impl Sandbox for DockerSandbox {
    fn wrap_command(&self, cmd: &mut Command) -> std::io::Result<()> {
        let program = cmd.get_program().to_string_lossy().to_string();
        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        let mut docker_cmd = Command::new("docker");
        docker_cmd.args([
            "run",
            "--rm",
            "--memory",
            "512m",
            "--cpus",
            "1.0",
            "--network",
            "none",
        ]);
        docker_cmd.arg(&self.image);
        docker_cmd.arg(&program);
        docker_cmd.args(&args);

        *cmd = docker_cmd;
        Ok(())
    }

    fn is_available(&self) -> bool {
        Self::is_installed()
    }

    fn name(&self) -> &str {
        "docker"
    }

    fn description(&self) -> &str {
        "Docker container isolation (requires docker)"
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn docker_sandbox_name() {
        let sandbox = DockerSandbox::default();
        assert_eq!(sandbox.name(), "docker");
    }

    #[test]
    fn docker_sandbox_default_image() {
        let sandbox = DockerSandbox::default();
        assert_eq!(sandbox.image, "alpine:latest");
    }

    #[test]
    fn docker_with_custom_image() {
        let result = DockerSandbox::with_image("ubuntu:latest".to_string());
        match result {
            Ok(sandbox) => assert_eq!(sandbox.image, "ubuntu:latest"),
            Err(_) => assert!(!DockerSandbox::is_installed()),
        }
    }

    // ── §1.1 Sandbox isolation flag tests ──────────────────────

    #[test]
    fn docker_wrap_command_includes_isolation_flags() {
        let sandbox = DockerSandbox::default();
        let mut cmd = Command::new("echo");
        cmd.arg("hello");
        sandbox.wrap_command(&mut cmd).unwrap();

        assert_eq!(
            cmd.get_program().to_string_lossy(),
            "docker",
            "wrapped command should use docker as program"
        );

        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        assert!(
            args.contains(&"run".to_string()),
            "must include 'run' subcommand"
        );
        assert!(
            args.contains(&"--rm".to_string()),
            "must include --rm for auto-cleanup"
        );
        assert!(
            args.contains(&"--network".to_string()),
            "must include --network flag"
        );
        assert!(
            args.contains(&"none".to_string()),
            "network must be set to 'none' for isolation"
        );
        assert!(
            args.contains(&"--memory".to_string()),
            "must include --memory limit"
        );
        assert!(
            args.contains(&"512m".to_string()),
            "memory limit must be 512m"
        );
        assert!(
            args.contains(&"--cpus".to_string()),
            "must include --cpus limit"
        );
        assert!(args.contains(&"1.0".to_string()), "CPU limit must be 1.0");
    }

    #[test]
    fn docker_wrap_command_preserves_original_command() {
        let sandbox = DockerSandbox::default();
        let mut cmd = Command::new("ls");
        cmd.arg("-la");
        sandbox.wrap_command(&mut cmd).unwrap();

        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        assert!(
            args.contains(&"alpine:latest".to_string()),
            "must include the container image"
        );
        assert!(
            args.contains(&"ls".to_string()),
            "original program must be passed as argument"
        );
        assert!(
            args.contains(&"-la".to_string()),
            "original args must be preserved"
        );
    }

    #[test]
    fn docker_wrap_command_uses_custom_image() {
        let sandbox = DockerSandbox {
            image: "ubuntu:22.04".to_string(),
        };
        let mut cmd = Command::new("echo");
        sandbox.wrap_command(&mut cmd).unwrap();

        let args: Vec<String> = cmd
            .get_args()
            .map(|s| s.to_string_lossy().to_string())
            .collect();

        assert!(
            args.contains(&"ubuntu:22.04".to_string()),
            "must use the custom image"
        );
    }
}
