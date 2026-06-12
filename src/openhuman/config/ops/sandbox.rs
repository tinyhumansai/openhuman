//! Sandbox / Docker runtime config operations.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use super::loader::{load_config_with_timeout, snapshot_config_json};

/// Partial update for the `[security.sandbox]` + `[runtime.docker]` blocks.
#[derive(Debug, Clone, Default)]
pub struct SandboxSettingsPatch {
    pub backend: Option<String>,
    pub enabled: Option<bool>,
    pub docker_image: Option<String>,
    pub docker_memory_limit_mb: Option<u64>,
    pub docker_cpu_limit: Option<f64>,
    pub env_passthrough: Option<Vec<String>>,
}

pub async fn get_sandbox_settings() -> Result<RpcOutcome<serde_json::Value>, String> {
    let config = load_config_with_timeout().await?;
    let sandbox = &config.sandbox;
    let docker = &config.runtime.docker;

    let docker_available = is_docker_available().await;

    let backend_str = match sandbox.backend {
        crate::openhuman::config::SandboxBackend::Auto => "auto",
        crate::openhuman::config::SandboxBackend::Landlock => "landlock",
        crate::openhuman::config::SandboxBackend::Firejail => "firejail",
        crate::openhuman::config::SandboxBackend::Bubblewrap => "bubblewrap",
        crate::openhuman::config::SandboxBackend::Docker => "docker",
        crate::openhuman::config::SandboxBackend::None => "none",
    };

    let detected_backend = detect_os_sandbox_backend();

    let value = serde_json::json!({
        "enabled": sandbox.enabled.unwrap_or(true),
        "backend": backend_str,
        "docker_image": docker.image,
        "docker_memory_limit_mb": docker.memory_limit_mb,
        "docker_cpu_limit": docker.cpu_limit,
        "docker_available": docker_available,
        "detected_backend": detected_backend,
        "env_passthrough": crate::openhuman::sandbox::ops::SANDBOX_ENV_PASSTHROUGH,
    });
    log::debug!("[config][sandbox] get_sandbox_settings: backend={backend_str}, docker_available={docker_available}");
    Ok(RpcOutcome::single_log(value, "sandbox settings read"))
}

pub async fn apply_sandbox_settings(
    config: &mut Config,
    update: SandboxSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    if let Some(ref backend) = update.backend {
        config.sandbox.backend = match backend.as_str() {
            "auto" => crate::openhuman::config::SandboxBackend::Auto,
            "landlock" => crate::openhuman::config::SandboxBackend::Landlock,
            "firejail" => crate::openhuman::config::SandboxBackend::Firejail,
            "bubblewrap" => crate::openhuman::config::SandboxBackend::Bubblewrap,
            "docker" => crate::openhuman::config::SandboxBackend::Docker,
            "none" => crate::openhuman::config::SandboxBackend::None,
            other => {
                log::warn!("[config][sandbox] rejected unknown backend: {other}");
                return Err(format!(
                    "unknown sandbox backend '{other}'; valid: auto, landlock, firejail, bubblewrap, docker, none"
                ));
            }
        };
    }
    if let Some(enabled) = update.enabled {
        config.sandbox.enabled = Some(enabled);
    }
    if let Some(ref image) = update.docker_image {
        let trimmed = image.trim();
        if trimmed.is_empty() {
            return Err("docker_image must not be blank".into());
        }
        config.runtime.docker.image = trimmed.to_string();
    }
    if let Some(memory) = update.docker_memory_limit_mb {
        config.runtime.docker.memory_limit_mb = Some(memory);
    }
    if let Some(cpu) = update.docker_cpu_limit {
        if cpu <= 0.0 {
            return Err("docker_cpu_limit must be positive".into());
        }
        config.runtime.docker.cpu_limit = Some(cpu);
    }
    if let Some(ref passthrough) = update.env_passthrough {
        log::debug!(
            "[config][sandbox] env_passthrough update: {} vars",
            passthrough.len()
        );
    }

    config.save().await.map_err(|e| e.to_string())?;

    log::debug!(
        "[config][sandbox] sandbox settings saved to {}",
        config.config_path.display()
    );
    let snapshot = snapshot_config_json(config)?;
    Ok(RpcOutcome::new(
        snapshot,
        vec![format!(
            "sandbox settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn load_and_apply_sandbox_settings(
    update: SandboxSettingsPatch,
) -> Result<RpcOutcome<serde_json::Value>, String> {
    let mut config = load_config_with_timeout().await?;
    apply_sandbox_settings(&mut config, update).await
}

async fn is_docker_available() -> bool {
    let fut = tokio::process::Command::new("docker")
        .arg("info")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status();
    match tokio::time::timeout(std::time::Duration::from_secs(5), fut).await {
        Ok(Ok(status)) => status.success(),
        _ => false,
    }
}

fn detect_os_sandbox_backend() -> &'static str {
    #[cfg(target_os = "linux")]
    {
        if std::path::Path::new("/sys/kernel/security/landlock").exists() {
            return "landlock";
        }
        if std::process::Command::new("firejail")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return "firejail";
        }
        if std::process::Command::new("bwrap")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok()
        {
            return "bubblewrap";
        }
        "none"
    }
    #[cfg(target_os = "macos")]
    {
        "seatbelt"
    }
    #[cfg(target_os = "windows")]
    {
        "appcontainer"
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        "none"
    }
}
