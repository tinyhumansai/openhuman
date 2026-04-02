//! User-facing capability catalog for the OpenHuman app.
//!
//! This module is the single source of truth for what the desktop app exposes
//! to end users, including where a capability lives in the UI and whether it is
//! stable, beta, coming soon, or deprecated.

mod catalog;
mod schemas;
mod types;

use crate::rpc::RpcOutcome;

pub use catalog::{all_capabilities, capabilities_by_category, lookup, search};
pub use schemas::{
    about_app_schemas, all_about_app_controller_schemas, all_about_app_registered_controllers,
};
pub use types::{Capability, CapabilityCategory, CapabilityStatus};

pub fn list_capabilities(category: Option<CapabilityCategory>) -> RpcOutcome<Vec<Capability>> {
    let capabilities = match category {
        Some(category) => capabilities_by_category(category),
        None => all_capabilities().to_vec(),
    };
    let log = format!(
        "about_app.list returned {} capabilities",
        capabilities.len()
    );
    RpcOutcome::single_log(capabilities, log)
}

pub fn lookup_capability(id: &str) -> Result<RpcOutcome<Capability>, String> {
    let capability = lookup(id).ok_or_else(|| format!("unknown capability id '{}'", id.trim()))?;
    Ok(RpcOutcome::single_log(
        capability,
        format!("about_app.lookup returned {}", capability.id),
    ))
}

pub fn search_capabilities(query: &str) -> RpcOutcome<Vec<Capability>> {
    let capabilities = search(query);
    let log = format!(
        "about_app.search returned {} capabilities for '{}'",
        capabilities.len(),
        query.trim()
    );
    RpcOutcome::single_log(capabilities, log)
}
