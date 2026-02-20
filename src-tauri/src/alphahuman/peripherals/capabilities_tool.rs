//! Hardware capabilities tool — Phase C: query device for reported GPIO pins.

use super::serial::SerialTransport;
use crate::alphahuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Tool: query device capabilities (GPIO pins, LED pin) from firmware.
pub struct HardwareCapabilitiesTool {
    /// (board_name, transport) for each serial board.
    boards: Vec<(String, Arc<SerialTransport>)>,
}

impl HardwareCapabilitiesTool {
    pub(crate) fn new(boards: Vec<(String, Arc<SerialTransport>)>) -> Self {
        Self { boards }
    }
}

#[async_trait]
impl Tool for HardwareCapabilitiesTool {
    fn name(&self) -> &str {
        "hardware_capabilities"
    }

    fn description(&self) -> &str {
        "Query connected hardware for reported GPIO pins and LED pin. Use when: user asks what pins are available."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "board": {
                    "type": "string",
                    "description": "Optional board name. If omitted, queries all."
                }
            }
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let filter = args.get("board").and_then(|v| v.as_str());
        let mut outputs = Vec::new();

        for (board_name, transport) in &self.boards {
            if let Some(b) = filter {
                if b != board_name {
                    continue;
                }
            }
            match transport.capabilities().await {
                Ok(result) => {
                    let output = if result.success {
                        if let Ok(parsed) =
                            serde_json::from_str::<serde_json::Value>(&result.output)
                        {
                            format!(
                                "{}: gpio {:?}, led_pin {:?}",
                                board_name,
                                parsed.get("gpio").unwrap_or(&json!([])),
                                parsed.get("led_pin").unwrap_or(&json!(null))
                            )
                        } else {
                            format!("{}: {}", board_name, result.output)
                        }
                    } else {
                        format!(
                            "{}: {}",
                            board_name,
                            result.error.as_deref().unwrap_or("unknown")
                        )
                    };
                    outputs.push(output);
                }
                Err(e) => {
                    outputs.push(format!("{}: error - {}", board_name, e));
                }
            }
        }

        let output = if outputs.is_empty() {
            if filter.is_some() {
                "No matching board or capabilities not supported.".to_string()
            } else {
                "No serial boards configured or capabilities not supported.".to_string()
            }
        } else {
            outputs.join("\n")
        };

        Ok(ToolResult {
            success: !outputs.is_empty(),
            output,
            error: None,
        })
    }
}
