//! The subsystem registry — one bound driver per capability slot.
//!
//! `docs/specs/kernel.md` §3 (the model), §3.7 (three runtime axes), §6 item 1.
//!
//! ## Scope
//!
//! This module is the **generic half** of kernel.md §6 item 1: `DriverClass`,
//! `DriverHealth`, a generic capability set, and `SubsystemRegistry`. It is
//! deliberately subsystem-agnostic and names no subsystem's contract crate —
//! whichever subsystem is cut over after memory (inference, channels, sandbox)
//! uses this same registry, so it must not inherit its vocabulary from a
//! *memory* crate.
//!
//! Since M2b the memory adapter exists in
//! [`crate::openhuman::memory::binding`] (it converts
//! `tinymemory_api::MemoryHealth` into [`DriverHealth`] and the contract's
//! typed capability set into [`DriverCapabilities`]), and M2c added the
//! read-only [`status`] projection plus the `subsystems` RPC namespace and the
//! `openhuman subsystems` CLI table.
//!
//! Still to land in later steps, despite being named in the same §6 sentence:
//! the generic `Driver` trait and the policy `Guard` (§3.4).

mod driver;
mod registry;
pub mod schemas;
mod status;

pub use driver::{DriverCapabilities, DriverClass, DriverHealth};
pub use registry::{BoundDriver, SubsystemRegistry, SubsystemSlot};
pub use schemas::{
    all_controller_schemas as all_subsystems_controller_schemas,
    all_registered_controllers as all_subsystems_registered_controllers, subsystems_status,
};
pub use status::{format_contract_version, registry_status, SubsystemStatus};
