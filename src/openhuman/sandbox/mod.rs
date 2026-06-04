//! Sandbox execution backends for agent tool isolation.
//!
//! Separates three concerns:
//! - **Where** tools run (this module — sandbox backend selection)
//! - **Which** tools are allowed (security policy / tool policy)
//! - **Whether** a tool needs host access (elevated ops)
//!
//! The gateway/core always runs on the host. Selected tool families
//! (shell, filesystem, process) execute inside a sandbox with controlled
//! workspace mounts, network policy, environment passthrough, and
//! explicit elevated escape paths.

pub mod docker;
pub mod ops;
pub mod schemas;
pub mod types;

pub use ops::{
    build_elevated_op, create_sandbox_backend, execute_in_sandbox, is_elevated_op,
    resolve_sandbox_policy,
};
pub use schemas::{
    all_controller_schemas as all_sandbox_controller_schemas,
    all_registered_controllers as all_sandbox_registered_controllers,
};
pub use types::{
    DockerOverrides, ElevatedOp, SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest,
    SandboxExecResult, SandboxPolicy, SandboxStatus,
};
