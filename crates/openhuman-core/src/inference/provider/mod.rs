//! Native chat-model construction plus cloud/local inference policy and DTOs.
//!
//! This module was previously `providers/` (in the pre-consolidation,
//! single-crate layout). It now lives under `inference/provider/` so all
//! inference concerns (local runtime, cloud providers, HTTP endpoint) share
//! a single domain root.

pub mod factory;
pub(crate) mod fallback_diagnostics;
pub mod http_error;
pub mod models;
pub(crate) mod openai_codex;

pub use factory::{
    create_chat_model_from_string, create_chat_model_from_string_with_model_id,
    create_chat_model_in, create_chat_model_with_model_id_in, probe_inference_readiness,
    provider_for_role, BYOK_INCOMPLETE_SENTINEL,
};

#[cfg(test)]
#[path = "model_listing_test_support.rs"]
pub(crate) mod model_listing_test_support;
