use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory_store::content::obsidian_registry;
use crate::rpc::RpcOutcome;

use super::types::{ObsidianVaultStatusResponse, VaultHealthCheckResponse};

pub async fn obsidian_vault_status_rpc(
    config: &Config,
    obsidian_config_dir: Option<String>,
) -> Result<RpcOutcome<ObsidianVaultStatusResponse>, String> {
    let cfg = config.clone();
    let resp = tokio::task::spawn_blocking(move || -> ObsidianVaultStatusResponse {
        let content_root = cfg.memory_tree_content_root();
        let extra = obsidian_config_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(std::path::Path::new);
        let reg = obsidian_registry::vault_registration_status(&content_root, extra);
        ObsidianVaultStatusResponse {
            registered: reg.registered,
            config_found: reg.config_found,
            content_root_abs: content_root.to_string_lossy().to_string(),
            host_os: std::env::consts::OS.to_string(),
        }
    })
    .await
    .map_err(|e| format!("obsidian_vault_status join error: {e}"))?;

    let log = format!(
        "memory_tree::read: obsidian_vault_status registered={} config_found={} root_hash={}",
        resp.registered,
        resp.config_found,
        crate::openhuman::memory::util::redact::redact(&resp.content_root_abs),
    );
    Ok(RpcOutcome::single_log(resp, log))
}

pub async fn vault_health_check_rpc(
    config: &Config,
    obsidian_config_dir: Option<String>,
) -> Result<RpcOutcome<VaultHealthCheckResponse>, String> {
    let cfg = config.clone();
    let fs_probe = tokio::task::spawn_blocking(move || {
        let content_root = cfg.memory_tree_content_root();
        let content_root_abs = content_root.to_string_lossy().to_string();
        let exists = content_root.is_dir();
        let readable = exists && std::fs::read_dir(&content_root).is_ok();
        let writable = exists && probe_directory_writable(&content_root);

        let extra = obsidian_config_dir
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(std::path::Path::new);
        let obsidian_registered =
            obsidian_registry::vault_registration_status(&content_root, extra).registered;

        (
            content_root_abs,
            exists,
            readable,
            writable,
            obsidian_registered,
        )
    })
    .await
    .map_err(|e| format!("vault_health_check fs probe join error: {e}"))?;

    let pipeline = crate::openhuman::memory_tree::tree::rpc::pipeline_status_rpc(config)
        .await
        .map_err(|e| format!("vault_health_check pipeline_status: {e}"))?;

    let (content_root_abs, exists, readable, writable, obsidian_registered) = fs_probe;
    let pipeline_healthy = pipeline.value.status != "error" && !pipeline.value.is_paused;
    let last_sync_ms = pipeline.value.last_sync_ms.max(0);

    let resp = VaultHealthCheckResponse {
        content_root_abs,
        exists,
        readable,
        writable,
        obsidian_registered,
        pipeline_healthy,
        last_sync_ms,
        host_os: std::env::consts::OS.to_string(),
    };

    let log = format!(
        "memory_tree::read: vault_health_check exists={} readable={} writable={} obsidian_registered={} pipeline_healthy={} last_sync_ms={} root_hash={}",
        resp.exists,
        resp.readable,
        resp.writable,
        resp.obsidian_registered,
        resp.pipeline_healthy,
        resp.last_sync_ms,
        crate::openhuman::memory::util::redact::redact(&resp.content_root_abs),
    );
    Ok(RpcOutcome::single_log(resp, log))
}

fn probe_directory_writable(dir: &std::path::Path) -> bool {
    use std::io::Write;
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let probe = dir.join(format!(
        ".openhuman-vault-writecheck-{}-{ts}.tmp",
        std::process::id()
    ));
    match std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&probe)
    {
        Ok(mut file) => {
            let write_ok = file.write_all(b"ok").is_ok();
            if let Err(e) = std::fs::remove_file(&probe) {
                log::debug!("[memory] vault write-probe cleanup failed: {e}");
            }
            write_ok
        }
        Err(_) => false,
    }
}
