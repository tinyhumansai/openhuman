//! Automatic Ollama installer and system binary discovery.

use std::path::{Path, PathBuf};

/// Name of the Inno Setup installer process. On Windows the installer is
/// spawned via PowerShell's `Start-Process`, which creates a top-level
/// process — it survives the parent OpenHuman process dying. If OpenHuman
/// is killed mid-install (or the user closes the app and reopens it before
/// install completes) we need to detect the in-flight installer instead
/// of launching a second one that would race on the same install dir.
#[cfg(windows)]
const OLLAMA_INSTALLER_PROCESS_NAME: &str = "OllamaSetup.exe";

/// Returns `true` when a Windows OllamaSetup.exe process is currently
/// running anywhere on the machine. macOS / Linux installs are spawned
/// as `sh` children of the Rust core and cannot orphan past it; the
/// non-Windows branch is therefore a constant `false`.
#[cfg(windows)]
pub(crate) fn is_ollama_installer_running() -> bool {
    use sysinfo::{ProcessesToUpdate, System};
    let mut sys = System::new();
    // refresh_processes is sufficient — we only need the process list, not
    // CPU/memory/disk/network. refresh_all() adds dozens of milliseconds of
    // blocking I/O on a loaded Windows machine and this function runs on the
    // async executor inside download_and_install_ollama.
    // sysinfo 0.33 added a second `remove_dead_processes` parameter.
    sys.refresh_processes(ProcessesToUpdate::All, true);
    sys.processes().values().any(|p| {
        p.name()
            .to_string_lossy()
            .eq_ignore_ascii_case(OLLAMA_INSTALLER_PROCESS_NAME)
    })
}

#[cfg(not(windows))]
pub(crate) fn is_ollama_installer_running() -> bool {
    false
}

