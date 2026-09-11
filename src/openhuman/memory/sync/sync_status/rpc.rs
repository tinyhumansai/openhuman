//! OpenHuman RPC shell for memory synchronization status.

use crate::openhuman::config::Config;
use crate::rpc::RpcOutcome;

use tinymemory_api::provider::sync::{SourceSyncStatus, SyncFreshness};

use super::{FreshnessLabel, MemorySyncStatus, StatusListResponse};

/// Carry one status across as the wire type this RPC has always answered.
///
/// Field-for-field, and that is checkable rather than hopeful: the contract's
/// `SourceSyncStatus` was modelled on the driver's own row, so `provider`,
/// `chunks_synced`, `chunks_pending`, `batch_total`, `batch_processed` and
/// `last_chunk_at_ms` are the same names carrying the same values, and both
/// freshness enums are the same three variants under the same
/// `rename_all = "snake_case"`. The response body is therefore byte-identical
/// to what the engine call produced — `response_keeps_top_level_statuses_array`
/// still holds.
///
/// The destructure is deliberate and is the reason this function is not a
/// `From` impl over `..`: it is the one seam between the contract's shape and
/// the published one, so a field added, removed or renamed upstream has to fail
/// here rather than quietly change what `memory_sync.status_list` answers.
pub(super) fn into_wire(status: SourceSyncStatus) -> MemorySyncStatus {
    let SourceSyncStatus {
        provider,
        chunks_synced,
        chunks_pending,
        batch_total,
        batch_processed,
        last_chunk_at_ms,
        freshness,
    } = status;
    MemorySyncStatus {
        provider,
        chunks_synced,
        chunks_pending,
        batch_total,
        batch_processed,
        last_chunk_at_ms,
        freshness: match freshness {
            SyncFreshness::Active => FreshnessLabel::Active,
            SyncFreshness::Recent => FreshnessLabel::Recent,
            SyncFreshness::Idle => FreshnessLabel::Idle,
        },
    }
}

pub async fn status_list_rpc(config: &Config) -> Result<RpcOutcome<StatusListResponse>, String> {
    tracing::debug!("[memory_sync_status][rpc] status_list via the bound driver");

    // Degrading a failure to an empty list is inherited behaviour, kept
    // deliberately rather than tightened here: this surface renders a status
    // table, and every caller of it today treats "no rows" as "nothing syncing".
    // Turning a backend blip into an RPC error is a real UI change and belongs
    // with whoever owns that screen, not smuggled into a routing swap. The warn
    // is what makes the difference visible in the meantime.
    let statuses = match crate::openhuman::memory::binding::for_config(config) {
        Ok(binding) => match binding.provider().as_source_sync() {
            Some(sync) => match sync.sync_statuses().await {
                Ok(statuses) => statuses.into_iter().map(into_wire).collect(),
                Err(error) => {
                    tracing::warn!(%error, "[memory_sync_status][rpc] driver status query failed");
                    Vec::new()
                }
            },
            None => {
                tracing::debug!(
                    driver = %binding.driver_id(),
                    "[memory_sync_status][rpc] driver does not serve SourceSync; reporting empty"
                );
                Vec::new()
            }
        },
        Err(error) => {
            tracing::warn!(%error, "[memory_sync_status][rpc] memory binding failed");
            Vec::new()
        }
    };

    Ok(RpcOutcome::new(StatusListResponse { statuses }, Vec::new()))
}

#[cfg(test)]
#[path = "rpc_tests.rs"]
mod tests;
