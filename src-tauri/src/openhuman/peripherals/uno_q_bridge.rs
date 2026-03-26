//! Arduino Uno Q Bridge — GPIO via socket to Bridge app.
//!
//! When OpenHuman runs on Uno Q, the Bridge app (Python + MCU) exposes
//! digitalWrite/digitalRead over a local socket. These tools connect to it.

use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

const BRIDGE_HOST: &str = "127.0.0.1";
const BRIDGE_PORT: u16 = 9999;

async fn bridge_request(cmd: &str, args: &[String]) -> anyhow::Result<String> {
    let addr = format!("{}:{}", BRIDGE_HOST, BRIDGE_PORT);
    let mut stream = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(&addr))
        .await
        .map_err(|_| anyhow::anyhow!("Bridge connection timed out"))??;

    let msg = format!("{} {}\n", cmd, args.join(" "));
    stream.write_all(msg.as_bytes()).await?;

    let mut buf = vec![0u8; 64];
    let n = tokio::time::timeout(Duration::from_secs(3), stream.read(&mut buf))
        .await
        .map_err(|_| anyhow::anyhow!("Bridge response timed out"))??;
    let resp = String::from_utf8_lossy(&buf[..n]).trim().to_string();
    Ok(resp)
}

/// Tool: read GPIO pin via Uno Q Bridge.
pub struct UnoQGpioReadTool;

#[async_trait]
impl Tool for UnoQGpioReadTool {
    fn name(&self) -> &str {
        "gpio_read"
    }

    fn description(&self) -> &str {
        "Read GPIO pin value (0 or 1) on Arduino Uno Q. Requires openhuman-uno-q-bridge app running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pin": {
                    "type": "integer",
                    "description": "GPIO pin number (e.g. 13 for LED)"
                }
            },
            "required": ["pin"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let pin = args
            .get("pin")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("Missing 'pin' parameter"))?;
        match bridge_request("gpio_read", &[pin.to_string()]).await {
            Ok(resp) => {
                if resp.starts_with("error:") {
                    Ok(ToolResult {
                        success: false,
                        output: resp.clone(),
                        error: Some(resp),
                    })
                } else {
                    Ok(ToolResult {
                        success: true,
                        output: resp,
                        error: None,
                    })
                }
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Bridge error: {}", e),
                error: Some(e.to_string()),
            }),
        }
    }
}

/// Tool: write GPIO pin via Uno Q Bridge.
pub struct UnoQGpioWriteTool;

#[async_trait]
impl Tool for UnoQGpioWriteTool {
    fn name(&self) -> &str {
        "gpio_write"
    }

    fn description(&self) -> &str {
        "Set GPIO pin high (1) or low (0) on Arduino Uno Q. Requires openhuman-uno-q-bridge app running."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pin": {
                    "type": "integer",
                    "description": "GPIO pin number"
                },
                "value": {
                    "type": "integer",
                    "description": "0 for low, 1 for high"
                }
            },
            "required": ["pin", "value"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        let pin = args
            .get("pin")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("Missing 'pin' parameter"))?;
        let value = args
            .get("value")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| anyhow::anyhow!("Missing 'value' parameter"))?;
        match bridge_request("gpio_write", &[pin.to_string(), value.to_string()]).await {
            Ok(resp) => {
                if resp.starts_with("error:") {
                    Ok(ToolResult {
                        success: false,
                        output: resp.clone(),
                        error: Some(resp),
                    })
                } else {
                    Ok(ToolResult {
                        success: true,
                        output: "done".into(),
                        error: None,
                    })
                }
            }
            Err(e) => Ok(ToolResult {
                success: false,
                output: format!("Bridge error: {}", e),
                error: Some(e.to_string()),
            }),
        }
    }
}
