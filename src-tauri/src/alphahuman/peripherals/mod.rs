//! Hardware peripherals — STM32, RPi GPIO, etc.
//!
//! Peripherals extend the agent with physical capabilities. See
//! `docs/hardware-peripherals-design.md` for the full design.

pub mod traits;

#[cfg(feature = "hardware")]
pub mod serial;

#[cfg(feature = "hardware")]
pub mod arduino_flash;
#[cfg(feature = "hardware")]
pub mod arduino_upload;
#[cfg(feature = "hardware")]
pub mod capabilities_tool;
#[cfg(feature = "hardware")]
pub mod nucleo_flash;
#[cfg(feature = "hardware")]
pub mod uno_q_bridge;
#[cfg(feature = "hardware")]
pub mod uno_q_setup;

#[cfg(all(feature = "peripheral-rpi", target_os = "linux"))]
pub mod rpi;

pub use traits::Peripheral;

use crate::alphahuman::config::{PeripheralBoardConfig, PeripheralsConfig};
#[cfg(feature = "hardware")]
use crate::alphahuman::tools::HardwareMemoryMapTool;
use crate::alphahuman::tools::Tool;
use anyhow::Result;

/// List configured boards from config (no connection yet).
pub fn list_configured_boards(config: &PeripheralsConfig) -> Vec<&PeripheralBoardConfig> {
    if !config.enabled {
        return Vec::new();
    }
    config.boards.iter().collect()
}

/// Create and connect peripherals from config, returning their tools.
/// Returns empty vec if peripherals disabled or hardware feature off.
#[cfg(feature = "hardware")]
pub async fn create_peripheral_tools(config: &PeripheralsConfig) -> Result<Vec<Box<dyn Tool>>> {
    if !config.enabled || config.boards.is_empty() {
        return Ok(Vec::new());
    }

    let mut tools: Vec<Box<dyn Tool>> = Vec::new();
    let mut serial_transports: Vec<(String, std::sync::Arc<serial::SerialTransport>)> = Vec::new();

    for board in &config.boards {
        // Arduino Uno Q: Bridge transport (socket to local Bridge app)
        if board.transport == "bridge" && (board.board == "arduino-uno-q" || board.board == "uno-q")
        {
            tools.push(Box::new(uno_q_bridge::UnoQGpioReadTool));
            tools.push(Box::new(uno_q_bridge::UnoQGpioWriteTool));
            tracing::info!(board = %board.board, "Uno Q Bridge GPIO tools added");
            continue;
        }

        // Native transport: RPi GPIO (Linux only)
        #[cfg(all(feature = "peripheral-rpi", target_os = "linux"))]
        if board.transport == "native"
            && (board.board == "rpi-gpio" || board.board == "raspberry-pi")
        {
            match rpi::RpiGpioPeripheral::connect_from_config(board).await {
                Ok(peripheral) => {
                    tools.extend(peripheral.tools());
                    tracing::info!(board = %board.board, "RPi GPIO peripheral connected");
                }
                Err(e) => {
                    tracing::warn!("Failed to connect RPi GPIO {}: {}", board.board, e);
                }
            }
            continue;
        }

        // Serial transport (STM32, ESP32, Arduino, etc.)
        if board.transport != "serial" {
            continue;
        }
        if board.path.is_none() {
            tracing::warn!("Skipping serial board {}: no path", board.board);
            continue;
        }

        match serial::SerialPeripheral::connect(board).await {
            Ok(peripheral) => {
                let mut p = peripheral;
                if p.connect().await.is_err() {
                    tracing::warn!("Peripheral {} connect warning (continuing)", p.name());
                }
                serial_transports.push((board.board.clone(), p.transport()));
                tools.extend(p.tools());
                if board.board == "arduino-uno" {
                    if let Some(ref path) = board.path {
                        tools.push(Box::new(arduino_upload::ArduinoUploadTool::new(
                            path.clone(),
                        )));
                        tracing::info!("Arduino upload tool added (port: {})", path);
                    }
                }
                tracing::info!(board = %board.board, "Serial peripheral connected");
            }
            Err(e) => {
                tracing::warn!("Failed to connect {}: {}", board.board, e);
            }
        }
    }

    // Phase B: Add hardware tools when any boards configured
    if !tools.is_empty() {
        let board_names: Vec<String> = config.boards.iter().map(|b| b.board.clone()).collect();
        tools.push(Box::new(HardwareMemoryMapTool::new(board_names.clone())));
        tools.push(Box::new(crate::alphahuman::tools::HardwareBoardInfoTool::new(
            board_names.clone(),
        )));
        tools.push(Box::new(crate::alphahuman::tools::HardwareMemoryReadTool::new(
            board_names,
        )));
    }

    // Phase C: Add hardware_capabilities tool when any serial boards
    if !serial_transports.is_empty() {
        tools.push(Box::new(capabilities_tool::HardwareCapabilitiesTool::new(
            serial_transports,
        )));
    }

    Ok(tools)
}

#[cfg(not(feature = "hardware"))]
pub async fn create_peripheral_tools(_config: &PeripheralsConfig) -> Result<Vec<Box<dyn Tool>>> {
    Ok(Vec::new())
}
