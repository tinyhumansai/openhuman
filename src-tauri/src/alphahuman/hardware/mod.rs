//! Hardware discovery and introspection.
//!
//! Feature-gated behind `hardware` and optionally `probe`.

pub mod registry;

#[cfg(feature = "hardware")]
pub mod discover;

#[cfg(feature = "hardware")]
pub mod introspect;

use anyhow::Result;
use serde::{Deserialize, Serialize};

// Re-export config types so UI flows can use `hardware::HardwareConfig` etc.
pub use crate::alphahuman::config::{HardwareConfig, HardwareTransport};

/// A hardware device discovered during auto-scan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscoveredDevice {
    pub name: String,
    pub detail: Option<String>,
    pub device_path: Option<String>,
    pub transport: HardwareTransport,
}

/// Introspection result for a specific device path.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HardwareIntrospect {
    pub path: String,
    pub vid: Option<u16>,
    pub pid: Option<u16>,
    pub board_name: Option<String>,
    pub architecture: Option<String>,
    pub memory_map_note: String,
}

/// Auto-discover connected hardware devices.
/// Returns an empty vec on platforms without hardware support.
pub fn discover_hardware() -> Vec<DiscoveredDevice> {
    #[cfg(feature = "hardware")]
    {
        if let Ok(devices) = discover::list_usb_devices() {
            return devices
                .into_iter()
                .map(|d| DiscoveredDevice {
                    name: d
                        .board_name
                        .unwrap_or_else(|| format!("{:04x}:{:04x}", d.vid, d.pid)),
                    detail: d.product_string,
                    device_path: None,
                    transport: if d.architecture.as_deref() == Some("native") {
                        HardwareTransport::Native
                    } else {
                        HardwareTransport::Serial
                    },
                })
                .collect();
        }
    }
    Vec::new()
}

/// Introspect a device by path (e.g. /dev/ttyACM0).
pub fn introspect_device(path: &str) -> Result<HardwareIntrospect> {
    #[cfg(feature = "hardware")]
    {
        let result = introspect::introspect_device(path)?;
        return Ok(HardwareIntrospect {
            path: result.path,
            vid: result.vid,
            pid: result.pid,
            board_name: result.board_name,
            architecture: result.architecture,
            memory_map_note: result.memory_map_note,
        });
    }

    #[cfg(not(feature = "hardware"))]
    {
        let _ = path;
        anyhow::bail!("Hardware introspection requires the 'hardware' feature");
    }
}

/// Return the recommended default choice index based on discovered devices.
/// 0 = Native, 1 = Tethered/Serial, 2 = Debug Probe, 3 = Software Only
pub fn recommended_default_choice(devices: &[DiscoveredDevice]) -> usize {
    if devices.is_empty() {
        3
    } else {
        1
    }
}

/// Build a `HardwareConfig` from a choice index (0-3) and discovered devices.
pub fn config_from_choice(choice: usize, devices: &[DiscoveredDevice]) -> HardwareConfig {
    match choice {
        0 => HardwareConfig {
            enabled: true,
            transport: HardwareTransport::Native,
            ..HardwareConfig::default()
        },
        1 => {
            let serial_port = devices
                .iter()
                .find(|d| d.transport == HardwareTransport::Serial)
                .and_then(|d| d.device_path.clone());
            HardwareConfig {
                enabled: true,
                transport: HardwareTransport::Serial,
                serial_port,
                ..HardwareConfig::default()
            }
        }
        2 => HardwareConfig {
            enabled: true,
            transport: HardwareTransport::Probe,
            ..HardwareConfig::default()
        },
        _ => HardwareConfig::default(),
    }
}
