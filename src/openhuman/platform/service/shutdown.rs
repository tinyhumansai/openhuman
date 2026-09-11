//! Core graceful-shutdown orchestration for the service domain.
//!
//! Mirrors [`super::restart`] but exits the running core process instead of
//! respawning it. RPC/CLI callers acknowledge the request and publish an
//! event; a long-lived subscriber performs the actual `process::exit`. The
//! split keeps the in-process trigger paths (RPC, CLI, internal) sharing one
//! shutdown execution path with the same logging.

use serde::Serialize;

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::rpc::RpcOutcome;

/// JSON-serializable acknowledgement returned to CLI / JSON-RPC callers
/// before the current process exits.
#[derive(Debug, Clone, Serialize)]
pub struct ShutdownStatus {
    pub accepted: bool,
    pub source: String,
    pub reason: String,
}

/// Accepts a shutdown request and publishes it to the global event bus.
///
/// Does not exit directly — the work is performed by
/// [`super::bus::ShutdownSubscriber`] so every in-process trigger uses the
/// same execution path.
pub async fn service_shutdown(
    source: Option<String>,
    reason: Option<String>,
) -> Result<RpcOutcome<ShutdownStatus>, String> {
    let source = source
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "jsonrpc".to_string());
    let reason = reason
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "service.shutdown".to_string());

    let _ = crate::core::bus::init().await;
    log::info!(
        "[service:shutdown] accepted shutdown request source={} reason={}",
        source,
        reason
    );
    BUS.publish(DomainEvent::SystemShutdownRequested {
        source: source.clone(),
        reason: reason.clone(),
    });

    Ok(RpcOutcome::single_log(
        ShutdownStatus {
            accepted: true,
            source,
            reason,
        },
        "service shutdown requested",
    ))
}

#[cfg(test)]
#[path = "shutdown_tests.rs"]
mod tests;
