//! Arduino upload tool — agent generates code, uploads via arduino-cli.
//!
//! When user says "make a heart on the LED grid", the agent generates Arduino
//! sketch code and calls this tool. OpenHuman compiles and uploads it — no
//! manual IDE or file editing.

use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::process::Command;

/// Tool: upload Arduino sketch (agent-generated code) to the board.
pub struct ArduinoUploadTool {
    /// Serial port path (e.g. /dev/cu.usbmodem33000283452)
    pub port: String,
}

impl ArduinoUploadTool {
    pub fn new(port: String) -> Self {
        Self { port }
    }
}

#[async_trait]
impl Tool for ArduinoUploadTool {
    fn name(&self) -> &str {
        "arduino_upload"
    }

    fn description(&self) -> &str {
        "Generate Arduino sketch code and upload it to the connected Arduino. Use when: user asks to 'make a heart', 'blink LED', or run any custom pattern on Arduino. You MUST write the full .ino sketch code (setup + loop). Arduino Uno: pin 13 = built-in LED. Saves to temp dir, runs arduino-cli compile and upload. Requires arduino-cli installed."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "code": {
                    "type": "string",
                    "description": "Full Arduino sketch code (complete .ino file content)"
                }
            },
            "required": ["code"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let code = args
            .get("code")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'code' parameter"))?;

        if code.trim().is_empty() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Code cannot be empty".into()),
            });
        }

        // Check arduino-cli exists
        if Command::new("arduino-cli").arg("version").output().is_err() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(
                    "arduino-cli not found. Install it: https://arduino.github.io/arduino-cli/"
                        .into(),
                ),
            });
        }

        let sketch_name = "openhuman_sketch";
        let temp_dir = std::env::temp_dir().join(format!("openhuman_{}", uuid::Uuid::new_v4()));
        let sketch_dir = temp_dir.join(sketch_name);
        let ino_path = sketch_dir.join(format!("{}.ino", sketch_name));

        if let Err(e) = tokio::fs::create_dir_all(&sketch_dir).await {
            return Ok(ToolResult {
                success: false,
                output: format!("Failed to create sketch dir: {}", e),
                error: Some(e.to_string()),
            });
        }

        if let Err(e) = tokio::fs::write(&ino_path, code).await {
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Ok(ToolResult {
                success: false,
                output: format!("Failed to write sketch: {}", e),
                error: Some(e.to_string()),
            });
        }

        let sketch_path = sketch_dir.to_string_lossy();
        let fqbn = "arduino:avr:uno";

        // Compile
        let compile = Command::new("arduino-cli")
            .args(["compile", "--fqbn", fqbn, &sketch_path])
            .output();

        let compile_output = match compile {
            Ok(o) => o,
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Ok(ToolResult {
                    success: false,
                    output: format!("arduino-cli compile failed: {}", e),
                    error: Some(e.to_string()),
                });
            }
        };

        if !compile_output.status.success() {
            let stderr = String::from_utf8_lossy(&compile_output.stderr);
            let _ = tokio::fs::remove_dir_all(&temp_dir).await;
            return Ok(ToolResult {
                success: false,
                output: format!("Compile failed:\n{}", stderr),
                error: Some("Arduino compile error".into()),
            });
        }

        // Upload
        let upload = Command::new("arduino-cli")
            .args(["upload", "-p", &self.port, "--fqbn", fqbn, &sketch_path])
            .output();

        let upload_output = match upload {
            Ok(o) => o,
            Err(e) => {
                let _ = tokio::fs::remove_dir_all(&temp_dir).await;
                return Ok(ToolResult {
                    success: false,
                    output: format!("arduino-cli upload failed: {}", e),
                    error: Some(e.to_string()),
                });
            }
        };

        let _ = tokio::fs::remove_dir_all(&temp_dir).await;

        if !upload_output.status.success() {
            let stderr = String::from_utf8_lossy(&upload_output.stderr);
            return Ok(ToolResult {
                success: false,
                output: format!("Upload failed:\n{}", stderr),
                error: Some("Arduino upload error".into()),
            });
        }

        Ok(ToolResult {
            success: true,
            output:
                "Sketch compiled and uploaded successfully. The Arduino is now running your code."
                    .into(),
            error: None,
        })
    }
}
