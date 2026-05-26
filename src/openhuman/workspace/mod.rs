//! Workspace layout and bootstrap files (CLI `init` and similar entrypoints).

pub mod ops;
pub mod rpc;
mod schemas;

pub use ops::*;
pub use schemas::{
    all_controller_schemas as all_workspace_controller_schemas,
    all_registered_controllers as all_workspace_registered_controllers,
};
