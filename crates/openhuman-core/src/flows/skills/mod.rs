//! Skills the flows domain ships inside the binary.
//!
//! `flow-authoring` is the Workflow Builder's reference manual. It used to be
//! ~25 KB of the agent's system prompt — paid on every turn of every authoring
//! session, including the turns that only wire two nodes together. As a bundled
//! skill the same text is one `read_workflow_resource` call away and costs
//! nothing until a page is actually needed.
//!
//! The split follows one rule, and it is the reason the whole prompt did not
//! move: **a rule that binds goes in the prompt; a rule you look up goes
//! here.** "Propose, never persist" cannot live in a manual, because a manual
//! only binds a model that chose to open it. An expression's jq syntax is the
//! opposite — nothing goes wrong by not knowing it until you need it, and a
//! lot goes wrong by half-remembering it.
//!
//! The line is not obvious from the outside, and getting it wrong is caught by
//! tests rather than by review: "prefer the minimal viable graph" was moved
//! here on the first pass and moved back, because
//! `standing_prompt_keeps_minimal_graph_warning_alongside_specialist_guidance`
//! pins it in the prompt — correctly. It constrains an instinct the model has
//! before it would think to consult anything.
//!
//! The manual's bytes live in `tinyflows-copilot`; this host only converts the
//! portable file list into its native bundled-skill registration.

use {tinyskills::BundledFile, tinyskills::BundledSkill};

/// The `flow-authoring` bundle, embedded from the sibling directory.
///
/// Listed file by file rather than swept from the directory: a build-time
/// directory walk would silently ship whatever happened to be sitting there,
/// and `include_str!` needs literal paths anyway. The `bundled_skill_matches_
/// the_directory_on_disk` test below fails when a page is added to the
/// directory and not to this list, which is the mistake this shape actually
/// invites.
pub const FLOW_AUTHORING: BundledSkill = BundledSkill {
    dir_name: "flow-authoring",
    files: &[
        BundledFile {
            path: "WORKFLOW.md",
            contents: tinyflows_copilot::resources::FLOW_AUTHORING_WORKFLOW,
        },
        BundledFile {
            path: "references/expressions.md",
            contents: tinyflows_copilot::resources::FLOW_AUTHORING_EXPRESSIONS,
        },
        BundledFile {
            path: "references/node-config.md",
            contents: tinyflows_copilot::resources::FLOW_AUTHORING_NODE_CONFIG,
        },
        BundledFile {
            path: "references/dry-run.md",
            contents: tinyflows_copilot::resources::FLOW_AUTHORING_DRY_RUN,
        },
    ],
};

#[cfg(test)]
#[path = "skills_tests.rs"]
mod tests;
