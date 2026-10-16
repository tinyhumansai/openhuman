//! Media generation domain — image and video generation agent tools over
//! generators a host supplies (the core ships none of its own).
//!
//! The work is split by ownership:
//!
//! - **TinyInference** (`tinyinference-image` / `tinyinference-video`, reached
//!   through `tinyagents_harness`) owns the wire contract, reference and
//!   output-shape standards, the submit → poll → download job loop, and the
//!   rule that a billed call returns media or an error, never an empty success.
//! - **TinyAgents** (`tinyagents_harness::media`) owns the tools: argument
//!   parsing, artifact persistence into the workspace, and result wording.
//! - **This module** owns tool names, descriptions and the local-reference
//!   policy ([`tools`]), and the [`MediaGenerators`] pair the tools run over
//!   ([`provider`]). The host that owns the media service (endpoint, credential,
//!   egress, privacy and budget gates) builds the generators and registers the
//!   tools through a tool source.

mod artifact_tool;
mod progress;
pub mod provider;
pub mod tools;

pub use artifact_tool::MediaArtifactTool;
pub use provider::{GeneratorRouter, MediaGenerators};
pub use tools::{
    media_tools_for_config, media_tools_from, MediaListModelsTool, MediaReferenceAllowlist,
    IMAGE_TOOL_NAME, LIST_MODELS_TOOL_NAME, VIDEO_TOOL_NAME,
};
