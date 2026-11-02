//! Sandbox backend operations split between host policy and execution adapters.

mod exec;
mod policy;

pub use exec::execute_in_sandbox;
pub(crate) use exec::execute_in_sandbox_yielding;
#[cfg(test)]
pub(crate) use exec::execute_unsandboxed;
#[cfg(any(feature = "modules", test))]
pub(crate) use policy::local_status_for_jail;
#[cfg(test)]
pub(crate) use policy::{config_disables_sandbox, sandbox_disabled_by_host, sandbox_off_value};
pub use policy::{create_sandbox_backend, SANDBOX_ENV_PASSTHROUGH, SANDBOX_OFF_ENV};
pub(crate) use policy::{create_sandbox_backend_with_config, sandbox_component_off};

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
