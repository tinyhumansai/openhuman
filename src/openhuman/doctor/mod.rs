//! Diagnostic checks for OpenHuman configuration, workspace health, and daemon state.

mod core;
pub mod ops;
mod schemas;
pub mod tools;

pub use core::*;
pub use ops as rpc;
pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_doctor_controller_schemas,
    all_registered_controllers as all_doctor_registered_controllers,
};
