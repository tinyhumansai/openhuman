fn main() {
    // TDLib build configuration (desktop only)
    // The tdlib-rs crate with download-tdlib feature handles downloading and linking
    // the prebuilt TDLib library automatically.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    {
        // Download and link TDLib library
        // Pass None to use default download location
        tdlib_rs::build::build(None);
    }

    tauri_build::build()
}
