//! The `subsystems` RPC namespace — one row per capability slot
//! (`docs/specs/kernel.md` §6 item 6).
//!
//! ## Why a controller lives under `crates/openhuman-core/src/core/`
//!
//! `AGENTS.md` says `crates/openhuman-core/src/core/` is transport only. This is the one deliberate
//! exception, and it is narrow: the subsystem registry *is* a kernel binding
//! table — the same category as `core::all`'s controller registry — and there
//! is no `crates/openhuman-core/src/` family that owns it. Giving it one would mean a new
//! `DomainGroup` variant plus the four compiler-enforced edits and three
//! drift-guard lists that come with it, for a single read-only function. So it
//! is registered from here, tagged `DomainGroup::Platform`.
//!
//! ## Aggregation
//!
//! Today `memory` is the only occupant, so the aggregate is one call into the
//! memory adapter. Each future subsystem appends its own adapter call here as
//! it is cut over; the *shape* of a row is already generic
//! ([`SubsystemStatus`]), so adding one is a one-line change with no wire
//! change for existing rows.

use serde_json::{Map, Value};

use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::status::SubsystemStatus;

pub fn all_controller_schemas() -> Vec<ControllerSchema> {
    vec![schemas("status")]
}

pub fn all_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: schemas("status"),
        handler: handle_status,
    }]
}

pub fn schemas(function: &str) -> ControllerSchema {
    match function {
        "status" => ControllerSchema {
            namespace: "subsystems",
            function: "status",
            description: "List every subsystem slot with its bound driver, class, health, contract version, and advertised capability families.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "subsystems",
                ty: TypeSchema::Json,
                comment: "One entry per slot: { slot, driver, class, health, health_reason, contract_version, capabilities[], fell_back_from, last_error }.",
                required: true,
            }],
        },
        _ => ControllerSchema {
            namespace: "subsystems",
            function: "unknown",
            description: "Unknown subsystems controller function.",
            inputs: vec![],
            outputs: vec![FieldSchema {
                name: "error",
                ty: TypeSchema::String,
                comment: "Lookup error details.",
                required: true,
            }],
        },
    }
}

/// Every subsystem slot's status, in slot declaration order.
///
/// Memory is the only occupant today. This is also what
/// [`crate::core::subsystems_cli`] renders as a table.
pub async fn subsystems_status() -> Vec<SubsystemStatus> {
    match crate::config::rpc::load_config_with_timeout().await {
        Ok(config) => vec![crate::memory::status::subsystem_status(&config).await],
        Err(error) => {
            log::warn!("[subsystems] config unavailable for status: {error}");
            Vec::new()
        }
    }
}

fn handle_status(_params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rows = subsystems_status().await;
        log::debug!("[subsystem] status requested: {} slot(s)", rows.len());
        Outcome::new(serde_json::json!({ "subsystems": rows }), vec![]).into_cli_compatible_json()
    })
}

#[cfg(test)]
#[path = "schemas_tests.rs"]
mod tests;
