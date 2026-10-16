//! Managed media generation: OpenRouter image and video generators reached
//! through the TinyHumans backend's `/agent-integrations/openrouter` proxy.
//!
//! The core owns the `media_generate_*` tools and the [`MediaGenerators`] pair
//! they run over (`openhuman_embed::__host::media::generation`); this module
//! supplies the generators ([`provider`]) and registers a tool source that
//! builds the tools over them ([`register`]).
//!
//! [`MediaGenerators`]: openhuman_embed::__host::media::generation::MediaGenerators

pub mod provider;

use std::sync::Arc;

use openhuman_embed::__host::media::generation::media_tools_for_config;
use openhuman_embed::__host::tool_sources::{
    register_tool_source_at, ToolSourcePlacement, ANCHOR_AFTER_SEARCH,
};

pub use provider::{managed_generators, OPENROUTER_PROXY_PATH};

/// The id the media tool source registers under.
pub const TOOL_SOURCE_ID: &str = "tinyhumans-media";

/// Registers the media tool source with the core. Idempotent.
pub(crate) fn register() {
    register_tool_source_at(
        TOOL_SOURCE_ID,
        ToolSourcePlacement::After(ANCHOR_AFTER_SEARCH),
        |ctx| {
            let Some(generators) = managed_generators(ctx.root_config) else {
                log::debug!("[tinyhumans] media: no managed generators; media tools skipped");
                return Vec::new();
            };
            media_tools_for_config(
                generators,
                ctx.root_config,
                ctx.action_dir,
                Arc::clone(&ctx.security.media_artifact_paths),
            )
        },
    );
}