/// Captured output from the Ollama install script.
pub(crate) struct InstallResult {
    pub exit_status: std::process::ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

/// Run the platform-specific Ollama install into the workspace and capture stdout/stderr.
pub(crate) async fn run_ollama_install_script(install_dir: &Path) -> Result<InstallResult, String> {
    let mut cmd = build_install_command(install_dir)?;

    let output = cmd
        .output()
        .await
        .map_err(|e| format!("failed to execute Ollama installer: {e}"))?;

    log::debug!(
        "[local_ai] Ollama install script finished (dir={} exit={}) stdout={} stderr={}",
        install_dir.display(),
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
    );

    Ok(InstallResult {
        exit_status: output.status,
        stdout: String::from_utf8_lossy(&output.stdout).to_string(),
        stderr: String::from_utf8_lossy(&output.stderr).to_string(),
    })
}

#[cfg(target_os = "windows")]
pub(crate) fn resolve_powershell_executable() -> std::ffi::OsString {
    // `Command::new("powershell")` relies on PATH. When OpenHuman.exe is
    // spawned by `cargo tauri dev` (or similar dev harnesses) the inherited
    // PATH can be sanitized down to a subset that excludes the
    // `WindowsPowerShell\v1.0` dir, and the spawn fails with `program not
    // found`. Probe known absolute locations first and fall back to the
    // bare name if none are present.
    //
    // %SystemRoot% defaults to `C:\Windows` and is always set on Windows
    // sessions.
    let system_root =
        std::env::var_os("SystemRoot").unwrap_or_else(|| std::ffi::OsString::from("C:\\Windows"));
    for relative in [
        "System32\\WindowsPowerShell\\v1.0\\powershell.exe",
        "SysWOW64\\WindowsPowerShell\\v1.0\\powershell.exe",
    ] {
        let mut candidate = std::path::PathBuf::from(&system_root);
        candidate.push(relative);
        if candidate.is_file() {
            return candidate.into_os_string();
        }
    }
    // PowerShell 7 (`pwsh.exe`) is another viable substitute when present.
    if let Ok(pf) = std::env::var("ProgramFiles") {
        let p7 = std::path::PathBuf::from(pf)
            .join("PowerShell")
            .join("7")
            .join("pwsh.exe");
        if p7.is_file() {
            return p7.into_os_string();
        }
    }
    std::ffi::OsString::from("powershell")
}

fn build_install_command(install_dir: &Path) -> Result<tokio::process::Command, String> {
    #[cfg(target_os = "windows")]
    {
        let powershell_exe = resolve_powershell_executable();
        log::debug!(
            "[local_ai] resolved powershell for installer: {}",
            powershell_exe.to_string_lossy()
        );
        let mut cmd = tokio::process::Command::new(&powershell_exe);
        // Kill the PowerShell child if the spawning future is dropped — e.g.
        // the in-process tokio runtime shuts down because the user closed
        // OpenHuman mid-install. Without this, `cmd.output().await` keeps
        // OpenHuman.exe alive (and port 7788 bound) for the full 60–120s of
        // the install download, producing zombie processes and
        // "port in use" errors on the next launch. Note: OllamaSetup.exe is
        // spawned by PowerShell as a TOP-LEVEL process (via `Start-Process`),
        // so it survives PowerShell's death. That's intentional — the
        // crash-resume detection in `is_ollama_installer_running` picks it
        // up on the next OpenHuman launch and waits.
        cmd.kill_on_drop(true);
        crate::openhuman::local_ai::process_util::apply_no_window(&mut cmd);
        cmd.env("OPENHUMAN_OLLAMA_INSTALL_DIR", install_dir);
        cmd.args([
            "-NoProfile",
            "-ExecutionPolicy",
            "Bypass",
            "-Command",
            r#"
            $ErrorActionPreference = "Stop"
            $ProgressPreference = "SilentlyContinue"
            $installDir = $env:OPENHUMAN_OLLAMA_INSTALL_DIR
            New-Item -ItemType Directory -Path $installDir -Force | Out-Null
            $installerUrl = "https://ollama.com/download/OllamaSetup.exe"
            $tempInstaller = Join-Path $env:TEMP "OllamaSetup.exe"
            Invoke-WebRequest -UseBasicParsing -Uri $installerUrl -OutFile $tempInstaller
            # /SILENT (not /VERYSILENT) so Inno Setup's small progress dialog
            # appears. The dialog is owned by the OS, not OpenHuman, so it
            # survives the parent process crashing — giving the user a visible
            # signal that an install is in flight even if OpenHuman dies.
            $args = "/SILENT /NORESTART /SUPPRESSMSGBOXES /CURRENTUSER /DIR=""$installDir"""
            $proc = Start-Process -FilePath $tempInstaller -ArgumentList $args -PassThru
            $proc.WaitForExit()
            if ($proc.ExitCode -ne 0) {
                throw "Installation failed with exit code $($proc.ExitCode)"
            }
            Remove-Item $tempInstaller -Force -ErrorAction SilentlyContinue
            "#,
        ]);
        return Ok(cmd);
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = tokio::process::Command::new("sh");
        // Same rationale as the Windows branch: kill the child if the
        // spawning task is dropped (e.g. app shutdown mid-install) so the
        // tokio runtime can exit cleanly instead of waiting for curl to
        // finish downloading the full Ollama.app bundle.
        cmd.kill_on_drop(true);
        cmd.env("OPENHUMAN_OLLAMA_INSTALL_DIR", install_dir);
        cmd.arg("-lc")
            .arg(
                r#"
                set -eu
                for tool in curl unzip mktemp rm cp chmod mkdir; do
                  command -v "$tool" >/dev/null 2>&1 || { echo "missing required tool: $tool" >&2; exit 1; }
                done
                dest="$OPENHUMAN_OLLAMA_INSTALL_DIR"
                tmp_dir="$(mktemp -d)"
                cleanup() { rm -rf "$tmp_dir"; }
                trap cleanup EXIT
                archive="$tmp_dir/Ollama-darwin.zip"
                echo ">>> Downloading Ollama for macOS into $dest" >&2
                curl --fail --show-error --location --progress-bar -o "$archive" "https://ollama.com/download/Ollama-darwin.zip"
                unzip -q "$archive" -d "$tmp_dir"
                rm -rf "$dest"
                mkdir -p "$dest"
                cp -R "$tmp_dir/Ollama.app/Contents/Resources/." "$dest/"
                chmod 755 "$dest/ollama"
                "#,
            );
        return Ok(cmd);
    }

    #[cfg(target_os = "linux")]
    {
        let mut cmd = tokio::process::Command::new("sh");
        cmd.kill_on_drop(true); // see Windows-branch comment above
        cmd.env("OPENHUMAN_OLLAMA_INSTALL_DIR", install_dir);
        cmd.arg("-lc")
            .arg(
                r#"
                set -eu
                for tool in curl tar uname rm mkdir; do
                  command -v "$tool" >/dev/null 2>&1 || { echo "missing required tool: $tool" >&2; exit 1; }
                done
                arch="$(uname -m)"
                case "$arch" in
                  x86_64) arch="amd64" ;;
                  aarch64|arm64) arch="arm64" ;;
                  *) echo "Unsupported architecture: $arch" >&2; exit 1 ;;
                esac
                dest="$OPENHUMAN_OLLAMA_INSTALL_DIR"
                archive_url="https://ollama.com/download/ollama-linux-${arch}.tar.zst"
                if ! command -v unzstd >/dev/null 2>&1; then
                  echo "missing required tool: unzstd (zstd package)" >&2
                  exit 1
                fi
                rm -rf "$dest"
                mkdir -p "$dest"
                echo ">>> Downloading Ollama for Linux into $dest" >&2
                curl --fail --show-error --location --progress-bar "$archive_url" | tar --use-compress-program=unzstd -xf - -C "$dest"
                chmod 755 "$dest/bin/ollama"
                "#,
            );
        return Ok(cmd);
    }

    #[allow(unreachable_code)]
    Err(format!(
        "Unsupported platform for automatic Ollama install: {}-{}",
        std::env::consts::OS,
        std::env::consts::ARCH
    ))
}

