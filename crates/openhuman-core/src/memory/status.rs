//! The memory row of the kernel's subsystem table (`subsystems.status`).

use crate::config::Config;
use crate::core::subsystem::SubsystemStatus;

use super::engine::{self, Binding};

/// The memory slot's status for `config`: the bound engine and its health, or
/// a `null` row with the reason memory is off.
pub async fn subsystem_status(config: &Config) -> SubsystemStatus {
    match engine::resolve(config).await {
        Binding::On(bound) => {
            let status = tinymemory_api::EngineStatus::from_engine(
                bound.engine.descriptor(),
                bound.engine.health().await,
            );
            subsystem_status_for_engine(bound.id, status)
        }
        Binding::Off { engine, reason, .. } => SubsystemStatus {
            slot: "memory".to_string(),
            driver: "null".to_string(),
            class: "null".to_string(),
            health: "down".to_string(),
            health_reason: Some(reason),
            contract_version: tinymemory_api::CONTRACT_VERSION.to_string(),
            capabilities: Vec::new(),
            fell_back_from: engine,
            last_error: None,
        },
    }
}

fn subsystem_status_for_engine(
    driver: String,
    status: tinymemory_api::EngineStatus,
) -> SubsystemStatus {
    let health = match status.health {
        tinymemory_api::EngineStatusHealth::Ready => "ready",
        tinymemory_api::EngineStatusHealth::Degraded => "degraded",
        tinymemory_api::EngineStatusHealth::Down | tinymemory_api::EngineStatusHealth::Off => {
            "down"
        }
    };
    SubsystemStatus {
        slot: "memory".to_string(),
        driver,
        class: "external".to_string(),
        health: health.to_string(),
        health_reason: status.reason,
        contract_version: status.contract_version.to_string(),
        capabilities: status
            .capabilities
            .into_iter()
            .map(str::to_string)
            .collect(),
        fell_back_from: None,
        last_error: None,
    }
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
