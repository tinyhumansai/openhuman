//! Sandbox backend operations split between host policy and execution adapters.

mod exec;
mod policy;

pub use exec::execute_in_sandbox;
#[cfg(test)]
pub(crate) use exec::execute_unsandboxed;
pub(crate) use exec::{execute_resolved, resolve_sandbox};
#[cfg(any(feature = "modules", test))]
pub(crate) use policy::local_status_for_jail;
#[cfg(test)]
pub(crate) use policy::{
    config_disables_sandbox, config_keeps_boxes, sandbox_disabled_by_host, sandbox_off_value,
};
pub use policy::{create_sandbox_backend, SANDBOX_ENV_PASSTHROUGH, SANDBOX_OFF_ENV};
pub(crate) use policy::{
    create_sandbox_backend_with_config, sandbox_component_off, sandbox_keeps_boxes,
};

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