pub(crate) fn find_system_ollama_binary() -> Option<PathBuf> {
    if let Some(from_env) = std::env::var("OLLAMA_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
    {
        let path = PathBuf::from(from_env);
        if path.is_file() {
            return Some(path);
        }
    }

    let binary_name = if cfg!(windows) {
        "ollama.exe"
    } else {
        "ollama"
    };
    if let Some(path_var) = std::env::var_os("PATH") {
        for entry in std::env::split_paths(&path_var) {
            let candidate = entry.join(binary_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    if cfg!(windows) {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(local_app_data) = std::env::var("LOCALAPPDATA") {
            candidates.push(
                PathBuf::from(&local_app_data)
                    .join("Programs")
                    .join("Ollama")
                    .join("ollama.exe"),
            );
            candidates.push(
                PathBuf::from(&local_app_data)
                    .join("Ollama")
                    .join("ollama.exe"),
            );
        }
        if let Ok(program_files) = std::env::var("PROGRAMFILES") {
            candidates.push(
                PathBuf::from(&program_files)
                    .join("Ollama")
                    .join("ollama.exe"),
            );
        }
        for candidate in candidates {
            if candidate.is_file() {
                log::debug!(
                    "[local_ai] found system Ollama at common Windows path: {}",
                    candidate.display()
                );
                return Some(candidate);
            }
        }
    }

    if cfg!(target_os = "macos") {
        let mut candidates = vec![
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/opt/homebrew/bin/ollama"),
        ];
        // Ollama.app installed in /Applications or ~/Applications ships its
        // CLI binary inside the app bundle resources directory.
        let bundle_rel = std::path::Path::new("Applications")
            .join("Ollama.app")
            .join("Contents")
            .join("Resources")
            .join("ollama");
        candidates.push(PathBuf::from("/").join(&bundle_rel));
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(&bundle_rel));
        }
        for candidate in candidates {
            if candidate.is_file() {
                log::debug!(
                    "[local_ai] found system Ollama at macOS path: {}",
                    candidate.display()
                );
                return Some(candidate);
            }
        }
    }

    if cfg!(target_os = "linux") {
        let common = [
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/usr/bin/ollama"),
        ];
        for candidate in common {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;

    /// Serialises tests that mutate process-global environment variables
    /// (OLLAMA_BIN, PATH) with other local-AI tests that also read these
    /// variables. Without this, cargo's test runner can interleave set/remove
    /// calls and cause flakes.

    fn env_lock() -> std::sync::MutexGuard<'static, ()> {
        crate::openhuman::local_ai::local_ai_test_guard()
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
}
