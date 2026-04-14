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
//! | Medium        | yes           | local/remote (hint-driven) |
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

pub mod factory;
pub mod health;
pub mod policy;
pub mod provider;
pub mod quality;
pub mod telemetry;

pub use factory::new_provider;
pub use health::LocalHealthChecker;
pub use policy::{classify, decide, RoutingTarget, TaskCategory};
pub use provider::IntelligentRoutingProvider;
pub use quality::is_low_quality;
pub use telemetry::{emit as emit_routing_record, RoutingRecord};
