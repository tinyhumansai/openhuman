//! Media generation domain — image and video generation agent tools backed by
//! OpenRouter through the OpenHuman backend's `/agent-integrations/openrouter`
//! proxy.
//!
//! The work is split by ownership:
//!
//! - **TinyInference** (`tinyinference-image` / `tinyinference-video`, reached
//!   through `tinyagents_harness`) owns the wire contract, reference and
//!   output-shape standards, the submit → poll → download job loop, and the
//!   rule that a billed call returns media or an error, never an empty success.
//! - **TinyAgents** (`tinyagents_harness::media`) owns the tools: argument
//!   parsing, artifact persistence into the workspace, and result wording.
//! - **This module** owns the host policy: endpoint, credential, egress,
//!   privacy and budget gates ([`provider`]), plus tool names, descriptions and
//!   the local-reference policy ([`tools`]).

mod artifact_tool;
mod progress;
pub mod provider;
pub mod tools;

pub use artifact_tool::MediaArtifactTool;
pub use provider::{managed_generators, MediaGenerators, OPENROUTER_PROXY_PATH};
pub(crate) use tools::build_media_tools_with_allowlist;
pub use tools::{
    build_media_tools, media_tools_from, MediaListModelsTool, IMAGE_TOOL_NAME,
    LIST_MODELS_TOOL_NAME, VIDEO_TOOL_NAME,
};
