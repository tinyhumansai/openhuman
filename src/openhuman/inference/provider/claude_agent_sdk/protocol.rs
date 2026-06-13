//! Wire types for the `claude --output-format stream-json` NDJSON protocol.

use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SdkMessage {
    Text {
        text: String,
    },
    Result {
        result: Option<String>,
        #[serde(rename = "is_error")]
        is_error: bool,
        #[serde(default)]
        total_cost_usd: Option<f64>,
    },
    Error {
        error: SdkError,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Deserialize)]
pub struct SdkError {
    pub message: String,
}
