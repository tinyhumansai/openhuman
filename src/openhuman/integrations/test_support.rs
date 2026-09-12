use axum::{
    extract::{Path, Query},
    routing::{get, post},
    Json, Router,
};
use parking_lot::Mutex;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub struct RecordedRequest {
    pub method: String,
    pub path: String,
    pub body: Value,
}

#[derive(Clone)]
struct FakeIntegrationState {
    requests: Arc<Mutex<Vec<RecordedRequest>>>,
}

pub struct FakeIntegrationBackend {
    pub base_url: String,
    state: FakeIntegrationState,
}

impl FakeIntegrationBackend {
    pub fn requests(&self) -> Vec<RecordedRequest> {
        self.state.requests.lock().clone()
    }
}

fn record(state: &FakeIntegrationState, method: &str, path: String, body: Value) {
    state.requests.lock().push(RecordedRequest {
        method: method.to_string(),
        path,
        body,
    });
}

fn as_string_array(value: &Value, key: &str) -> Vec<String> {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

include!("test_support_backend.rs");
