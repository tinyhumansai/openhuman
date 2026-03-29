//! JSON-RPC / CLI controller surface for diagnostics.

use crate::openhuman::config::Config;
use crate::openhuman::doctor::{self, DoctorReport, ModelProbeReport};
use crate::rpc::RpcOutcome;

pub async fn doctor_report(config: &Config) -> Result<RpcOutcome<DoctorReport>, String> {
    let report = doctor::run(config).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "doctor report generated"))
}

pub async fn doctor_models(
    config: &Config,
    use_cache: bool,
) -> Result<RpcOutcome<ModelProbeReport>, String> {
    let report = doctor::run_models(config, use_cache).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "model probes completed"))
}
