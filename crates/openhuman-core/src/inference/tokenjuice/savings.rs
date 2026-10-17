//! Host adapter for compaction savings accounting.
//!
//! The aggregate math, the per-model and per-compressor buckets and the
//! snapshot at `workspace_dir/state/tokenjuice_savings.json` live in the
//! tinyjuice module (`ConfigureSavings` / `RecordSavings` / `SavingsStats` /
//! `ResetSavings`). OpenHuman keeps only what is policy: which model the
//! saving is attributed to and the USD price per input token from
//! [`crate::agent::cost`].
//!
//! The snapshot file is written by the module itself (`ConfigureSavings` takes
//! its path), not by the host, so it is outside the host's storage-bypass
//! ratchet. Keeping it on the storage port needs a storage-backed savings
//! contract in tinyjuice first; the host passes the path until then.

use std::sync::{Mutex, OnceLock};

use tinyjuice_bus::names::methods;
pub use tinyjuice_bus::savings::{RecordSavingsRequest, SavingsAggregate, SavingsBucket};
use tinyjuice_bus::types::CompressorKind;

const MODULE: &str = "tinyjuice";

/// Model used to price the saved input tokens (the configured default).
fn model_slot() -> &'static Mutex<String> {
    static SLOT: OnceLock<Mutex<String>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(crate::config::DEFAULT_MODEL.to_string()))
}

/// Install the attribution model (host policy) and point the module at the
/// snapshot location; the module loads a prior snapshot once per path.
pub async fn configure(
    client: &tinybus::module::ModuleClient,
    attribution_model: String,
    workspace_dir: &std::path::Path,
) -> Result<(), String> {
    if !attribution_model.trim().is_empty() {
        *model_slot().lock().unwrap_or_else(|p| p.into_inner()) = attribution_model;
    }
    let path = workspace_dir.join("state").join("tokenjuice_savings.json");
    client
        .call::<()>(
            MODULE,
            methods::CONFIGURE_SAVINGS,
            (path.to_string_lossy().into_owned(),),
        )
        .await
        .map_err(|e| e.to_string())
}

/// The model savings are currently attributed to.
pub fn attribution_model() -> String {
    model_slot()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .clone()
}

/// USD per million input tokens for `model`; `None` for a model the catalog
/// does not price, so the saving is counted in tokens only, never guessed.
fn input_per_mtok_usd(model: &str) -> Option<f64> {
    crate::agent::cost::lookup_pricing(model)
        .and_then(|pricing| pricing.input_per_token)
        .map(|per_token| per_token * 1_000_000.0)
}

/// Build the module request for one compaction, or `None` when nothing shrank.
fn request(
    compressor: CompressorKind,
    original_tokens: u64,
    compacted_tokens: u64,
) -> Option<RecordSavingsRequest> {
    request_for(
        attribution_model(),
        compressor,
        original_tokens,
        compacted_tokens,
    )
}

/// [`request`] for an explicit attribution `model`.
fn request_for(
    model: String,
    compressor: CompressorKind,
    original_tokens: u64,
    compacted_tokens: u64,
) -> Option<RecordSavingsRequest> {
    if original_tokens <= compacted_tokens {
        return None;
    }
    Some(RecordSavingsRequest {
        input_per_mtok_usd: input_per_mtok_usd(&model),
        model,
        compressor: compressor.as_str().to_string(),
        original_tokens,
        compacted_tokens,
    })
}

/// Record one compaction's savings through the module.
pub async fn record(
    client: &tinybus::module::ModuleClient,
    compressor: CompressorKind,
    original_tokens: u64,
    compacted_tokens: u64,
) {
    let Some(req) = request(compressor, original_tokens, compacted_tokens) else {
        return;
    };
    if let Err(e) = client
        .call::<()>(MODULE, methods::RECORD_SAVINGS, (req,))
        .await
    {
        log::debug!("[tokenjuice][savings] record failed: {e}");
    }
}

/// Snapshot the module's savings aggregate.
pub async fn stats(client: &tinybus::module::ModuleClient) -> Result<SavingsAggregate, String> {
    client
        .call(MODULE, methods::SAVINGS_STATS, ())
        .await
        .map_err(|e| e.to_string())
}

/// Clear all recorded savings (and the persisted snapshot).
pub async fn reset(client: &tinybus::module::ModuleClient) -> Result<(), String> {
    client
        .call::<()>(MODULE, methods::RESET_SAVINGS, ())
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
#[path = "savings_tests.rs"]
mod tests;
