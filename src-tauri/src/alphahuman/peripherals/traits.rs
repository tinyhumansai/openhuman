//! Peripheral trait — hardware boards (STM32, RPi GPIO) that expose tools.
//!
//! Peripherals are the agent's "arms and legs": remote devices that run minimal
//! firmware and expose capabilities (GPIO, sensors, actuators) as tools.

use async_trait::async_trait;

use crate::alphahuman::tools::Tool;

/// A hardware peripheral that exposes capabilities as tools.
///
/// Implement this for boards like Nucleo-F401RE (serial), RPi GPIO (native), etc.
/// When connected, the peripheral's tools are merged into the agent's tool registry.
#[async_trait]
pub trait Peripheral: Send + Sync {
    /// Human-readable peripheral name (e.g. "nucleo-f401re-0")
    fn name(&self) -> &str;

    /// Board type identifier (e.g. "nucleo-f401re", "rpi-gpio")
    fn board_type(&self) -> &str;

    /// Connect to the peripheral (open serial, init GPIO, etc.)
    async fn connect(&mut self) -> anyhow::Result<()>;

    /// Disconnect and release resources
    async fn disconnect(&mut self) -> anyhow::Result<()>;

    /// Check if the peripheral is reachable and responsive
    async fn health_check(&self) -> bool;

    /// Tools this peripheral provides (e.g. gpio_read, gpio_write, sensor_read)
    fn tools(&self) -> Vec<Box<dyn Tool>>;
}
