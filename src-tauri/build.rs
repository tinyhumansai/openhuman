fn main() {
    // Get the target OS from environment variable (set by Cargo during cross-compilation)
    let target = std::env::var("TARGET").unwrap_or_default();
    let is_mobile_target = target.contains("android") || target.contains("ios");

    // TDLib build configuration (desktop only)
    // The tdlib-rs crate with download-tdlib feature handles downloading and linking
    // the prebuilt TDLib library automatically.
    // Note: We check the TARGET env var because cfg() checks the HOST platform for build scripts.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if !is_mobile_target {
        // Download and link TDLib library
        // Pass None to use default download location
        tdlib_rs::build::build(None);
    }

    tauri_build::build()
}
