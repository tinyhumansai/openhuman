//! Flash OpenHuman Nucleo-F401RE firmware via probe-rs.
//!
//! Builds the Embassy firmware and flashes via ST-Link (built into Nucleo).
//! Requires: cargo install probe-rs-tools --locked

use anyhow::{Context, Result};
use std::path::PathBuf;
use std::process::Command;

const CHIP: &str = "STM32F401RETx";
const TARGET: &str = "thumbv7em-none-eabihf";

/// Check if probe-rs UI is available (from probe-rs-tools).
pub fn probe_rs_available() -> bool {
    Command::new("probe-rs")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Flash OpenHuman Nucleo firmware. Builds from firmware/openhuman-nucleo.
pub fn flash_nucleo_firmware() -> Result<()> {
    if !probe_rs_available() {
        anyhow::bail!(
            "probe-rs not found. Install it:\n  cargo install probe-rs-tools --locked\n\n\
             Or: curl -LsSf https://github.com/probe-rs/probe-rs/releases/latest/download/probe-rs-tools-installer.sh | sh\n\n\
             Connect Nucleo via USB (ST-Link). Then run this command again."
        );
    }

    // CARGO_MANIFEST_DIR = repo root (openhuman's Cargo.toml)
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let firmware_dir = repo_root.join("firmware").join("openhuman-nucleo");
    if !firmware_dir.join("Cargo.toml").exists() {
        anyhow::bail!(
            "Nucleo firmware not found at {}. Run from openhuman repo root.",
            firmware_dir.display()
        );
    }

    println!("Building OpenHuman Nucleo firmware...");
    let build = Command::new("cargo")
        .args(["build", "--release", "--target", TARGET])
        .current_dir(&firmware_dir)
        .output()
        .context("cargo build failed")?;

    if !build.status.success() {
        let stderr = String::from_utf8_lossy(&build.stderr);
        anyhow::bail!("Build failed:\n{}", stderr);
    }

    let elf_path = firmware_dir
        .join("target")
        .join(TARGET)
        .join("release")
        .join("openhuman-nucleo");

    if !elf_path.exists() {
        anyhow::bail!("Built binary not found at {}", elf_path.display());
    }

    println!("Flashing to Nucleo-F401RE (connect via USB)...");
    let flash = Command::new("probe-rs")
        .args(["run", "--chip", CHIP, elf_path.to_str().unwrap()])
        .output()
        .context("probe-rs run failed")?;

    if !flash.status.success() {
        let stderr = String::from_utf8_lossy(&flash.stderr);
        anyhow::bail!(
            "Flash failed:\n{}\n\n\
             Ensure Nucleo is connected via USB. The ST-Link is built into the board.",
            stderr
        );
    }

    println!("OpenHuman Nucleo firmware flashed successfully.");
    println!("The Nucleo now supports: ping, capabilities, gpio_read, gpio_write.");
    println!("Add to config.toml: board = \"nucleo-f401re\", transport = \"serial\", path = \"/dev/ttyACM0\"");
    Ok(())
}
