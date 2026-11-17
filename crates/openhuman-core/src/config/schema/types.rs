//! The persisted [`Config`] document, its model-tier constants, and the
//! resolvers layered on top of it.
//!
//! Load/save and env overrides extend `Config` in `load/`.

mod config;
mod config_clone;
mod defaults;
mod model_ids;
mod output_language;
mod resolvers;
mod time_zone;

pub(crate) use config::ConfigDocumentSnapshot;
pub use config::{Config, ConfigRuntimeState, CustomEmbeddingsConfig, ModelRegistryEntry};
pub use model_ids::WORKLOAD_ROLES;
pub use output_language::{normalize_output_language, output_language_directive};
pub use time_zone::{device_time_zone, normalize_time_zone};

#[cfg(test)]
use crate::config::schema::{CapabilityProviderTrustState, TeamModelConfig};

#[cfg(test)]
#[path = "types_model_pin_tests.rs"]
mod model_pin_tests;

#[cfg(test)]
#[path = "types_model_ids_tests.rs"]
mod model_ids_tests;
