//! The `subsystems` status namespace: one wire row per capability slot.
//!
//! `docs/specs/kernel.md` §3 (the model) and §6 item 6 (the status projection).
//!
//! ## Scope
//!
//! This module owns the generic wire shape ([`SubsystemStatus`]) and the
//! `subsystems.status` RPC plus the `openhuman subsystems` CLI table built on
//! it. Memory is the only occupant: [`crate::memory::status`] fills its row
//! from the bound Memory v2 engine, and a future subsystem appends its own
//! adapter call in [`schemas::subsystems_status`]. There is no in-process
//! driver registry; each slot reports its own state.

pub mod schemas;
mod status;

pub use schemas::{
    all_controller_schemas as all_subsystems_controller_schemas,
    all_registered_controllers as all_subsystems_registered_controllers, subsystems_status,
};
pub use status::SubsystemStatus;
