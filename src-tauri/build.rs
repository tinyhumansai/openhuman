fn main() {
    // Get the target OS from environment variable (set by Cargo during cross-compilation)
    let target = std::env::var("TARGET").unwrap_or_default();
    let is_mobile_target = target.contains("android") || target.contains("ios");
    let is_macos_target = target.contains("apple") && !target.contains("ios");

    // TDLib build configuration (desktop only)
    // The tdlib-rs crate with download-tdlib feature handles downloading and linking
    // the prebuilt TDLib library automatically.
    // Note: We check the TARGET env var because cfg() checks the HOST platform for build scripts.
    #[cfg(not(any(target_os = "android", target_os = "ios")))]
    if !is_mobile_target {
        // Download and link TDLib library
        // Pass None to use default download location
        tdlib_rs::build::build(None);

        // On macOS, copy TDLib and its dependencies to libraries/ so that
        // tauri.conf.json > bundle > macOS > frameworks can bundle them into
        // the .app's Contents/Frameworks/ directory.
        if is_macos_target {
            prepare_macos_libraries();
        }
    }

    // Add @executable_path/../Frameworks to rpath so the binary can find
    // bundled dylibs at runtime (macOS only).
    if is_macos_target && !is_mobile_target {
        println!("cargo:rustc-link-arg=-Wl,-rpath,@executable_path/../Frameworks");
    }

    tauri_build::build()
}

/// Copy TDLib dylib and its non-system dependencies (OpenSSL) to src-tauri/libraries/
/// and rewrite their install names to use @rpath so they work inside Contents/Frameworks/.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn prepare_macos_libraries() {
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};

    let out_dir = std::env::var("OUT_DIR").expect("OUT_DIR not set");
    let tdlib_lib_dir = PathBuf::from(&out_dir).join("tdlib").join("lib");
    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set"));
    let dest_dir = manifest_dir.join("libraries");

    // Create libraries/ directory
    fs::create_dir_all(&dest_dir).expect("Failed to create libraries directory");

    let tdlib_dylib = "libtdjson.1.8.29.dylib";
    let tdlib_src = tdlib_lib_dir.join(tdlib_dylib);

    if !tdlib_src.exists() {
        println!(
            "cargo:warning=TDLib dylib not found at {}, skipping library preparation",
            tdlib_src.display()
        );
        return;
    }

    // Copy TDLib dylib
    let tdlib_dest = dest_dir.join(tdlib_dylib);
    fs::copy(&tdlib_src, &tdlib_dest).expect("Failed to copy TDLib dylib");
    make_writable(&tdlib_dest);

    // Fix TDLib's install name
    run_install_name_tool(&["-id", &format!("@rpath/{tdlib_dylib}"), tdlib_dest.to_str().unwrap()]);

    // Find and bundle non-system dependencies (e.g. OpenSSL from Homebrew)
    let deps = get_non_system_deps(&tdlib_dest);
    for dep_path in &deps {
        let dep_name = Path::new(dep_path)
            .file_name()
            .unwrap()
            .to_str()
            .unwrap();
        let dep_dest = dest_dir.join(dep_name);

        if Path::new(dep_path).exists() {
            fs::copy(dep_path, &dep_dest).unwrap_or_else(|e| panic!("Failed to copy {dep_name}: {e}"));
            make_writable(&dep_dest);

            // Fix the dependency's install name
            run_install_name_tool(&[
                "-id",
                &format!("@rpath/{dep_name}"),
                dep_dest.to_str().unwrap(),
            ]);

            // Update TDLib's reference to this dependency
            run_install_name_tool(&[
                "-change",
                dep_path,
                &format!("@rpath/{dep_name}"),
                tdlib_dest.to_str().unwrap(),
            ]);

            println!("cargo:warning=Bundled dependency: {dep_name}");
        } else {
            println!("cargo:warning=Dependency not found: {dep_path}");
        }
    }

    // Fix cross-references between dependencies (e.g. libssl -> libcrypto)
    let bundled_libs: Vec<PathBuf> = fs::read_dir(&dest_dir)
        .unwrap()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|ext| ext == "dylib"))
        .collect();

    for lib in &bundled_libs {
        let lib_deps = get_non_system_deps(lib);
        for dep_path in &lib_deps {
            let dep_name = Path::new(dep_path)
                .file_name()
                .unwrap()
                .to_str()
                .unwrap();
            // Only fix if the dep is one we bundled
            if dest_dir.join(dep_name).exists() {
                run_install_name_tool(&[
                    "-change",
                    dep_path,
                    &format!("@rpath/{dep_name}"),
                    lib.to_str().unwrap(),
                ]);
            }
        }
    }

    println!("cargo:warning=TDLib libraries prepared in libraries/");
}

/// Get non-system dependencies of a dylib using otool
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn get_non_system_deps(dylib_path: &std::path::Path) -> Vec<String> {
    use std::process::Command;

    let output = Command::new("otool")
        .args(["-L", dylib_path.to_str().unwrap()])
        .output()
        .expect("Failed to run otool");

    let stdout = String::from_utf8_lossy(&output.stdout);
    stdout
        .lines()
        .skip(1) // First line is the dylib itself
        .filter_map(|line| {
            let path = line.trim().split_whitespace().next()?;
            // Keep only non-system, non-rpath paths (e.g. Homebrew libs)
            if path.starts_with("/usr/lib/")
                || path.starts_with("/System/")
                || path.starts_with("@rpath/")
                || path.starts_with("@executable_path/")
            {
                None
            } else if path.starts_with("/opt/homebrew/") || path.starts_with("/usr/local/") {
                Some(path.to_string())
            } else {
                None
            }
        })
        .collect()
}

/// Make a file writable (Homebrew libs are read-only)
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn make_writable(path: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let mut perms = std::fs::metadata(path)
        .expect("Failed to read file metadata")
        .permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(path, perms).expect("Failed to set file permissions");
}

/// Run install_name_tool with the given arguments
#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn run_install_name_tool(args: &[&str]) {
    use std::process::Command;

    let status = Command::new("install_name_tool")
        .args(args)
        .status()
        .expect("Failed to run install_name_tool");

    if !status.success() {
        println!(
            "cargo:warning=install_name_tool failed with args: {}",
            args.join(" ")
        );
    }
}
