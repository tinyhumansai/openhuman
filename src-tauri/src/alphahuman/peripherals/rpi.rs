//! Raspberry Pi GPIO peripheral — native rppal access.
//!
//! Only compiled when `peripheral-rpi` feature is enabled and target is Linux.
//! Uses BCM pin numbering (e.g. GPIO 17, 27).

use crate::alphahuman::config::PeripheralBoardConfig;
use crate::alphahuman::peripherals::traits::Peripheral;
use crate::alphahuman::tools::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::{json, Value};

/// RPi GPIO peripheral — direct access via rppal.
pub struct RpiGpioPeripheral {
    board: PeripheralBoardConfig,
}

impl RpiGpioPeripheral {
    /// Create a new RPi GPIO peripheral from config.
    pub fn new(board: PeripheralBoardConfig) -> Self {
        Self { board }
    }

    /// Attempt to connect (init rppal). Returns Ok if GPIO is available.
    pub async fn connect_from_config(board: &PeripheralBoardConfig) -> anyhow::Result<Self> {
        let mut peripheral = Self::new(board.clone());
        peripheral.connect().await?;
        Ok(peripheral)
    }
}

#[async_trait]
impl Peripheral for RpiGpioPeripheral {
    fn name(&self) -> &str {
        &self.board.board
    }

    fn board_type(&self) -> &str {
        "rpi-gpio"
    }

    async fn connect(&mut self) -> anyhow::Result<()> {
        // Verify GPIO is accessible by doing a no-op init
        let result = tokio::task::spawn_blocking(|| rppal::gpio::Gpio::new()).await??;
        drop(result);
        Ok(())
    }

    async fn disconnect(&mut self) -> anyhow::Result<()> {
        Ok(())
    }

    async fn health_check(&self) -> bool {
        tokio::task::spawn_blocking(|| rppal::gpio::Gpio::new().is_ok())
            .await
            .unwrap_or(false)
    }

    fn tools(&self) -> Vec<Box<dyn Tool>> {
        vec![Box::new(RpiGpioReadTool), Box::new(RpiGpioWriteTool)]
    }
}

/// Tool: read GPIO pin value (BCM numbering).
struct RpiGpioReadTool;

#[async_trait]
impl Tool for RpiGpioReadTool {
    fn name(&self) -> &str {
        "gpio_read"
    }

    fn description(&self) -> &str {
        "Read the value (0 or 1) of a GPIO pin on Raspberry Pi. Uses BCM pin numbers (e.g. 17, 27)."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pin": {
                    "type": "integer",
                    "description": "BCM GPIO pin number (e.g. 17, 27)"
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
        let pin_u8 = pin as u8;

        let value = tokio::task::spawn_blocking(move || {
            let gpio = rppal::gpio::Gpio::new()?;
            let pin = gpio.get(pin_u8)?.into_input();
            Ok::<_, anyhow::Error>(match pin.read() {
                rppal::gpio::Level::Low => 0,
                rppal::gpio::Level::High => 1,
            })
        })
        .await??;

        Ok(ToolResult {
            success: true,
            output: format!("pin {} = {}", pin, value),
            error: None,
        })
    }
}

/// Tool: write GPIO pin value (BCM numbering).
struct RpiGpioWriteTool;

#[async_trait]
impl Tool for RpiGpioWriteTool {
    fn name(&self) -> &str {
        "gpio_write"
    }

    fn description(&self) -> &str {
        "Set a GPIO pin high (1) or low (0) on Raspberry Pi. Uses BCM pin numbers."
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "pin": {
                    "type": "integer",
                    "description": "BCM GPIO pin number"
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
        let pin_u8 = pin as u8;
        let level = match value {
            0 => rppal::gpio::Level::Low,
            _ => rppal::gpio::Level::High,
        };

        tokio::task::spawn_blocking(move || {
            let gpio = rppal::gpio::Gpio::new()?;
            let mut pin = gpio.get(pin_u8)?.into_output();
            pin.write(level);
            Ok::<_, anyhow::Error>(())
        })
        .await??;

        Ok(ToolResult {
            success: true,
            output: format!("pin {} = {}", pin, value),
            error: None,
        })
    }
}
