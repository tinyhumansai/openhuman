//! The `StateStore` capability, backed by the flows KV table.
//!
//! Namespaced per flow, so one flow's state cannot collide with another's.

#![allow(unused_imports)]

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::{json, Value};
use tinyflows::caps::*;
use tinyflows::error::{EngineError, Result};

use super::*;
use crate::openhuman::config::Config;
use crate::openhuman::flows;

/// [`StateStore`] adapter over the `flows::` domain's `flow_state` KV table.
pub struct FlowStateStore {
    pub config: Arc<Config>,
    pub namespace: String,
}

#[async_trait]
impl StateStore for FlowStateStore {
    async fn load(&self, key: &str) -> Result<Option<Value>> {
        let config = self.config.clone();
        let namespace = self.namespace.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || flows::kv_get(&config, &namespace, &key))
            .await
            .map_err(|e| EngineError::Capability(format!("flow state load task failed: {e}")))?
            .map_err(|e| EngineError::Capability(e.to_string()))
    }

    async fn store(&self, key: &str, value: Value) -> Result<()> {
        let config = self.config.clone();
        let namespace = self.namespace.clone();
        let key = key.to_string();
        tokio::task::spawn_blocking(move || flows::kv_set(&config, &namespace, &key, &value))
            .await
            .map_err(|e| EngineError::Capability(format!("flow state store task failed: {e}")))?
            .map_err(|e| EngineError::Capability(e.to_string()))
    }
}
