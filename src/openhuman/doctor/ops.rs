//! JSON-RPC / CLI controller surface for diagnostics.

use crate::openhuman::config::Config;
use crate::openhuman::doctor::{self, DoctorReport, ModelProbeReport};
use crate::rpc::RpcOutcome;

pub async fn doctor_report(config: &Config) -> Result<RpcOutcome<DoctorReport>, String> {
    // `doctor::run` calls `check_embedding_model_health` which uses
    // `reqwest::blocking::Client` — that panics inside a tokio runtime.
    // Move the entire sync `run()` onto a blocking thread.
    let config_clone = config.clone();
    let report = tokio::task::spawn_blocking(move || doctor::run(&config_clone))
        .await
        .map_err(|e| format!("doctor task join error: {e}"))?
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "doctor report generated"))
}

pub async fn doctor_models(
    config: &Config,
    use_cache: bool,
) -> Result<RpcOutcome<ModelProbeReport>, String> {
    let report = doctor::run_models(config, use_cache).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "model probes completed"))
}
