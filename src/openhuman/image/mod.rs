//! Image-tool contracts for model-facing agents.
//!
//! This module is intentionally a high-level contract layer. OpenHuman already
//! has lower-level image helpers (`image_info`, browser screenshots, and
//! multimodal `[IMAGE:...]` normalization). The image layer defines the
//! stable tool names, schema, gating, and prompt guidance that agents should see
//! when a runtime can provide Codex-like media tools.
//!
//! The two first-class contracts are:
//!
//! - [`image_generation`] — create or edit raster images and return stored file
//!   references.
//! - [`image_view`] — attach a local image file as model-visible image content
//!   so the agent can inspect it.
//!
//! Keeping this contract separate from execution lets provider runtimes adopt
//! the surface incrementally without duplicating business logic in the tools
//! registry.

pub mod image_generation;
pub mod image_view;
pub mod prompt;
pub mod types;

pub use image_generation::{
    image_generation_spec, ImageGenerationOutputFormat, IMAGE_GENERATION_TOOL_NAME,
};
pub use image_view::{image_view_spec, ImageDetail, IMAGE_VIEW_TOOL_NAME};
pub use prompt::{render_image_prompt_guidance, ImagePromptOptions};
pub use types::{
    image_specs, is_image_tool_gated, ImagePermission, ImageToolConfig, ImageToolSpec,
    IMAGE_TOOL_NAMES,
};

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
