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
