//! Intelligent model routing — policy-driven selection between local and remote
//! inference backends.
//!
//! # Overview
//!
//! The routing layer sits between callers (agent harness, channels, tools) and
//! the concrete inference providers. It classifies each request by task
//! complexity, checks local model health, and forwards the request to the most
//! appropriate backend:
//!
//! | Task category | Local healthy | Target  |
//! |---------------|---------------|---------|
//! | Lightweight   | yes           | local   |
//! | Lightweight   | no            | remote  |
//! | Medium        | yes           | local   |
//! | Medium        | no            | remote  |
//! | Heavy         | either        | remote  |
//!
//! When a local call fails the request is transparently retried on the remote
//! backend and a structured telemetry event is emitted.
//!
//! # Quick start
//!
//! ```rust,ignore
//! use std::sync::Arc;
//! use crate::openhuman::routing;
//! use crate::openhuman::providers::create_backend_inference_provider;
//! use crate::openhuman::providers::compatible::{AuthStyle, OpenAiCompatibleProvider};
//!
//! let remote = create_backend_inference_provider(api_key, api_url, &opts)?;
//! let provider = routing::new_provider(remote, &config.local_ai, &config.default_model);
//! ```

pub mod health;
pub mod policy;
pub mod provider;
pub mod quality;
pub mod telemetry;

pub use health::LocalHealthChecker;
pub use policy::{classify, decide, RoutingTarget, TaskCategory};
pub use provider::IntelligentRoutingProvider;
pub use quality::is_low_quality;
pub use telemetry::{emit as emit_routing_record, RoutingRecord};

use std::sync::Arc;

use crate::openhuman::config::LocalAiConfig;
use crate::openhuman::local_ai::OLLAMA_BASE_URL;
use crate::openhuman::providers::compatible::{AuthStyle, OpenAiCompatibleProvider};
use crate::openhuman::providers::Provider;

/// Construct an [`IntelligentRoutingProvider`] from a remote backend provider
/// and the local AI configuration.
///
/// When `local_ai_config.enabled` is `false` the returned provider behaves
/// identically to the remote provider (local health always returns `false`).
///
/// `remote_fallback_model` is the model string sent to the remote backend when
/// a lightweight/medium task falls back from a failed local call. Typically
/// this is the configured `default_model` (e.g. `"reasoning-v1"`).
pub fn new_provider(
    remote: Box<dyn Provider>,
    local_ai_config: &LocalAiConfig,
    remote_fallback_model: &str,
) -> IntelligentRoutingProvider {
    let local_base = format!("{}/v1", OLLAMA_BASE_URL);
    let local: Box<dyn Provider> = Box::new(OpenAiCompatibleProvider::new(
        "ollama",
        &local_base,
        None, // Ollama does not require authentication
        AuthStyle::Bearer,
    ));

    let health = Arc::new(LocalHealthChecker::new(OLLAMA_BASE_URL));

    IntelligentRoutingProvider::new(
        remote,
        local,
        local_ai_config.chat_model_id.clone(),
        remote_fallback_model.to_string(),
        local_ai_config.enabled,
        health,
    )
}
