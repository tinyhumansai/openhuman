//! USB device discovery — enumerate devices and enrich with board registry.

use super::registry;
use anyhow::Result;
use nusb::MaybeFuture;

/// Information about a discovered USB device.
#[derive(Debug, Clone)]
pub struct UsbDeviceInfo {
    pub bus_id: String,
    pub device_address: u8,
    pub vid: u16,
    pub pid: u16,
    pub product_string: Option<String>,
    pub board_name: Option<String>,
    pub architecture: Option<String>,
}

/// Enumerate all connected USB devices and enrich with board registry lookup.
#[cfg(feature = "hardware")]
pub fn list_usb_devices() -> Result<Vec<UsbDeviceInfo>> {
    let mut devices = Vec::new();

    let iter = nusb::list_devices()
        .wait()
        .map_err(|e| anyhow::anyhow!("USB enumeration failed: {e}"))?;

    for dev in iter {
        let vid = dev.vendor_id();
        let pid = dev.product_id();
        let board = registry::lookup_board(vid, pid);

        devices.push(UsbDeviceInfo {
            bus_id: dev.bus_id().to_string(),
            device_address: dev.device_address(),
            vid,
            pid,
            product_string: dev.product_string().map(String::from),
            board_name: board.map(|b| b.name.to_string()),
            architecture: board.and_then(|b| b.architecture.map(String::from)),
        });
    }

    Ok(devices)
}
