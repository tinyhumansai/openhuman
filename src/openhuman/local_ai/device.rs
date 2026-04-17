//! Device profile detection for guided model selection.

use serde::{Deserialize, Serialize};
use sysinfo::System;

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

/// Probe the current machine and return a [`DeviceProfile`].
///
/// GPU detection is best-effort: Apple Silicon is assumed to have a GPU (Metal);
/// on other platforms we report "unknown" unless more specific probing is added later.
pub fn detect_device_profile() -> DeviceProfile {
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

    // Intel Mac: macOS with Intel CPU — no Metal GPU acceleration for whisper.
    if os_lower.contains("mac") {
        tracing::debug!("Intel Mac detected — no GPU acceleration available for whisper");
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
    let output = std::process::Command::new("nvidia-smi")
        .args(["--query-gpu=name", "--format=csv,noheader"])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;

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
mod tests {
    use super::*;

    #[test]
    fn detect_device_profile_returns_nonzero_hardware() {
        let profile = detect_device_profile();
        assert!(profile.total_ram_bytes > 0, "RAM should be > 0");
        assert!(profile.cpu_count > 0, "CPU count should be > 0");
        assert!(!profile.os_name.is_empty(), "OS name should be non-empty");
    }

    #[test]
    fn total_ram_gb_rounds_down() {
        let profile = DeviceProfile {
            total_ram_bytes: 17_179_869_184, // 16 GiB exactly
            cpu_count: 8,
            cpu_brand: "test".to_string(),
            os_name: "test".to_string(),
            os_version: "1.0".to_string(),
            has_gpu: false,
            gpu_description: None,
        };
        assert_eq!(profile.total_ram_gb(), 16);
    }

    #[test]
    fn total_ram_gb_reports_zero_for_sub_gb_systems() {
        let profile = DeviceProfile {
            total_ram_bytes: 500_000_000,
            cpu_count: 1,
            cpu_brand: "x".into(),
            os_name: "x".into(),
            os_version: "1".into(),
            has_gpu: false,
            gpu_description: None,
        };
        assert_eq!(profile.total_ram_gb(), 0);
    }

    #[test]
    fn total_ram_gb_truncates_partial_gigabyte() {
        // 1 GiB + 512 MiB should round down to 1 GiB.
        let profile = DeviceProfile {
            total_ram_bytes: (1024 * 1024 * 1024) + (512 * 1024 * 1024),
            cpu_count: 1,
            cpu_brand: "x".into(),
            os_name: "x".into(),
            os_version: "1".into(),
            has_gpu: false,
            gpu_description: None,
        };
        assert_eq!(profile.total_ram_gb(), 1);
    }

    #[test]
    fn detect_gpu_reports_apple_silicon_from_brand() {
        let (has, desc) = detect_gpu("Apple M2 Pro", "Darwin");
        assert!(has);
        assert_eq!(desc.as_deref(), Some("Apple Silicon (Metal)"));
    }

    #[test]
    fn detect_gpu_reports_apple_silicon_from_arm_on_mac() {
        // macOS + ARM CPU but brand lacks the literal "apple" string —
        // the arm+mac heuristic must still flag this as Apple Silicon.
        let (has, desc) = detect_gpu("arm based", "macOS");
        assert!(has);
        assert_eq!(desc.as_deref(), Some("Apple Silicon (Metal)"));
    }

    #[test]
    fn detect_gpu_reports_no_gpu_on_intel_mac() {
        let (has, desc) = detect_gpu("Intel Core i7", "macOS");
        assert!(!has);
        assert_eq!(desc.as_deref(), Some("Intel Mac (no Metal GPU)"));
    }

    #[test]
    fn detect_gpu_no_gpu_on_linux_without_nvidia() {
        // Linux without nvidia-smi should report no GPU (or NVIDIA if nvidia-smi is present).
        // Since we can't mock nvidia-smi here, we at least verify the function doesn't panic.
        let (has, desc) = detect_gpu("AMD Ryzen 9", "Linux");
        // On CI/dev machines without nvidia-smi, this should be (false, None).
        // If nvidia-smi is present, it returns (true, Some("NVIDIA ...")), which is also fine.
        if !has {
            assert!(desc.is_none());
        }
    }

    #[test]
    fn detect_gpu_windows_without_nvidia() {
        let (has, desc) = detect_gpu("Intel Core i9", "Windows");
        // Same as Linux: depends on nvidia-smi availability
        if !has {
            assert!(desc.is_none());
        }
    }

    #[test]
    fn total_ram_gb_exact_boundary() {
        let profile = DeviceProfile {
            total_ram_bytes: 1024 * 1024 * 1024, // exactly 1 GiB
            cpu_count: 1,
            cpu_brand: "x".into(),
            os_name: "x".into(),
            os_version: "1".into(),
            has_gpu: false,
            gpu_description: None,
        };
        assert_eq!(profile.total_ram_gb(), 1);
    }

    #[test]
    fn total_ram_gb_zero_bytes() {
        let profile = DeviceProfile {
            total_ram_bytes: 0,
            cpu_count: 1,
            cpu_brand: "x".into(),
            os_name: "x".into(),
            os_version: "1".into(),
            has_gpu: false,
            gpu_description: None,
        };
        assert_eq!(profile.total_ram_gb(), 0);
    }

    #[test]
    fn device_profile_serde_round_trip() {
        let original = DeviceProfile {
            total_ram_bytes: 8 * 1024 * 1024 * 1024,
            cpu_count: 4,
            cpu_brand: "CPU".into(),
            os_name: "OS".into(),
            os_version: "1.2.3".into(),
            has_gpu: true,
            gpu_description: Some("GPU".into()),
        };
        let s = serde_json::to_string(&original).unwrap();
        let back: DeviceProfile = serde_json::from_str(&s).unwrap();
        assert_eq!(back.total_ram_bytes, original.total_ram_bytes);
        assert_eq!(back.cpu_count, original.cpu_count);
        assert_eq!(back.has_gpu, original.has_gpu);
        assert_eq!(back.gpu_description, original.gpu_description);
    }
}
