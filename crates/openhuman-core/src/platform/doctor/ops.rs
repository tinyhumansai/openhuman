//! JSON-RPC / CLI controller surface for diagnostics.
//!
//! Also the home of the async probes [`doctor::run`] cannot take itself. It is
//! blocking by contract (see its docs), so anything that has to be `await`ed
//! is resolved here and passed down as a value.

use crate::config::Config;
use crate::core::Outcome;
use crate::platform::doctor::{self, DoctorReport, MemoryEngineCheck, ModelProbeReport};

pub async fn doctor_report(config: &Config) -> Result<Outcome<DoctorReport>, String> {
    // Awaited before the blocking hop, not inside it: `doctor::run` may not
    // `.await`. See `MemoryEngineCheck`.
    let memory = memory_engine_check(config).await;

    // `doctor::run` calls `check_embedding_model_health` which uses
    // `reqwest::blocking::Client` — that panics inside a tokio runtime.
    // Move the entire sync `run()` onto a blocking thread.
    let config_clone = config.clone();
    let report = tokio::task::spawn_blocking(move || doctor::run(&config_clone, memory))
        .await
        .map_err(|e| format!("doctor task join error: {e}"))?
        .map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(report, "doctor report generated"))
}

/// The memory engine's status for the doctor report.
async fn memory_engine_check(config: &Config) -> MemoryEngineCheck {
    let view = crate::memory::ops::engine_get(config).await;
    let status = serde_json::to_value(view.status)
        .ok()
        .and_then(|value| value.as_str().map(str::to_string))
        .unwrap_or_else(|| "down".to_string());
    MemoryEngineCheck {
        engine: view.engine,
        status,
        reason: view.reason,
    }
}

pub async fn doctor_models(
    config: &Config,
    use_cache: bool,
) -> Result<Outcome<ModelProbeReport>, String> {
    let report = doctor::run_models(config, use_cache).map_err(|e| e.to_string())?;
    Ok(Outcome::single_log(report, "model probes completed"))
}
