//! Flash OpenHuman Arduino firmware via arduino-cli.
//!
//! Ensures arduino-cli is available (installs via brew on macOS if missing),
//! installs the AVR core, compiles and uploads the base firmware.

use anyhow::{Context, Result};
use std::process::Command;

/// OpenHuman Arduino Uno base firmware (capabilities, gpio_read, gpio_write).
const FIRMWARE_INO: &str = include_str!("../../firmware/openhuman-arduino/openhuman-arduino.ino");

const FQBN: &str = "arduino:avr:uno";
const SKETCH_NAME: &str = "openhuman-arduino";

/// Check if arduino-cli is available.
pub fn arduino_cli_available() -> bool {
    Command::new("arduino-cli")
        .arg("version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Try to install arduino-cli. Returns Ok(()) if installed or already present.
pub fn ensure_arduino_cli() -> Result<()> {
    if arduino_cli_available() {
        return Ok(());
    }

    #[cfg(target_os = "macos")]
    {
        println!("arduino-cli not found. Installing via Homebrew...");
        let status = Command::new("brew")
            .args(["install", "arduino-cli"])
            .status()
            .context("Failed to run brew install")?;
        if !status.success() {
            anyhow::bail!("brew install arduino-cli failed. Install manually: https://arduino.github.io/arduino-cli/");
        }
        println!("arduino-cli installed.");
        if !arduino_cli_available() {
            anyhow::bail!("arduino-cli still not found after install. Ensure it's in PATH.");
        }
    }

    #[cfg(target_os = "linux")]
    {
        println!("arduino-cli not found. Run the install script:");
        println!("  curl -fsSL https://raw.githubusercontent.com/arduino/arduino-cli/master/install.sh | sh");
        println!();
        println!("Or install via package manager (e.g. apt install arduino-cli on Debian/Ubuntu).");
        anyhow::bail!("arduino-cli not installed. Install it and try again.");
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        println!("arduino-cli not found. Install it: https://arduino.github.io/arduino-cli/");
        anyhow::bail!("arduino-cli not installed.");
    }

    #[allow(unreachable_code)]
    Ok(())
}

/// Ensure arduino:avr core is installed.
fn ensure_avr_core() -> Result<()> {
    let out = Command::new("arduino-cli")
        .args(["core", "list"])
        .output()
        .context("arduino-cli core list failed")?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if stdout.contains("arduino:avr") {
        return Ok(());
    }

    println!("Installing Arduino AVR core...");
    let status = Command::new("arduino-cli")
        .args(["core", "install", "arduino:avr"])
        .status()
        .context("arduino-cli core install failed")?;
    if !status.success() {
        anyhow::bail!("Failed to install arduino:avr core");
    }
    println!("AVR core installed.");
    Ok(())
}

/// Flash OpenHuman firmware to Arduino at the given port.
pub fn flash_arduino_firmware(port: &str) -> Result<()> {
    ensure_arduino_cli()?;
    ensure_avr_core()?;

    let temp_dir = std::env::temp_dir().join(format!("openhuman_flash_{}", uuid::Uuid::new_v4()));
    let sketch_dir = temp_dir.join(SKETCH_NAME);
    let ino_path = sketch_dir.join(format!("{}.ino", SKETCH_NAME));

    std::fs::create_dir_all(&sketch_dir).context("Failed to create sketch dir")?;
    std::fs::write(&ino_path, FIRMWARE_INO).context("Failed to write firmware")?;

    let sketch_path = sketch_dir.to_string_lossy();

    // Compile
    println!("Compiling OpenHuman Arduino firmware...");
    let compile = Command::new("arduino-cli")
        .args(["compile", "--fqbn", FQBN, &*sketch_path])
        .output()
        .context("arduino-cli compile failed")?;

    if !compile.status.success() {
        let stderr = String::from_utf8_lossy(&compile.stderr);
        let _ = std::fs::remove_dir_all(&temp_dir);
        anyhow::bail!("Compile failed:\n{}", stderr);
    }

    // Upload
    println!("Uploading to {}...", port);
    let upload = Command::new("arduino-cli")
        .args(["upload", "-p", port, "--fqbn", FQBN, &*sketch_path])
        .output()
        .context("arduino-cli upload failed")?;

    let _ = std::fs::remove_dir_all(&temp_dir);

    if !upload.status.success() {
        let stderr = String::from_utf8_lossy(&upload.stderr);
        anyhow::bail!("Upload failed:\n{}\n\nEnsure the board is connected and the port is correct (e.g. /dev/cu.usbmodem* on macOS).", stderr);
    }

    println!("OpenHuman firmware flashed successfully.");
    println!("The Arduino now supports: capabilities, gpio_read, gpio_write.");
    Ok(())
}

/// Resolve port from config or path. Returns the path to use for flashing.
pub fn resolve_port(config: &crate::openhuman::config::Config, path_override: Option<&str>) -> Option<String> {
    if let Some(p) = path_override {
        return Some(p.to_string());
    }
    config
        .peripherals
        .boards
        .iter()
        .find(|b| b.board == "arduino-uno" && b.transport == "serial")
        .and_then(|b| b.path.clone())
}
