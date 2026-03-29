//! Automatic Ollama installer and system binary discovery.

use std::path::PathBuf;

pub(crate) async fn run_ollama_install_script() -> Result<std::process::ExitStatus, String> {
    #[cfg(target_os = "windows")]
    {
        return tokio::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-ExecutionPolicy",
                "Bypass",
                "-Command",
                "irm https://ollama.com/install.ps1 | iex",
            ])
            .status()
            .await
            .map_err(|e| format!("failed to execute Ollama PowerShell installer: {e}"));
    }

    #[cfg(target_os = "macos")]
    {
        return tokio::process::Command::new("sh")
            .arg("-lc")
            .arg("curl -fsSL https://ollama.com/install.sh | sh -mac")
            .status()
            .await
            .map_err(|e| format!("failed to execute Ollama macOS installer: {e}"));
    }

    #[cfg(target_os = "linux")]
    {
        return tokio::process::Command::new("sh")
            .arg("-lc")
            .arg("curl -fsSL https://ollama.com/install.sh | sh")
            .status()
            .await
            .map_err(|e| format!("failed to execute Ollama Linux installer: {e}"));
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
