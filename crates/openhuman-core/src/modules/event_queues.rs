//! Calling the `tinyeventqueues` module over the bus.
//!
//! Each function is the host half of one member of
//! `ai.tinyhumans.tinyeventqueues.EventQueues`. The member names and payloads
//! come from `tinyeventqueues_bus`, so a renamed member is a compile error
//! here rather than an `UnknownMethod` at runtime. Deadlines belong to the
//! caller, as with the other module adapters.

use tinybus::module::{ModuleCallError, ModuleClient};
use tinyeventqueues_bus::names::methods;
use tinyeventqueues_bus::{
    DeadItem, DrainReport, DrainRequest, DrainResponse, InboundEvent, IngestRequest,
    IngestResponse, ListDeadRequest, ListDeadResponse, RedriveRequest, RedriveResponse,
    StatsRequest, StatsResponse,
};

/// Registry id of the module these calls go to.
pub const MODULE_ID: &str = "tinyeventqueues";

/// Durably queue `events`. Returns once the module has written them, so the
/// platform may be acknowledged after it.
///
/// # Errors
///
/// A sanitised [`ModuleCallError`]: the module is unavailable, speaks another
/// contract, or failed the call.
pub async fn ingest(
    client: &ModuleClient,
    events: Vec<InboundEvent>,
) -> Result<IngestResponse, ModuleCallError> {
    tracing::debug!(count = events.len(), "[inbound][module] Ingest");
    client
        .call(MODULE_ID, methods::INGEST, (IngestRequest::new(events),))
        .await
}

/// Queue depths and outcome counts.
///
/// # Errors
///
/// As [`ingest`].
pub async fn stats(client: &ModuleClient) -> Result<StatsResponse, ModuleCallError> {
    client
        .call(MODULE_ID, methods::STATS, (StatsRequest::new(),))
        .await
}

/// Up to `limit` dead-lettered items.
///
/// # Errors
///
/// As [`ingest`].
pub async fn list_dead(
    client: &ModuleClient,
    limit: u32,
) -> Result<Vec<DeadItem>, ModuleCallError> {
    let response: ListDeadResponse = client
        .call(
            MODULE_ID,
            methods::LIST_DEAD,
            (ListDeadRequest::new(limit),),
        )
        .await?;
    Ok(response.dead)
}

/// Put the dead letter `id` back into the pipeline. `false` when it was
/// already gone.
///
/// # Errors
///
/// As [`ingest`]; an unknown id is a module fault.
pub async fn redrive(client: &ModuleClient, id: &str) -> Result<bool, ModuleCallError> {
    let response: RedriveResponse = client
        .call(MODULE_ID, methods::REDRIVE, (RedriveRequest::new(id),))
        .await?;
    Ok(response.redriven)
}

/// Run one pipeline pass now. The module runs passes on its own timer; this is
/// for diagnostics and tests.
///
/// # Errors
///
/// As [`ingest`].
pub async fn drain(client: &ModuleClient) -> Result<DrainReport, ModuleCallError> {
    let response: DrainResponse = client
        .call(MODULE_ID, methods::DRAIN, (DrainRequest::new(),))
        .await?;
    Ok(response.report)
}

#[cfg(test)]
#[path = "event_queues_tests.rs"]
mod tests;
