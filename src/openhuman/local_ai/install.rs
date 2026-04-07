//! Automatic Ollama installer and system binary discovery.

use std::path::{Path, PathBuf};

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

fn build_install_command(install_dir: &Path) -> Result<tokio::process::Command, String> {
    #[cfg(target_os = "windows")]
    {
        let mut cmd = tokio::process::Command::new("powershell");
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
            $args = "/VERYSILENT /NORESTART /SUPPRESSMSGBOXES /CURRENTUSER /DIR=""$installDir"""
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
        let common = [
            PathBuf::from("/usr/local/bin/ollama"),
            PathBuf::from("/opt/homebrew/bin/ollama"),
        ];
        for candidate in common {
            if candidate.is_file() {
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
