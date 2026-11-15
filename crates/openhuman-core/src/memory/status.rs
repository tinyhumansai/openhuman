//! The kernel's subsystem table (`docs/specs/kernel.md` §3 the model, §6 item
//! 6 the status projection): the `subsystems.status` RPC, the bare
//! `openhuman subsystems` CLI table, the generic wire row they both render
//! ([`SubsystemStatus`]), and the memory row that fills it.
//!
//! ## Why this lives in `memory`
//!
//! Memory is the only subsystem slot, so the slot table is a projection of
//! the bound Memory v2 engine and has no other owner. The controller is still
//! tagged `DomainGroup::Platform` in `core::all` (not `Memory`), so its
//! availability under `DomainSet::harness()` is unchanged: like `health`, it
//! is absent there, while `memory.provider_status` stays reachable. A future
//! subsystem appends its own adapter call in [`subsystems_status`]; the row
//! shape is already generic, so that is a one-line change with no wire change
//! for existing rows. There is no in-process driver registry; each slot
//! reports its own state.
//!
//! ## Capabilities cross the wire as opaque strings, never as a typed set
//!
//! A driver speaking a newer minor contract may legitimately advertise an
//! operation this build has never heard of, and status must be able to
//! *report* what it saw. So the payload carries `Vec<String>` and
//! [`SubsystemStatus`] derives `Serialize` **only**. Do not add
//! `Deserialize`, and do not retype `capabilities` as a contract type.
//!
//! Health is flattened to a `health` discriminant plus an optional reason so
//! a status consumer can render an unfamiliar driver without a total `match`.
//!
//! ## The CLI table
//!
//! [`run_subsystems_command`] is reached through the `RegisteredCliAdapter`
//! seam ([`crate::core::all::cli_handler_for_namespace`]), which
//! `run_namespace_command` consults when the namespace is invoked with no
//! function or with `--help`. `openhuman subsystems status` bypasses it and
//! prints the raw JSON through the generic namespace dispatcher, so there is
//! no hand-written subcommand match arm anywhere — registering the controller
//! is what makes the subcommand exist.
//!
//! ```text
//! openhuman subsystems           # table
//! openhuman subsystems status    # JSON
//! ```

use anyhow::Result;
use serde::Serialize;
use serde_json::{Map, Value};

use crate::config::Config;
use crate::core::all::{ControllerFuture, RegisteredController};
use crate::core::runtime::DispatchCtx;
use crate::core::Outcome;
use crate::core::{ControllerSchema, FieldSchema, TypeSchema};

use super::engine::{self, Binding};

/// One subsystem slot's status, as rendered on the wire.
///
/// `Serialize` only — see the module docs.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SubsystemStatus {
    /// The slot name: `"memory"`, `"inference"`, …
    pub slot: String,
    /// The bound driver id, e.g. `"tinyhumans"`, `"cortexdb"`, `"null"`.
    pub driver: String,
    /// How the host bound it: `"embedded"` | `"external"` | `"null"`.
    pub class: String,
    /// Liveness discriminant: `"ready"` | `"degraded"` | `"down"`.
    pub health: String,
    /// Operator-facing reason when degraded or down; `None` when ready.
    /// Never contains a credential, endpoint token, or user memory content.
    pub health_reason: Option<String>,
    /// The `(major, minor)` contract version the driver speaks, rendered as
    /// `"<major>.<minor>"`. Display data — version *compatibility* is decided
    /// by the contract crate, never by string-comparing this field.
    pub contract_version: String,
    /// Advertised capability families as opaque strings. See the module docs.
    pub capabilities: Vec<String>,
    /// The driver id that was asked for and failed, when this binding is a
    /// fallback (kernel.md §3.7 — a fallback is never silent). `None` for a
    /// normal bind.
    pub fell_back_from: Option<String>,
    /// Last bind or call failure, operator-facing. `None` when clean. Subject
    /// to the same redaction rule as `health_reason`.
    pub last_error: Option<String>,
}

// ---------------------------------------------------------------------------
// The memory row
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// The `subsystems` RPC namespace
// ---------------------------------------------------------------------------

pub fn all_subsystems_controller_schemas() -> Vec<ControllerSchema> {
    vec![subsystems_schemas("status")]
}

pub fn all_subsystems_registered_controllers() -> Vec<RegisteredController> {
    vec![RegisteredController {
        schema: subsystems_schemas("status"),
        handler: handle_status,
    }]
}

pub fn subsystems_schemas(function: &str) -> ControllerSchema {
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
/// [`run_subsystems_command`] renders as a table.
pub async fn subsystems_status(ctx: &DispatchCtx) -> Vec<SubsystemStatus> {
    match ctx.load_config().await {
        Ok(config) => vec![subsystem_status(&config).await],
        Err(error) => {
            log::warn!("[subsystems] config unavailable for status: {error}");
            Vec::new()
        }
    }
}

fn handle_status(ctx: DispatchCtx, _params: Map<String, Value>) -> ControllerFuture {
    Box::pin(async move {
        let rows = subsystems_status(&ctx).await;
        log::debug!("[subsystem] status requested: {} slot(s)", rows.len());
        Outcome::new(serde_json::json!({ "subsystems": rows }), vec![]).into_cli_compatible_json()
    })
}

// ---------------------------------------------------------------------------
// `openhuman subsystems` — the human-readable slot table
// ---------------------------------------------------------------------------

pub fn run_subsystems_command(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "-h" || a == "--help") {
        print_help();
        return Ok(());
    }

    // A current-thread runtime is enough: a status call probes a bound driver's
    // health and touches no orchestrator, unlike the generic dispatcher's
    // multi-thread runtime with an enlarged agent worker stack.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    let rows = rt.block_on(cli_subsystems_status());

    println!(
        "{:<10} {:<14} {:<9} {:<9} {:<9} CAPABILITIES",
        "SLOT", "DRIVER", "CLASS", "HEALTH", "CONTRACT"
    );
    for row in &rows {
        let driver = if row.driver.is_empty() {
            "-"
        } else {
            row.driver.as_str()
        };
        let capabilities = if row.capabilities.is_empty() {
            "-".to_string()
        } else {
            row.capabilities.join(",")
        };
        println!(
            "{:<10} {:<14} {:<9} {:<9} {:<9} {}",
            row.slot, driver, row.class, row.health, row.contract_version, capabilities
        );
        if let Some(reason) = &row.health_reason {
            println!("  health: {reason}");
        }
        if let Some(previous) = &row.fell_back_from {
            println!("  fell back from: {previous}");
        }
        if let Some(err) = &row.last_error {
            println!("  last error: {err}");
        }
    }
    Ok(())
}

/// The slot table for a standalone CLI invocation: the same rows the
/// `subsystems status` RPC returns, resolved from the on-disk config.
async fn cli_subsystems_status() -> Vec<SubsystemStatus> {
    // A standalone CLI call is the host's one context.
    subsystems_status(&DispatchCtx::host_default()).await
}

fn print_help() {
    println!("openhuman subsystems — kernel subsystem slots and their bound drivers");
    println!();
    println!("USAGE:");
    println!("  openhuman subsystems           Print the slot table");
    println!("  openhuman subsystems status    Print the same data as JSON");
}

#[cfg(test)]
#[path = "status_tests.rs"]
mod tests;
