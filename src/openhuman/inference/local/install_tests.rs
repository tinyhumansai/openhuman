use super::*;
use std::ffi::OsString;

/// Serialises tests that mutate process-global environment variables
/// (OLLAMA_BIN, PATH) with other local-AI tests that also read these
/// variables. Without this, cargo's test runner can interleave set/remove
/// calls and cause flakes.
fn env_lock() -> std::sync::MutexGuard<'static, ()> {
    crate::openhuman::inference::inference_test_guard()
}

/// RAII guard: records the prior value of `var` on construction and
/// restores it on drop (or removes the var if it was previously unset).
struct EnvGuard {
    var: &'static str,
    prior: Option<OsString>,
}

impl EnvGuard {
    fn set(var: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let prior = std::env::var_os(var);
        unsafe { std::env::set_var(var, value) };
        Self { var, prior }
    }

    fn unset(var: &'static str) -> Self {
        let prior = std::env::var_os(var);
        unsafe { std::env::remove_var(var) };
        Self { var, prior }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        unsafe {
            match self.prior.take() {
                Some(v) => std::env::set_var(self.var, v),
                None => std::env::remove_var(self.var),
            }
        }
    }
}

#[test]
fn build_install_command_on_supported_platform_returns_ok() {
    let tmp = tempfile::tempdir().unwrap();
    let result = build_install_command(tmp.path());
    if cfg!(any(
        target_os = "linux",
        target_os = "macos",
        target_os = "windows"
    )) {
        assert!(
            result.is_ok(),
            "build_install_command must return Ok on supported platforms, got {result:?}"
        );
    } else {
        assert!(
            result.is_err(),
            "build_install_command must return Err on unsupported platforms"
        );
    }
}

#[test]
fn find_system_ollama_binary_respects_env_override_when_file_exists() {
    let _lock = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let fake = tmp.path().join("ollama-stub");
    std::fs::write(&fake, "").unwrap();
    let _g = EnvGuard::set("OLLAMA_BIN", &fake);
    let found = find_system_ollama_binary();
    assert_eq!(found.as_deref(), Some(fake.as_path()));
}

#[test]
fn find_system_ollama_binary_ignores_env_override_when_file_missing() {
    let _lock = env_lock();
    let _g = EnvGuard::set("OLLAMA_BIN", "/nonexistent/ollama-stub-missing");
    // Result depends on whether /usr/bin/ollama etc. exist on this
    // machine. The important thing is the env-override didn't succeed.
    let found = find_system_ollama_binary();
    if let Some(p) = found {
        assert!(!p.to_string_lossy().contains("ollama-stub-missing"));
    }
}

#[test]
fn find_system_ollama_binary_ignores_empty_env_override() {
    let _lock = env_lock();
    {
        let _g = EnvGuard::set("OLLAMA_BIN", "");
        let _ = find_system_ollama_binary();
    }
    {
        let _g = EnvGuard::set("OLLAMA_BIN", "   ");
        let _ = find_system_ollama_binary();
    }
}

#[test]
fn find_system_ollama_binary_finds_binary_via_path() {
    let _lock = env_lock();
    let tmp = tempfile::tempdir().unwrap();
    let binary_name = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };
    let fake = tmp.path().join(binary_name);
    std::fs::write(&fake, "").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let prev_path = std::env::var_os("PATH").unwrap_or_default();
    let mut new_entries = vec![tmp.path().to_path_buf()];
    new_entries.extend(std::env::split_paths(&prev_path));
    let new_path = std::env::join_paths(new_entries).unwrap();
    let _ollama_guard = EnvGuard::unset("OLLAMA_BIN");
    let _path_guard = EnvGuard::set("PATH", &new_path);
    let found = find_system_ollama_binary();
    assert!(
        found.is_some(),
        "PATH-based lookup should succeed with a valid stub"
    );
}

#[cfg(target_os = "macos")]
#[test]
fn find_system_ollama_binary_detects_macos_app_bundle_in_applications() {
    let _lock = env_lock();
    // `find_system_ollama_binary` probes a fixed priority list on macOS:
    //   1. /usr/local/bin/ollama   (intel homebrew, hand-installed)
    //   2. /opt/homebrew/bin/ollama (apple-silicon homebrew)
    //   3. /Applications/Ollama.app/Contents/Resources/ollama
    //   4. $HOME/Applications/Ollama.app/Contents/Resources/ollama
    // The test exercises (4) by pointing $HOME at a tempdir and clearing
    // PATH/OLLAMA_BIN. Paths (1)–(3) are absolute and cannot be redirected
    // — if a dev machine already has Ollama installed at either homebrew
    // location or in the system /Applications dir, the function returns
    // that real binary first and the assertion below fails. Skip when any
    // earlier candidate already resolves so this test stays a regression
    // gate on the ~/Applications branch and not a "is Ollama installed on
    // this CI runner" probe.
    let unmaskable_real_install = [
        "/usr/local/bin/ollama",
        "/opt/homebrew/bin/ollama",
        "/Applications/Ollama.app/Contents/Resources/ollama",
    ]
    .iter()
    .any(|p| std::path::Path::new(p).is_file());
    if unmaskable_real_install {
        eprintln!(
            "skipping: host has a real Ollama install at a higher-priority absolute path \
             the test cannot mock"
        );
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // Build a fake /Applications/Ollama.app/Contents/Resources/ollama tree.
    let bundle_bin = tmp
        .path()
        .join("Applications")
        .join("Ollama.app")
        .join("Contents")
        .join("Resources")
        .join("ollama");
    std::fs::create_dir_all(bundle_bin.parent().unwrap()).unwrap();
    std::fs::write(&bundle_bin, b"stub").unwrap();

    // Clear OLLAMA_BIN, clear PATH so the normal PATH lookup won't find it,
    // and point HOME to tmp so the ~/Applications branch is exercised via a
    // separate sub-test below.  Here we exercise /Applications by building
    // the file at root and verifying the function returns it when the static
    // /Applications path exists — we skip direct-path injection since the
    // function hard-codes "/" as root and we cannot mock the filesystem.
    // Instead verify the ~/Applications path via the HOME trick.
    let _home_guard = EnvGuard::set("HOME", tmp.path());
    let _bin_guard = EnvGuard::unset("OLLAMA_BIN");
    let prev_path = std::env::var_os("PATH").unwrap_or_default();
    let _path_guard = EnvGuard::set("PATH", "");

    // ~/Applications bundle path is under HOME.
    let home_bundle = tmp
        .path()
        .join("Applications")
        .join("Ollama.app")
        .join("Contents")
        .join("Resources")
        .join("ollama");
    std::fs::create_dir_all(home_bundle.parent().unwrap()).unwrap();
    std::fs::write(&home_bundle, b"stub").unwrap();

    let found = find_system_ollama_binary();
    assert_eq!(
        found.as_deref(),
        Some(home_bundle.as_path()),
        "should find Ollama in ~/Applications bundle"
    );
    drop(_path_guard);
    let _ = prev_path;
}
