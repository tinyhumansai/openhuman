//! Built-in doctor checks for configuration and native domain readiness.

mod channels;
mod config_file;
mod config_provenance;
mod config_semantics;
mod embeddings;
mod embeddings_live;
mod ids;
mod inference;
pub(crate) mod inference_catalog;
mod inference_live;
mod integrations;
mod mcp;
mod memory;
mod memory_live;
mod modules;
mod registry;
mod runtime;
mod runtime_settings;
mod sandbox;
mod search;
mod secrets;
mod skills;
mod storage;
mod voice;
mod wallet;
mod workspace;

use std::sync::Arc;

use super::contract::DoctorCheck;

pub(super) use ids::canonical_segment;
pub(super) use registry::for_context;

pub(super) fn builtins() -> Vec<Arc<dyn DoctorCheck>> {
    let mut checks = config_file::checks();
    checks.push(Arc::new(config_provenance::EnvironmentOverrideCheck));
    checks.push(Arc::new(config_provenance::ConfigSourceCheck));
    checks.extend(runtime_settings::checks());
    checks.extend(workspace::checks());
    checks.extend(runtime::checks());
    checks.push(Arc::new(inference::ProviderRef));
    checks.push(Arc::new(inference::Construct));
    checks.push(Arc::new(inference::Temperature));
    checks.extend(memory::checks());
    checks.push(Arc::new(memory_live::MemoryRoundtripCheck));
    checks.extend(storage::checks());
    checks.push(Arc::new(secrets::KeyringCheck));
    checks.extend(embeddings::checks());
    checks.push(Arc::new(embeddings_live::EmbeddingsLiveCheck));
    checks.extend(config_semantics::checks());
    checks
}

#[cfg(test)]
#[path = "config_semantics_tests.rs"]
mod config_semantics_tests;

#[cfg(test)]
#[path = "inference_live_tests.rs"]
mod inference_live_tests;

#[cfg(test)]
#[path = "inference_live_budget_tests.rs"]
mod inference_live_budget_tests;

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
