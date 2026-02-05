use std::env;

fn main() {
    setup_tdlib();
    tauri_build::build();
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn setup_tdlib() {
    // download-tdlib: downloads prebuilt TDLib and configures linker flags
    tdlib_rs::build::build(None);

    // On macOS, replace the bundled dylib with our source-built version
    // that targets macOS 10.15 (the prebuilt one targets macOS 14.0)
    #[cfg(target_os = "macos")]
    copy_local_tdlib_for_bundle();
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn setup_tdlib() {
    // No TDLib on mobile
}

/// On macOS, copy the source-built TDLib dylib (with 10.15 deployment target)
/// to libraries/ for Tauri bundler. Lookup order:
///   1. tdlib-prebuilt/macos-<arch>/  (committed to git, no build needed)
///   2. tdlib-local/lib/              (local build via build-tdlib-from-source.sh)
///   3. download-tdlib output         (fallback, targets macOS 14.0+)
#[cfg(target_os = "macos")]
fn copy_local_tdlib_for_bundle() {
    use std::path::PathBuf;

    let manifest_dir =
        PathBuf::from(env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let libraries_dir = manifest_dir.join("libraries");

    // Use CARGO_CFG_TARGET_ARCH to handle cross-compilation (e.g. arm64 host → x86_64 target)
    let target_arch = env::var("CARGO_CFG_TARGET_ARCH").unwrap_or_else(|_| "aarch64".into());
    let arch_name = match target_arch.as_str() {
        "aarch64" => "arm64",
        other => other,
    };

    let tdlib_version = "1.8.29";
    let dylib_name = format!("libtdjson.{tdlib_version}.dylib");

    // 1. Check tdlib-prebuilt/ (committed to git)
    let prebuilt = manifest_dir
        .join("tdlib-prebuilt")
        .join(format!("macos-{arch_name}"))
        .join(&dylib_name);

    // 2. Check tdlib-local/<arch>/ (local source build)
    let local = manifest_dir
        .join("tdlib-local")
        .join(arch_name)
        .join("lib")
        .join(&dylib_name);

    let src_dylib = if prebuilt.exists() {
        println!("cargo:warning=Using prebuilt TDLib from {}", prebuilt.display());
        prebuilt
    } else if local.exists() {
        println!("cargo:warning=Using locally-built TDLib from {}", local.display());
        local
    } else {
        // 3. Fall back to the download-tdlib output
        println!(
            "cargo:warning=No source-built TDLib found for macos-{arch_name}. \
             The prebuilt TDLib will be bundled instead (targets macOS 14.0+). \
             Run: cd src-tauri && ./scripts/build-tdlib-from-source.sh"
        );
        let out_dir = env::var("OUT_DIR").unwrap();
        PathBuf::from(&out_dir).join("tdlib").join("lib").join(&dylib_name)
    };

    if !src_dylib.exists() {
        println!("cargo:warning=TDLib dylib not found at {}", src_dylib.display());
        return;
    }

    let dst_dylib = libraries_dir.join(&dylib_name);
    std::fs::create_dir_all(&libraries_dir).expect("Failed to create libraries/");
    std::fs::copy(&src_dylib, &dst_dylib).expect("Failed to copy TDLib dylib to libraries/");
    set_permissions_rw(&dst_dylib);
    fix_install_name(&dst_dylib, &dylib_name);

    // Add rpath so the binary finds the dylib in Contents/Frameworks/
    println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
}

#[cfg(target_os = "macos")]
fn fix_install_name(dylib_path: &std::path::Path, dylib_name: &str) {
    run_cmd(
        "install_name_tool",
        &[
            "-id",
            &format!("@rpath/{dylib_name}"),
            dylib_path.to_str().unwrap(),
        ],
    );
}

#[cfg(target_os = "macos")]
fn set_permissions_rw(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .expect("Failed to read metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("Failed to set permissions");
}

#[cfg(target_os = "macos")]
fn run_cmd(cmd: &str, args: &[&str]) {
    let status = std::process::Command::new(cmd)
        .args(args)
        .status()
        .unwrap_or_else(|e| panic!("Failed to run {cmd}: {e}"));
    if !status.success() {
        panic!("{cmd} failed with exit code {:?}", status.code());
    }
}
