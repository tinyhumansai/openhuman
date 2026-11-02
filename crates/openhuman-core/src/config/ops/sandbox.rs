//! Sandbox / Docker runtime config operations.

use crate::config::Config;
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;

use super::loader::snapshot_config_json;

/// Partial update for the `[sandbox]` + `[runtime.docker]` blocks.
#[derive(Debug, Clone, Default)]
pub struct SandboxSettingsPatch {
    pub backend: Option<String>,
    pub enabled: Option<bool>,
    pub docker_image: Option<String>,
    pub docker_memory_limit_mb: Option<u64>,
    pub docker_cpu_limit: Option<f64>,
    pub env_passthrough: Option<Vec<String>>,
}

pub async fn get_sandbox_settings(ctx: &DispatchCtx) -> Result<Outcome<serde_json::Value>, String> {
    let config = ctx.load_config().await?;
    let sandbox = &config.sandbox;
    let docker = &config.runtime.docker;

    // The daemon probe has no timeout of its own; bound it so a wedged
    // daemon cannot hang the settings read.
    let docker_available = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        crate::sandbox::docker::is_docker_available(&config),
    )
    .await
    .map_err(|_| "Docker status probe timed out".to_owned())??;

    let backend_str = backend_name(&sandbox.backend);

    let detected_backend = crate::sandbox::native_jail::status(&config)
        .await
        .map(|(backend, _, _)| backend)
        .unwrap_or_else(|_| "unavailable".to_owned());

    let value = serde_json::json!({
        "enabled": sandbox.enabled.unwrap_or(true)
            && crate::core::runtime::ComponentSet::from_config(&config.components).sandbox,
        "backend": backend_str,
        "docker_image": docker.image,
        "docker_memory_limit_mb": docker.memory_limit_mb,
        "docker_cpu_limit": docker.cpu_limit,
        "docker_available": docker_available,
        "detected_backend": detected_backend,
        "env_passthrough": crate::sandbox::ops::SANDBOX_ENV_PASSTHROUGH,
    });
    log::debug!(
        "[config][sandbox] get_sandbox_settings: backend={backend_str}, docker_available={docker_available}"
    );
    Ok(Outcome::single_log(value, "sandbox settings read"))
}

/// The settings name of a backend, as `[security.sandbox] backend` spells it.
fn backend_name(backend: &crate::config::SandboxBackend) -> &'static str {
    use crate::config::SandboxBackend;
    match backend {
        SandboxBackend::Auto => "auto",
        SandboxBackend::Landlock => "landlock",
        SandboxBackend::Firejail => "firejail",
        SandboxBackend::Bubblewrap => "bubblewrap",
        SandboxBackend::Docker => "docker",
        SandboxBackend::OpenShell => "openshell",
        SandboxBackend::None => "none",
    }
}

/// The backend a settings name selects. `firejail` and `bubblewrap` stay
/// readable in saved settings but cannot be chosen: TinyBox does the native
/// sandboxing in this build.
fn parse_backend(name: &str) -> Result<crate::config::SandboxBackend, String> {
    use crate::config::SandboxBackend;
    Ok(match name {
        "auto" => SandboxBackend::Auto,
        "landlock" => SandboxBackend::Landlock,
        "firejail" | "bubblewrap" => {
            return Err(format!(
                "unsupported sandbox backend '{name}': this build uses TinyBox for native sandboxing; valid: auto, landlock, docker, openshell, none"
            ));
        }
        "docker" => SandboxBackend::Docker,
        "openshell" => SandboxBackend::OpenShell,
        "none" => SandboxBackend::None,
        other => {
            log::warn!("[config][sandbox] rejected unknown backend: {other}");
            return Err(format!(
                "unknown sandbox backend '{other}'; valid: auto, landlock, docker, openshell, none"
            ));
        }
    })
}

pub async fn apply_sandbox_settings(
    config: &mut Config,
    update: SandboxSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    if update.env_passthrough.is_some() {
        return Err(
            "env_passthrough is not configurable; sandbox environment variables use the built-in safe allowlist"
                .into(),
        );
    }
    if let Some(ref backend) = update.backend {
        config.sandbox.backend = parse_backend(backend)?;
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
    config.save().await.map_err(super::config_save_error)?;

    log::debug!(
        "[config][sandbox] sandbox settings saved to {}",
        config.config_path.display()
    );
    let snapshot = snapshot_config_json(config).await?;
    Ok(Outcome::new(
        snapshot,
        vec![format!(
            "sandbox settings saved to {}",
            config.config_path.display()
        )],
    ))
}

pub async fn load_and_apply_sandbox_settings(
    ctx: &DispatchCtx,
    update: SandboxSettingsPatch,
) -> Result<Outcome<serde_json::Value>, String> {
    let mut config = ctx.load_config().await?;
    apply_sandbox_settings(&mut config, update).await
}

#[cfg(test)]
#[path = "sandbox_tests.rs"]
mod tests;
