//! Service management helpers for OpenHuman daemon.
//!
//! The OS managers (launchd, systemd, schtasks, E2E mock) live in
//! `openhuman-platform`; [`spec`] describes the core daemon to them.

pub mod bus;
pub mod daemon_host;
pub mod ops;
mod restart;
mod schemas;
mod shutdown;
mod spec;
pub mod tools;

pub use ops as rpc;
pub use ops::*;
pub use restart::apply_startup_restart_delay_from_env;
pub use restart::RestartStatus;
pub use schemas::{
    all_controller_schemas as all_service_controller_schemas,
    all_registered_controllers as all_service_registered_controllers,
};
pub use shutdown::ShutdownStatus;
pub use spec::{install, start, status, stop, uninstall};
