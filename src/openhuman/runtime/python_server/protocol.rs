use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReadyLine {
    #[serde(default)]
    pub ready: bool,
    #[serde(default)]
    pub protocol: Option<u32>,
    #[serde(default)]
    pub backends: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonServerRequest {
    pub id: String,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonServerError {
    pub code: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PythonServerResponse {
    pub id: Option<String>,
    #[serde(default)]
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Value>,
    #[serde(default)]
    pub error: Option<PythonServerError>,
}

#[cfg(test)]
#[path = "protocol_tests.rs"]
mod tests;
