//! Moving memory from the legacy CortexDB tree into the per-user tree.
//!
//! The engine-to-engine migration (copy, placement, cleanup, the job and its
//! service) lives in `tinymemory_tools::layout_migration`; this module is the
//! OpenHuman host adapter over its two ports:
//!
//! - [`state`]: the file-backed [`MigrationStateStore`] under the workspace.
//! - [`app_host`]: the [`LayoutHost`] for one account (engine binding,
//!   placement, the layout switch, the free period), with [`claim`] naming the
//!   account that takes a legacy tree several accounts share.
//! - [`service`]: running in the background (one run per workspace, gated on
//!   the import) and mapping the module's errors to [`MemoryError`].
//!
//! [`MemoryError`]: crate::memory::error::MemoryError

pub mod app_host;
pub mod claim;
pub mod service;
pub mod state;

#[cfg(test)]
mod test_host;

pub use app_host::AppHost;
pub use service::{retry, scan, start, status, tick, StartParams};
pub use state::FileStateStore;
pub use tinymemory_tools::layout_migration::{
    LayoutHost, MigrationState, MigrationStateStore, MigrationStatus, Outcome, Phase, ScanView,
    Trigger,
};
