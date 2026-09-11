//! JSON-RPC / CLI controller surface for diagnostics.
//!
//! Also the home of the async probes [`doctor::run`] cannot take itself. It is
//! blocking by contract (see its docs), so anything that has to be `await`ed
//! is resolved here and passed down as a value.

use crate::openhuman::config::Config;
use crate::openhuman::platform::doctor::{self, DoctorReport, MemoryChunkCount, ModelProbeReport};
use crate::rpc::RpcOutcome;

pub async fn doctor_report(config: &Config) -> Result<RpcOutcome<DoctorReport>, String> {
    // Awaited before the blocking hop, not inside it: `doctor::run` may not
    // `.await`, and the driver call may not be blocked on from a runtime
    // worker. See `MemoryChunkCount`.
    let memory_chunks = memory_chunk_count(config).await;

    // `doctor::run` calls `check_embedding_model_health` which uses
    // `reqwest::blocking::Client` — that panics inside a tokio runtime.
    // Move the entire sync `run()` onto a blocking thread.
    let config_clone = config.clone();
    let report = tokio::task::spawn_blocking(move || doctor::run(&config_clone, memory_chunks))
        .await
        .map_err(|e| format!("doctor task join error: {e}"))?
        .map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "doctor report generated"))
}

/// How many chunks the workspace's bound memory driver holds.
///
/// `MemoryMaintenance::store_stats` is the driver-neutral answer to the
/// question `check_memory_tree_db` used to put to the engine's SQLite
/// connection directly (#5560).
///
/// A driver that does not serve `Maintenance` is reported as a **failed
/// probe**, not as zero — and that is the one judgement call in here. The
/// sibling status surfaces (`memory_tree`'s `store_stats`/`queue_stats`)
/// degrade a missing family to an empty snapshot, because a status panel that
/// errors tells the user less than one showing an empty store. The doctor is
/// the opposite surface: it exists to name what is wrong, and "0 chunks" from
/// a driver that cannot count is indistinguishable from a memory store the
/// user has just watched themselves fill.
async fn memory_chunk_count(config: &Config) -> MemoryChunkCount {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let Some(maintenance) = binding.provider().as_maintenance() else {
        return Err(format!(
            "driver '{}' does not serve Maintenance",
            binding.driver_id()
        ));
    };
    let stats = maintenance
        .store_stats()
        .await
        .map_err(|e| format!("store_stats: {e}"))?;
    log::debug!(
        "[doctor] memory_chunk_count: driver='{}' chunks={}",
        binding.driver_id(),
        stats.chunks
    );
    Ok(stats.chunks)
}

pub async fn doctor_models(
    config: &Config,
    use_cache: bool,
) -> Result<RpcOutcome<ModelProbeReport>, String> {
    let report = doctor::run_models(config, use_cache).map_err(|e| e.to_string())?;
    Ok(RpcOutcome::single_log(report, "model probes completed"))
}
