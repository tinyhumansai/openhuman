//! The memory row of the kernel's subsystem table (`subsystems.status`).

use crate::config::Config;
use crate::core::subsystem::SubsystemStatus;

use super::engine::{self, Binding};

/// Contract version the memory slot speaks: TinyMemory's v2 engine contract.
pub const CONTRACT_VERSION: &str = "2.0";

/// The operations every v2 engine serves.
const OPERATIONS: [&str; 5] = ["recall", "fetch", "store", "forget", "list"];

/// The memory slot's status for `config`: the bound engine and its health, or
/// a `null` row with the reason memory is off.
pub async fn subsystem_status(config: &Config) -> SubsystemStatus {
    match engine::resolve(config) {
        Binding::On(bound) => {
            let (health, reason) = match bound.engine.health().await {
                tinymemory::EngineHealth::Ok => ("ready", None),
                tinymemory::EngineHealth::Degraded(reason) => ("degraded", Some(reason)),
                tinymemory::EngineHealth::Down(reason) => ("down", Some(reason)),
            };
            SubsystemStatus {
                slot: "memory".to_string(),
                driver: bound.id,
                class: "external".to_string(),
                health: health.to_string(),
                health_reason: reason,
                contract_version: CONTRACT_VERSION.to_string(),
                capabilities: OPERATIONS.iter().map(|op| (*op).to_string()).collect(),
                fell_back_from: None,
                last_error: None,
            }
        }
        Binding::Off { engine, reason, .. } => SubsystemStatus {
            slot: "memory".to_string(),
            driver: "null".to_string(),
            class: "null".to_string(),
            health: "down".to_string(),
            health_reason: Some(reason),
            contract_version: CONTRACT_VERSION.to_string(),
            capabilities: Vec::new(),
            fell_back_from: engine,
            last_error: None,
        },
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
