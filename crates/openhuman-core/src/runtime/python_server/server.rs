//! Host side of the runtime Python server.
//!
//! The worker process itself (spawn, handshake, request/response, restart,
//! idle expiry, startup back-off) lives in `tinyruntime-pyserver`. This file
//! only maps `Config` and the managed interpreter onto a
//! [`ServerLaunch`], and holds the process-wide slot.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Result};
use serde_json::json;
use tinyruntime_pyserver::{IdleRule, PythonServer, ServerLaunch, ServerSlot, ServerStatus};

use super::registry::{enabled_backends, RuntimePythonBackend};
use crate::config::Config;

/// A running worker process, as the rest of the core names it.
pub type RuntimePythonServer = PythonServer;

static SLOT: ServerSlot = ServerSlot::new();

pub async fn ensure_started(config: &Config) -> Result<Arc<RuntimePythonServer>> {
    let requested: Vec<String> = enabled_backends(config)
        .iter()
        .map(|backend| backend.id().to_string())
        .collect();
    let idle = IdleRule {
        backend: RuntimePythonBackend::Kompress.id(),
        timeout: Duration::from_secs(config.tokenjuice.ml_sidecar_idle_timeout_secs),
    };
    let server = SLOT
        .ensure(&requested, Some(idle), async {
            prepare_launch(config)
                .await
                .map_err(|error| tinyruntime_pyserver::Error::Prepare(format!("{error:#}")))
        })
        .await?;
    Ok(server)
}

pub async fn status() -> ServerStatus {
    SLOT.status().await
}

async fn prepare_launch(config: &Config) -> Result<ServerLaunch> {
    let backends = enabled_backends(config);
    if backends.is_empty() {
        bail!("no runtime python server backends enabled");
    }

    // The server runs ONE interpreter. Kompress is the only backend, and it
    // owns its venv.
    let mut env: Vec<(String, String)> = Vec::new();
    let python_bin = if backends.contains(&RuntimePythonBackend::Kompress) {
        let rt = super::kompress::ensure_kompress(config).await?;
        push_kompress_env(&mut env, config, &rt.hf_home);
        rt.python_bin
    } else {
        crate::runtime::python::PythonBootstrap::new(std::sync::Arc::new(config.clone()))
            .resolve()
            .await?
            .python_bin
    };

    env.push((
        "OPENHUMAN_RPS_BACKENDS".to_string(),
        backends
            .iter()
            .map(|b| b.id())
            .collect::<Vec<_>>()
            .join(","),
    ));

    let script_path = write_server_script(config).await?;

    Ok(ServerLaunch::new(
        python_bin,
        script_path,
        backends.iter().map(|b| b.id().to_string()).collect(),
        env,
    ))
}

/// Environment the worker needs to load + run the Kompress model offline.
fn push_kompress_env(env: &mut Vec<(String, String)>, config: &Config, hf_home: &std::path::Path) {
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_MODEL".to_string(),
        config.tokenjuice.ml_model_id.clone(),
    ));
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_DEVICE".to_string(),
        config.tokenjuice.ml_device.clone(),
    ));
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_TARGET_RATIO".to_string(),
        config.tokenjuice.ml_target_ratio.to_string(),
    ));
    env.push((
        "OPENHUMAN_RPS_KOMPRESS_MAX_INPUT_CHARS".to_string(),
        config.tokenjuice.ml_max_input_chars.to_string(),
    ));
    env.push(("HF_HOME".to_string(), hf_home.display().to_string()));
    // Model was pre-downloaded during provisioning; load offline so startup
    // never blocks on the network.
    env.push(("HF_HUB_OFFLINE".to_string(), "1".to_string()));
    env.push(("TRANSFORMERS_OFFLINE".to_string(), "1".to_string()));
    env.push(("HF_HUB_DISABLE_TELEMETRY".to_string(), "1".to_string()));
}

async fn write_server_script(config: &Config) -> Result<PathBuf> {
    let root = python_server_cache_root(config);
    Ok(tinyruntime_pyserver::write_script(&root).await?)
}

/// Where the runtime Python server keeps its venvs, model caches and script.
pub(crate) fn python_server_cache_root(config: &Config) -> PathBuf {
    let configured = config.runtime_python.cache_dir.trim();
    if !configured.is_empty() {
        return PathBuf::from(configured).join("runtime-python-server");
    }
    if let Some(user_cache) = dirs::cache_dir() {
        return user_cache.join("openhuman").join("runtime-python-server");
    }
    config.workspace_dir.join("runtime_python_server")
}

pub async fn request_kompress_compress(
    config: &Config,
    text: &str,
) -> Result<super::kompress::KompressResponse> {
    let server = ensure_started(config).await?;
    Ok(server
        .request(
            "kompress.compress",
            json!({
                "text": text,
                "target_ratio": config.tokenjuice.ml_target_ratio,
                "max_input_chars": config.tokenjuice.ml_max_input_chars,
            }),
        )
        .await?)
}

#[cfg(test)]
#[path = "server_tests.rs"]
mod tests;
