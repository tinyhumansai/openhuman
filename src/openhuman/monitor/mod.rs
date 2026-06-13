//! Background monitor domain.
//!
//! Monitors are first-class, core-owned background event sources for agent
//! sessions. The domain owns lifecycle, bounded output storage, event
//! publishing, and agent tool wrappers.

pub mod ops;
pub mod runner;
pub mod schemas;
pub mod store;
pub mod tools;
pub mod types;

pub use schemas::{all_monitor_controller_schemas, all_monitor_registered_controllers};
