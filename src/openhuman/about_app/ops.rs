//! RPC entry points for the about_app capability catalog.

use crate::rpc::RpcOutcome;

use super::types::{Capability, CapabilityCategory};
use super::{all_capabilities, capabilities_by_category, lookup, search};

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
