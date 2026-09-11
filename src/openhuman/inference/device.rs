//! Device profile detection for guided model selection.

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;
use sysinfo::System;

static DEVICE_PROFILE_CACHE: OnceLock<DeviceProfile> = OnceLock::new();

/// Summary of local hardware relevant for model tier selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeviceProfile {
    pub total_ram_bytes: u64,
    pub cpu_count: usize,
    pub cpu_brand: String,
    pub os_name: String,
    pub os_version: String,
    pub has_gpu: bool,
    pub gpu_description: Option<String>,
}

impl DeviceProfile {
    /// Total RAM expressed in whole gigabytes (rounded down).
    pub fn total_ram_gb(&self) -> u64 {
        self.total_ram_bytes / (1024 * 1024 * 1024)
    }
}

/// Return the detected [`DeviceProfile`] for the current machine.
///
/// The profile is probed once on first call and cached for the process lifetime.
/// Hardware changes during runtime are detected after the app restarts.
/// GPU detection is best-effort: Apple Silicon is assumed to have a GPU (Metal);
/// on other platforms we report "unknown" unless more specific probing is added later.
pub fn detect_device_profile() -> DeviceProfile {
    DEVICE_PROFILE_CACHE
        .get_or_init(detect_device_profile_uncached)
        .clone()
}

fn detect_device_profile_uncached() -> DeviceProfile {
    let mut sys = System::new_all();
    sys.refresh_all();

    let total_ram_bytes = sys.total_memory();
    let cpu_count = sys.cpus().len();
    let cpu_brand = sys
        .cpus()
        .first()
        .map(|c| c.brand().trim().to_string())
        .unwrap_or_default();

    let os_name = System::name().unwrap_or_else(|| "unknown".to_string());
    let os_version = System::os_version().unwrap_or_else(|| "unknown".to_string());

    let (has_gpu, gpu_description) = detect_gpu(&cpu_brand, &os_name);

    tracing::debug!(
        total_ram_bytes,
        cpu_count,
        cpu_brand = %cpu_brand,
        os_name = %os_name,
        os_version = %os_version,
        has_gpu,
        gpu_description = ?gpu_description,
        "device profile detected"
    );

    DeviceProfile {
        total_ram_bytes,
        cpu_count,
        cpu_brand,
        os_name,
        os_version,
        has_gpu,
        gpu_description,
    }
}

/// Best-effort GPU detection.
///
/// Apple Silicon always has a unified GPU (Metal). On Windows/Linux, we probe
/// for NVIDIA GPUs via `nvidia-smi`. On other systems we conservatively report
/// no GPU.
fn detect_gpu(cpu_brand: &str, os_name: &str) -> (bool, Option<String>) {
    let brand_lower = cpu_brand.to_ascii_lowercase();
    let os_lower = os_name.to_ascii_lowercase();

    // Apple Silicon detection: brand contains "apple" or we're on macOS with an ARM chip.
    if brand_lower.contains("apple") || (os_lower.contains("mac") && brand_lower.contains("arm")) {
        tracing::debug!("GPU detected: Apple Silicon (Metal)");
        return (true, Some("Apple Silicon (Metal)".to_string()));
    }

    // Intel Mac: macOS with Intel CPU — no Metal GPU acceleration.
    if os_lower.contains("mac") {
        tracing::debug!("Intel Mac detected — no GPU acceleration available");
        return (false, Some("Intel Mac (no Metal GPU)".to_string()));
    }

    // Windows / Linux: probe for NVIDIA GPU via nvidia-smi.
    if let Some(desc) = probe_nvidia_smi() {
        tracing::debug!("GPU detected via nvidia-smi: {desc}");
        return (true, Some(desc));
    }

    tracing::debug!("no GPU detected — voice model will use CPU");
    (false, None)
}

/// Probe for an NVIDIA GPU by running `nvidia-smi --query-gpu=name --format=csv,noheader`.
/// Returns `Some("NVIDIA <name> (CUDA)")` on success, `None` if nvidia-smi is not available.
fn probe_nvidia_smi() -> Option<String> {
    let mut cmd = std::process::Command::new("nvidia-smi");
    cmd.args(["--query-gpu=name", "--format=csv,noheader"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    let output = cmd.output().ok()?;

    if !output.status.success() {
        return None;
    }

    let name = String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()?
        .trim()
        .to_string();

    if name.is_empty() {
        return None;
    }

    Some(format!("NVIDIA {name} (CUDA)"))
}

#[cfg(test)]
#[path = "device_tests.rs"]
mod tests;
