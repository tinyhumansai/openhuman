use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct BackendStatus {
    pub id: String,
    pub enabled: bool,
    pub ready: bool,
    pub message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct RuntimePythonServerStatus {
    pub enabled: bool,
    pub running: bool,
    pub backends: Vec<BackendStatus>,
    pub message: Option<String>,
}

impl RuntimePythonServerStatus {
    pub fn disabled(message: impl Into<String>) -> Self {
        Self {
            enabled: false,
            running: false,
            backends: Vec::new(),
            message: Some(message.into()),
        }
    }
}
