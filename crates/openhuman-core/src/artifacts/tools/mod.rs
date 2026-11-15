//! Artifact-producing agent tools: `generate_document` (`.docx`) and
//! `generate_presentation` (`.pptx`).
//!
//! Both build their output through [`super::production::Production`] and the
//! [`super::FileRoots`] files folder, so they live with the artifact domain
//! rather than in the generic `tools/` registry. The whole directory is behind
//! the `documents` feature (see `artifacts/mod.rs`); [`registered_tools`] builds
//! the family, `tools::ops` composes it, and `artifacts::ops` re-dispatches a presentation for Retry.

mod document;
mod document_engine;
mod document_types;
mod presentation;
mod presentation_engine;
mod presentation_images;
mod presentation_types;

pub use document::DocumentTool;
pub use presentation::PresentationTool;

use std::sync::Arc;

use crate::security::SecurityPolicy;

/// Build the artifact-generation tool family for this config, in catalogue
/// order: `generate_presentation` (#2778), then `generate_document` (#4847).
/// Both are native-Rust engines emitting through the same byte-agnostic
/// artifact pipeline, and both are always registered while `documents` is on.
pub(crate) fn registered_tools(
    config: &crate::config::Config,
    security: Arc<SecurityPolicy>,
) -> Vec<Box<dyn tinytools::Tool>> {
    vec![
        Box::new(PresentationTool::for_config(config, security.clone())),
        Box::new(DocumentTool::for_config(config, security)),
    ]
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
