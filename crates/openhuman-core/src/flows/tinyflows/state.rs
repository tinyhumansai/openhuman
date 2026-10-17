//! A flow's namespaced state (`flow:<id>`), on whichever store this host uses.
//!
//! The engine reads and writes it through `StateStore` during a run, and the
//! dedup settlement (`flows::bus::dedup_commit`) reads it back through
//! `DedupKv` after the run, so both must name the same records. They are
//! `tinyflows_drivers::catalog::FlowStateDocuments` in the acting agent's scope
//! of the flow catalog's storage (`flows::store`). When the scope cannot be
//! resolved (SaaS mode with no acting agent) every call fails instead of
//! falling back to a shared bucket.

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tinyflows::caps::StateStore;
use tinyflows::error::{EngineError, Result as EngineResult};
use tinyflows::nodes::control_flow::dedup_settle::DedupKv;
use tinyflows_drivers::catalog::FlowStateDocuments;

use crate::config::Config;

/// One flow's state namespace on the store this call resolves to.
#[derive(Debug, Clone)]
pub enum FlowState {
    /// The flow catalog's storage.
    Documents(FlowStateDocuments),
    /// The storage scope could not be resolved; every call fails with this.
    Unavailable(String),
}

impl FlowState {
    /// The state of `namespace` for this call.
    pub fn open(config: &Config, namespace: impl Into<String>) -> Self {
        let namespace = namespace.into();
        match crate::flows::store::documents(config) {
            Ok(catalog) => Self::Documents(FlowStateDocuments::new(catalog, namespace)),
            Err(error) => {
                tracing::warn!(
                    target: "flows",
                    %error,
                    "[flows] flow state unavailable: the storage scope did not resolve"
                );
                Self::Unavailable(error.to_string())
            }
        }
    }

    /// As the engine's `StateStore` capability.
    pub fn into_state_store(self) -> Arc<dyn StateStore> {
        Arc::new(self)
    }

    fn unavailable(reason: &str) -> String {
        format!("flow state unavailable: {reason}")
    }
}

#[async_trait]
impl StateStore for FlowState {
    async fn load(&self, key: &str) -> EngineResult<Option<Value>> {
        match self {
            Self::Documents(store) => store.load(key).await,
            Self::Unavailable(reason) => Err(EngineError::Capability(Self::unavailable(reason))),
        }
    }

    async fn store(&self, key: &str, value: Value) -> EngineResult<()> {
        match self {
            Self::Documents(store) => store.store(key, value).await,
            Self::Unavailable(reason) => Err(EngineError::Capability(Self::unavailable(reason))),
        }
    }
}

impl DedupKv for FlowState {
    fn kv_get(&self, key: &str) -> std::result::Result<Option<Value>, String> {
        match self {
            Self::Documents(store) => store.kv_get(key),
            Self::Unavailable(reason) => Err(Self::unavailable(reason)),
        }
    }

    fn kv_set(&self, key: &str, value: &Value) -> std::result::Result<(), String> {
        match self {
            Self::Documents(store) => store.kv_set(key, value),
            Self::Unavailable(reason) => Err(Self::unavailable(reason)),
        }
    }

    fn kv_delete(&self, key: &str) -> std::result::Result<(), String> {
        match self {
            Self::Documents(store) => store.kv_delete(key),
            Self::Unavailable(reason) => Err(Self::unavailable(reason)),
        }
    }
}

#[cfg(test)]
#[path = "state_tests.rs"]
mod tests;
