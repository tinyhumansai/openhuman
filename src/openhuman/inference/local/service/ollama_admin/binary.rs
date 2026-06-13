use std::path::{Path, PathBuf};

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::install::{
    find_system_ollama_binary, run_ollama_install_script,
};
use crate::openhuman::inference::local::process_util::apply_no_window;
use crate::openhuman::inference::paths::find_workspace_ollama_binary;

use super::super::LocalAiService;

impl LocalAiService {
    pub(in crate::openhuman::inference::local::service) async fn resolve_or_install_ollama_binary(
        &self,
        config: &Config,
    ) -> Result<PathBuf, String> {
        // 1. Check user-configured ollama_binary_path from Settings.
        if let Some(ref custom_path) = config.local_ai.ollama_binary_path {
            let path = PathBuf::from(custom_path);
            if path.is_file() {
                log::debug!(
                    "[local_ai] using configured ollama_binary_path: {}",
                    path.display()
                );
                return Ok(path);
            }
            log::warn!(
                "[local_ai] configured ollama_binary_path does not exist: {}, falling through",
                path.display()
            );
        }

        // 2. OLLAMA_BIN env var.
        if let Some(from_env) = std::env::var("OLLAMA_BIN")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            let path = PathBuf::from(from_env);
            if path.exists() {
                return Ok(path);
            }
        }

        if let Some(workspace_bin) = find_workspace_ollama_binary(config) {
            if self.command_works(&workspace_bin).await {
                log::debug!(
                    "[local_ai] using workspace-managed ollama binary: {}",
                    workspace_bin.display()
                );
                return Ok(workspace_bin);
            }
            log::warn!(
                "[local_ai] workspace-managed ollama binary is present but not executable, reinstalling: {}",
                workspace_bin.display()
            );
        }

        if self.command_works(Path::new("ollama")).await {
            return Ok(PathBuf::from("ollama"));
        }

        self.download_and_install_ollama(config).await?;
        if let Some(installed) = find_workspace_ollama_binary(config) {
            Ok(installed)
        } else if let Some(system_bin) = find_system_ollama_binary() {
            log::debug!(
                "[local_ai] workspace binary not found after install, using system binary: {}",
                system_bin.display()
            );
            Ok(system_bin)
        } else {
            Err("Ollama download completed but executable is missing. \
                 The installer may have placed it in an unexpected location. \
                 Set OLLAMA_BIN or configure the path in Settings > Local Model."
                .to_string())
        }
    }

    pub(in crate::openhuman::inference::local::service) async fn command_works(
        &self,
        command: &Path,
    ) -> bool {
        let mut cmd = tokio::process::Command::new(command);
        cmd.arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        apply_no_window(&mut cmd);
        cmd.status().await.map(|s| s.success()).unwrap_or(false)
    }

    pub(in crate::openhuman::inference::local::service) async fn download_and_install_ollama(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        let install_dir = crate::openhuman::inference::paths::workspace_ollama_dir(config);
        tokio::fs::create_dir_all(&install_dir)
            .await
            .map_err(|e| format!("failed to create Ollama install directory: {e}"))?;

        // Crash-resume guard: Inno Setup's installer is spawned via
        // PowerShell's `Start-Process`, which creates a top-level process.
        // It outlives OpenHuman crashing, the user closing the app, or
        // the bootstrap task being cancelled. If a prior launch left an
        // OllamaSetup.exe running, wait for it instead of starting a
        // second one — two concurrent installers race on the same dir
        // and corrupt the install.
        if crate::openhuman::inference::local::install::is_ollama_installer_running() {
            log::info!(
                "[local_ai] detected in-flight OllamaSetup.exe — \
                 waiting for it to finish before deciding whether to install"
            );
            {
                let mut status = self.status.lock();
                status.state = "installing".to_string();
                status.warning = Some("Resuming Ollama install from a previous launch".to_string());
                status.error_detail = None;
                status.error_category = None;
            }
            // Bounded wait: a stuck OllamaSetup.exe (e.g. Inno Setup dialog
            // waiting on user input) must not block app startup forever. Five
            // minutes covers a slow download + UAC prompt; past that we mark
            // the install as failed-but-recoverable and let the caller decide.
            let wait_start = std::time::Instant::now();
            const INSTALLER_WAIT_TIMEOUT: std::time::Duration =
                std::time::Duration::from_secs(5 * 60);
            let mut timed_out = false;
            while crate::openhuman::inference::local::install::is_ollama_installer_running() {
                if wait_start.elapsed() >= INSTALLER_WAIT_TIMEOUT {
                    timed_out = true;
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
            if timed_out {
                log::warn!(
                    "[local_ai] OllamaSetup.exe still running after {}s — giving up the wait",
                    INSTALLER_WAIT_TIMEOUT.as_secs()
                );
                let mut status = self.status.lock();
                status.state = "install_failed".to_string();
                status.warning = None;
                status.error_category = Some("install_stuck".to_string());
                status.error_detail = Some(format!(
                    "Previous OllamaSetup.exe install was still running after {}s. \
                     Cancel the installer (System tray / Task Manager) and retry.",
                    INSTALLER_WAIT_TIMEOUT.as_secs()
                ));
                return Err("Previous Ollama installer is stuck. Cancel it and retry.".to_string());
            }
            // The prior installer is gone. If it succeeded, our regular
            // discovery paths will find the binary and we can short-circuit
            // the install entirely. If it failed, fall through and run a
            // fresh install below.
            if find_workspace_ollama_binary(config).is_some()
                || find_system_ollama_binary().is_some()
            {
                log::info!("[local_ai] resumed prior install completed successfully");
                return Ok(());
            }
            log::warn!(
                "[local_ai] prior installer exited but binary not found — running fresh install"
            );
        }

        {
            let mut status = self.status.lock();
            status.state = "installing".to_string();
            status.warning = Some("Installing Ollama runtime (first run)".to_string());
            status.download_progress = None;
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
            status.error_detail = None;
            status.error_category = None;
        }

        let result = run_ollama_install_script(&install_dir).await?;
        if !result.exit_status.success() {
            let stderr_tail: String = result
                .stderr
                .lines()
                .rev()
                .take(20)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            log::warn!(
                "[local_ai] Ollama install script failed (exit={})\nstdout: {}\nstderr: {}",
                result.exit_status,
                result.stdout,
                result.stderr,
            );
            {
                let mut status = self.status.lock();
                status.error_detail = Some(if stderr_tail.is_empty() {
                    result
                        .stdout
                        .lines()
                        .rev()
                        .take(20)
                        .collect::<Vec<_>>()
                        .into_iter()
                        .rev()
                        .collect::<Vec<_>>()
                        .join("\n")
                } else {
                    stderr_tail
                });
                status.error_category = Some("install".to_string());
            }
            return Err(format!(
                "Ollama install script failed (exit code {}). \
                 Install Ollama manually from https://ollama.com or set its path in Settings > Local Model.",
                result.exit_status.code().unwrap_or(-1)
            ));
        }

        log::debug!(
            "[local_ai] Ollama install script succeeded, stdout: {}",
            result.stdout.chars().take(500).collect::<String>(),
        );

        let installed = find_workspace_ollama_binary(config)
            .or_else(find_system_ollama_binary)
            .ok_or_else(|| "Ollama installer finished but binary was not found".to_string())?;
        log::debug!(
            "[local_ai] Ollama install finished with binary at {}",
            installed.display()
        );

        {
            let mut status = self.status.lock();
            status.warning = Some("Ollama runtime installed".to_string());
            status.download_progress = Some(1.0);
        }
        Ok(())
    }
}
