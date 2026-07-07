//! System prompt builder for the tool-free `master_reporter` built-in.
//!
//! Human-facing OpenHuman archetype + safety + workspace context. Deliberately
//! omits the tiny.place tool belt and the reasoning core's steering seam — this
//! agent only relays an untrusted peer reply into the Master chat, so it carries
//! no tools and no autonomous directive.

use crate::openhuman::context::prompt::{render_safety, render_workspace, PromptContext};
use anyhow::Result;

const ARCHETYPE: &str = include_str!("prompt.md");

pub fn build(ctx: &PromptContext<'_>) -> Result<String> {
    let mut out = String::with_capacity(2048);
    out.push_str(ARCHETYPE.trim_end());
    out.push_str("\n\n");

    let safety = render_safety();
    out.push_str(safety.trim_end());
    out.push_str("\n\n");

    let workspace = render_workspace(ctx)?;
    if !workspace.trim().is_empty() {
        out.push_str(workspace.trim_end());
        out.push('\n');
    }

    Ok(out)
}
