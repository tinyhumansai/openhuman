//! RPC entry points for the about_app capability catalog.

use crate::core::Outcome;

use super::types::{Capability, CapabilityCategory, CapabilityStatus};
use super::{all_capabilities, capabilities_by_category, lookup, search};
use crate::core::runtime::{ComponentSet, DispatchCtx};

/// The `[components]` name backing a capability domain, if one does.
pub fn component_for_domain(domain: &str) -> Option<&'static str> {
    Some(match domain {
        "flows" | "workflow_runs" => "flows",
        "memory" => "memory",
        "channels" => "channels",
        "voice" => "voice",
        "wallet" | "web3" | "x402" => "wallet",
        "tokenjuice" => "juice",
        _ => return None,
    })
}

/// `capability`, marked [`CapabilityStatus::Disabled`] when its component is
/// off in `components`.
pub fn apply_components(capability: Capability, components: &ComponentSet) -> Capability {
    // The sandbox and reply-remix capabilities live in the shared `security`
    // and `conversation` domains, so they are matched by id.
    let component = match capability.id {
        "security.sandbox_backends" => Some("sandbox"),
        "conversation.reply_remix" => Some("memes"),
        // Scheduled jobs run in the tinyflows module.
        "automation.view_cron_jobs"
        | "automation.set_job_intervals"
        | "automation.view_execution_history"
        | "automation.morning_briefing" => Some("tinyflows"),
        _ => component_for_domain(capability.domain),
    };
    match component {
        Some(name) if !components.enabled(name) => Capability {
            status: CapabilityStatus::Disabled,
            ..capability
        },
        _ => capability,
    }
}

pub fn list_capabilities(
    ctx: &DispatchCtx,
    category: Option<CapabilityCategory>,
) -> Outcome<Vec<Capability>> {
    let capabilities = match category {
        Some(category) => capabilities_by_category(category),
        None => all_capabilities().to_vec(),
    };
    let components = ctx.components();
    let capabilities: Vec<_> = capabilities
        .into_iter()
        .map(|c| apply_components(c, &components))
        .collect();
    let log = format!(
        "about_app.list returned {} capabilities",
        capabilities.len()
    );
    Outcome::single_log(capabilities, log)
}

pub fn lookup_capability(ctx: &DispatchCtx, id: &str) -> Result<Outcome<Capability>, String> {
    let capability = lookup(id).ok_or_else(|| format!("unknown capability id '{}'", id.trim()))?;
    let capability = apply_components(capability, &ctx.components());
    Ok(Outcome::single_log(
        capability,
        format!("about_app.lookup returned {}", capability.id),
    ))
}

pub fn search_capabilities(ctx: &DispatchCtx, query: &str) -> Outcome<Vec<Capability>> {
    let components = ctx.components();
    let capabilities: Vec<_> = search(query)
        .into_iter()
        .map(|c| apply_components(c, &components))
        .collect();
    let log = format!(
        "about_app.search returned {} capabilities for '{}'",
        capabilities.len(),
        query.trim()
    );
    Outcome::single_log(capabilities, log)
}

#[cfg(test)]
#[path = "ops_tests.rs"]
mod tests;
