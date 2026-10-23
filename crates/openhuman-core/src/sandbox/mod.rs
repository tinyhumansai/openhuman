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
pub(crate) mod local_grants;
pub(crate) mod native_jail;
pub(crate) mod operator;
pub mod ops;
#[cfg(feature = "security-module")]
pub mod plan;
pub mod policy;
pub mod schemas;
pub(crate) mod shell_exec;
pub(crate) mod shell_live;
pub(crate) mod shell_processes;
pub(crate) mod shell_progress;
#[cfg(feature = "security-module")]
pub(crate) mod spawn_plan;
pub mod types;
pub(crate) mod untrusted;

pub use ops::{
    build_elevated_op, command_requires_sandbox, create_sandbox_backend, execute_in_sandbox,
    is_elevated_op,
};
pub use policy::spawn_policy;
pub use schemas::{
    all_controller_schemas as all_sandbox_controller_schemas,
    all_registered_controllers as all_sandbox_registered_controllers,
};
pub use types::{
    DockerOverrides, ElevatedOp, SandboxBackendHandle, SandboxBackendKind, SandboxExecRequest,
    SandboxExecResult, SandboxIsolation, SandboxPolicy, SandboxStatus,
};
