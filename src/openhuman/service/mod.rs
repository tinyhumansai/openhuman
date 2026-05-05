//! Service management helpers for OpenHuman daemon.

pub mod bus;
mod core;
pub mod daemon;
pub mod daemon_host;
pub mod ops;
mod restart;
mod schemas;
mod shutdown;

mod common;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "macos")]
mod macos;
pub(crate) mod mock;
#[cfg(windows)]
mod windows;

pub use core::*;
pub use ops as rpc;
pub use ops::*;
pub use restart::apply_startup_restart_delay_from_env;
pub use restart::RestartStatus;
pub use schemas::{
    all_controller_schemas as all_service_controller_schemas,
    all_registered_controllers as all_service_registered_controllers,
};
pub use shutdown::ShutdownStatus;
