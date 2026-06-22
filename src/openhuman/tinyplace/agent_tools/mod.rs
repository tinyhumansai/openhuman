//! Agent-facing tiny.place tool surface.
//!
//! Replaces the old 1:1 wrapping of ~160 controllers with a small, curated set
//! the LLM can actually reason about:
//!
//! * **flow tools** ([`flows_read`], [`flows_write`]) — one call = one
//!   agent-friendly markdown result with a `Next steps` suggestion block;
//! * **`tinyplace_graphql`** ([`graphql`]) — the batched read gateway;
//! * **`tinyplace_call`** ([`raw`]) — the escape hatch over every controller for
//!   the long tail;
//! * **`tinyplace_help`** ([`help`]) — the operating manual, as markdown.
//!
//! Every tool renders markdown in Rust; the LLM never sees raw JSON. The ~160
//! controllers stay registered for the desktop renderer (see
//! [`super::schemas`]); they are simply no longer advertised one-per-tool.

mod common;
mod flows_read;
mod flows_write;
mod graphql;
mod help;
mod raw;
mod render;
mod suggest;

use crate::openhuman::tools::traits::Tool;

/// The curated tiny.place agent tool set.
pub fn all_tinyplace_agent_tools() -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    tools.extend(flows_read::read_tools());
    tools.extend(flows_write::write_tools());
    tools.push(graphql::graphql_tool());
    tools.push(raw::RawCallTool::boxed());
    tools.push(help::help_tool());
    log::debug!(
        "[tinyplace][flow] assembled {} curated agent tools",
        tools.len()
    );
    tools
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::tools::traits::PermissionLevel;
    use std::collections::HashSet;

    #[test]
    fn curated_surface_is_small_and_named() {
        let tools = all_tinyplace_agent_tools();
        let names: HashSet<&str> = tools.iter().map(|t| t.name()).collect();

        // A handful of flows + the gateway + escape hatch + help — not 160.
        assert!(
            tools.len() < 25,
            "surface should stay small, got {}",
            tools.len()
        );

        for required in [
            "tinyplace_whoami",
            "tinyplace_status",
            "tinyplace_feed",
            "tinyplace_find_work",
            "tinyplace_register",
            "tinyplace_follow",
            "tinyplace_post_bounty",
            "tinyplace_submit_work",
            "tinyplace_job_apply",
            "tinyplace_graphql",
            "tinyplace_call",
            "tinyplace_help",
        ] {
            assert!(
                names.contains(required),
                "missing curated tool `{required}`"
            );
        }
    }

    #[test]
    fn write_flows_are_gated_reads_are_not() {
        let tools = all_tinyplace_agent_tools();
        let by_name = |n: &str| tools.iter().find(|t| t.name() == n).expect(n);

        let register = by_name("tinyplace_register");
        assert_eq!(register.permission_level(), PermissionLevel::Write);
        assert!(register.external_effect());

        let feed = by_name("tinyplace_feed");
        assert_eq!(feed.permission_level(), PermissionLevel::ReadOnly);
        assert!(!feed.external_effect());
    }

    #[test]
    fn every_tool_advertises_markdown() {
        for tool in all_tinyplace_agent_tools() {
            assert!(
                tool.supports_markdown(),
                "{} should support markdown",
                tool.name()
            );
        }
    }
}
