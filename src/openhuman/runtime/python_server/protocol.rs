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
mod tests {
    use super::*;

    #[test]
    fn ready_line_parses() {
        let ready: ReadyLine =
            serde_json::from_str(r#"{"ready":true,"protocol":1,"backends":["spacy"]}"#).unwrap();
        assert!(ready.ready);
        assert_eq!(ready.protocol, Some(PROTOCOL_VERSION));
        assert_eq!(ready.backends, vec!["spacy"]);
    }

    #[test]
    fn response_parses_error_envelope() {
        let response: PythonServerResponse = serde_json::from_str(
            r#"{"id":"7","ok":false,"error":{"code":"bad_request","message":"missing text"}}"#,
        )
        .unwrap();
        assert!(!response.ok);
        assert_eq!(response.id.as_deref(), Some("7"));
        assert_eq!(response.error.unwrap().code, "bad_request");
    }
}
