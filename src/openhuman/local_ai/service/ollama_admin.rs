use std::path::{Path, PathBuf};

use futures_util::StreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::install::{find_system_ollama_binary, run_ollama_install_script};
use crate::openhuman::local_ai::lm_studio_api::lm_studio_base_url;
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    ollama_base_url, OllamaModelTag, OllamaPullEvent, OllamaPullProgress, OllamaPullRequest,
    OllamaTagsResponse,
};
use crate::openhuman::local_ai::paths::{find_workspace_ollama_binary, workspace_ollama_binary};
use crate::openhuman::local_ai::presets::{self, VisionMode};
use crate::openhuman::local_ai::process_util::apply_no_window;
use crate::openhuman::local_ai::provider::{provider_from_config, LocalAiProvider};

use super::spawn_marker::{self, OllamaSpawnMarker};
use super::LocalAiService;

fn lm_studio_models_error_means_unreachable(error: &str) -> bool {
    error.starts_with("lm studio models request failed:")
}

impl LocalAiService {
    pub(in crate::openhuman::local_ai::service) async fn ensure_ollama_server(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        // If openhuman crashed last session and left a daemon running, the
        // spawn marker lets us recognise it and reclaim it (kill + respawn
        // under owned-child tracking) instead of either leaking it forever
        // or hitting an external daemon that just happens to be on :11434.
        self.reclaim_orphan_if_ours(config).await;

        if self.ollama_healthy().await {
            // Server is running — verify it can actually execute models by checking
            // if the runner works. A stale server with a missing binary will 500.
            if self.ollama_runner_ok().await {
                return Ok(());
            }
            // Runner is broken (e.g. binary moved).
            log::warn!("[local_ai] Ollama server responds but runner is broken");
            // Only restart if we own it. Killing an external daemon's
            // broken runner is the user's job, not ours — friendly-fire.
            self.kill_ollama_server().await;
            if self.ollama_healthy().await {
                // Our kill was a no-op (or didn't take effect) — daemon is external.
                return Err("An external Ollama daemon on :11434 has a broken runner. \
                     Restart it manually (or stop it so openhuman can take over)."
                    .to_string());
            }
        }

        let ollama_cmd = self.resolve_or_install_ollama_binary(config).await?;
        self.start_and_wait_for_server(config, &ollama_cmd).await
    }

    /// Like `ensure_ollama_server`, but forces a fresh install of the Ollama binary
    /// (ignoring cached/workspace binaries). Used as a retry after the first attempt fails.
    pub(in crate::openhuman::local_ai::service) async fn ensure_ollama_server_fresh(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        // Force a fresh download regardless of existing binaries.
        self.download_and_install_ollama(config).await?;

        let Some(ollama_cmd) = find_workspace_ollama_binary(config) else {
            // Also check system path after install.
            let system_bin = find_system_ollama_binary()
                .ok_or_else(|| "Ollama installed but binary not found on system".to_string())?;
            // Try to use the system binary directly.
            return self.start_and_wait_for_server(config, &system_bin).await;
        };

        self.start_and_wait_for_server(config, &ollama_cmd).await
    }

    /// Check if a healthy daemon on `:11434` is actually openhuman's own
    /// orphan from a prior session (i.e. we crashed before the graceful
    /// shutdown hook fired). If so, kill it so the upcoming spawn can
    /// resume owned-child tracking. External daemons are never touched.
    async fn reclaim_orphan_if_ours(&self, config: &Config) {
        let Some(marker) = spawn_marker::read_marker(config) else {
            return;
        };
        if !spawn_marker::pid_is_alive(marker.pid) {
            log::debug!(
                "[local_ai] stale ollama spawn marker (pid={} no longer alive); clearing",
                marker.pid
            );
            spawn_marker::clear_marker(config);
            return;
        }
        if !self.ollama_healthy().await {
            // PID is alive but :11434 isn't healthy — either Ollama is
            // mid-boot or the recorded PID was reused for an unrelated
            // process. Leave the marker; either the daemon will come up
            // and the next call will reclaim it, or `start_and_wait_for_server`
            // will overwrite it on a fresh spawn.
            log::debug!(
                "[local_ai] ollama spawn marker pid={} alive but :11434 not healthy yet; \
                 deferring reclaim",
                marker.pid
            );
            return;
        }
        log::info!(
            "[local_ai] reclaiming openhuman-owned ollama orphan from prior session \
             (pid={}, binary={})",
            marker.pid,
            marker.binary_path
        );
        kill_pid_by_id(marker.pid);
        spawn_marker::clear_marker(config);
        // Brief settle so the listener releases :11434 before we respawn.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    async fn start_and_wait_for_server(
        &self,
        config: &Config,
        ollama_cmd: &Path,
    ) -> Result<(), String> {
        if self.ollama_healthy().await {
            // A daemon is already up — adopt it. We did NOT spawn it (or any
            // prior spawn was already reclaimed in `reclaim_orphan_if_ours`),
            // so `owned_ollama` stays `None` and the daemon survives openhuman
            // exit. This is the contract: external/adopted daemons are never
            // killed; only our own children die with us.
            return Ok(());
        }

        // Defensive: if a previous spawn attempt left a stale `Child` in
        // `owned_ollama` (e.g. ensure_ollama_server_fresh after a failed
        // first pass), clear it before respawning. Without this, the new
        // child would replace the field and the old one would be leaked.
        self.kill_ollama_server().await;
        spawn_marker::clear_marker(config);

        let mut version_cmd = tokio::process::Command::new(ollama_cmd);
        version_cmd
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        apply_no_window(&mut version_cmd);
        if let Err(err) = version_cmd.status().await {
            return Err(format!(
                "Ollama binary not available ({}; error: {err}).",
                ollama_cmd.display()
            ));
        }

        let mut serve_cmd = tokio::process::Command::new(ollama_cmd);
        serve_cmd
            .arg("serve")
            .stdout(std::process::Stdio::null())
            // Pipe stderr so we can detect specific failure modes — most
            // importantly Windows Controlled Folder Access blocks, which
            // surface as "Access is denied" / "operation was blocked" /
            // 0x80070005 in Ollama's own stderr when CFA refuses writes
            // to the model cache or even prevents the binary from running.
            .stderr(std::process::Stdio::piped());
        apply_no_window(&mut serve_cmd);
        let mut serve_child = match serve_cmd.spawn() {
            Ok(child) => {
                log::debug!(
                    "[local_ai] spawned `ollama serve` from {}",
                    ollama_cmd.display()
                );
                child
            }
            Err(err) => {
                log::warn!(
                    "[local_ai] failed to spawn `ollama serve` from {}: {err}",
                    ollama_cmd.display()
                );
                return Err(format!(
                    "Failed to start Ollama server ({}): {err}",
                    ollama_cmd.display()
                ));
            }
        };

        // Drain stderr into a bounded buffer in the background. We keep
        // the last ~16KB so we can quote it back to the user / Sentry on
        // failure but don't grow unbounded if Ollama logs heavily.
        let stderr_buffer = std::sync::Arc::new(parking_lot::Mutex::new(String::new()));
        if let Some(stderr) = serve_child.stderr.take() {
            let buf = std::sync::Arc::clone(&stderr_buffer);
            tokio::spawn(async move {
                use tokio::io::{AsyncBufReadExt, BufReader};
                let mut reader = BufReader::new(stderr);
                let mut line = String::new();
                while reader
                    .read_line(&mut line)
                    .await
                    .map(|n| n > 0)
                    .unwrap_or(false)
                {
                    let mut b = buf.lock();
                    let new_len = b.len() + line.len();
                    if new_len > 16 * 1024 {
                        let drop_n = new_len - 16 * 1024;
                        let drop_n = std::cmp::min(drop_n, b.len());
                        b.drain(0..drop_n);
                    }
                    b.push_str(&line);
                    line.clear();
                }
            });
        }

        for _ in 0..20 {
            if self.ollama_healthy().await {
                // Daemon is up. Take ownership so we can kill it on exit and
                // write the spawn marker so a crashed openhuman can reclaim
                // this PID on next launch instead of orphaning it forever.
                let pid = serve_child.id().unwrap_or(0);
                if pid == 0 {
                    log::warn!(
                        "[local_ai] spawned ollama child has no PID — owned-child kill \
                         will be a no-op but daemon is healthy, continuing"
                    );
                } else {
                    let marker = OllamaSpawnMarker::new(pid, ollama_cmd);
                    if let Err(e) = spawn_marker::write_marker(config, &marker) {
                        // Marker write failure is non-fatal — graceful shutdown
                        // still kills via the in-memory `Child` handle. Only
                        // crash-recovery on next launch is degraded.
                        log::warn!(
                            "[local_ai] failed to write ollama spawn marker (pid={pid}): {e}"
                        );
                    }
                }
                *self.owned_ollama.lock() = Some(serve_child);
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        // Health probe timed out. The serve child is unhealthy and may be
        // holding the Ollama port — kill it before returning so the next
        // bootstrap attempt isn't blocked by a zombie listener.
        if let Err(err) = serve_child.kill().await {
            log::warn!("[local_ai] failed to kill unhealthy `ollama serve` child: {err}");
        }

        // Classify the failure from captured stderr.
        let stderr_snapshot = stderr_buffer.lock().clone();
        let lowered = stderr_snapshot.to_ascii_lowercase();
        // Match only explicit Controlled Folder Access markers. Generic
        // strings like "access is denied" or "is not recognized as a trusted"
        // appear in many unrelated Windows errors and previously caused us
        // to surface a misleading CFA remediation message.
        let cfa_signatures = ["controlled folder access", "operation was blocked"];
        let cfa_hit = cfa_signatures.iter().any(|sig| lowered.contains(sig));
        if cfa_hit {
            log::warn!(
                "[local_ai] Ollama failed to start — Controlled Folder Access blocked it. \
                 stderr tail: {stderr_snapshot}"
            );
            self.status.lock().error_detail = Some(stderr_snapshot);
            return Err(format!(
                "Ollama was blocked by Windows Controlled Folder Access. \
                 Open Windows Security → Ransomware protection → Allow an app \
                 through Controlled folder access, and add `{}`.",
                ollama_cmd.display()
            ));
        }
        // Non-CFA timeout — surface the stderr tail anyway for diagnosis.
        if !stderr_snapshot.is_empty() {
            log::warn!("[local_ai] Ollama not reachable. stderr tail: {stderr_snapshot}");
            self.status.lock().error_detail = Some(stderr_snapshot);
        }
        Err("Ollama runtime is not reachable after fresh install. Start `ollama serve` manually and retry.".to_string())
    }

    async fn resolve_or_install_ollama_binary(&self, config: &Config) -> Result<PathBuf, String> {
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

    async fn command_works(&self, command: &Path) -> bool {
        let mut cmd = tokio::process::Command::new(command);
        cmd.arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        apply_no_window(&mut cmd);
        cmd.status().await.map(|s| s.success()).unwrap_or(false)
    }

    async fn download_and_install_ollama(&self, config: &Config) -> Result<(), String> {
        let install_dir = crate::openhuman::local_ai::paths::workspace_ollama_dir(config);
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
        if crate::openhuman::local_ai::install::is_ollama_installer_running() {
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
            while crate::openhuman::local_ai::install::is_ollama_installer_running() {
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

    async fn ollama_healthy(&self) -> bool {
        self.http
            .get(format!("{}/api/tags", ollama_base_url()))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Filesystem-only precondition: is *any* Ollama binary discoverable?
    ///
    /// This is the cheapest possible check — no process spawns, no HTTP, no
    /// timeouts. Callers that need to decide whether it's even worth talking
    /// to `/api/tags` should consult this first. Returning `false` here means
    /// the UI should drive the user to install Ollama instead of polling for
    /// model state that can never appear.
    pub(in crate::openhuman::local_ai::service) fn ollama_binary_present(
        &self,
        config: &Config,
    ) -> bool {
        if let Some(ref custom) = config.local_ai.ollama_binary_path {
            if PathBuf::from(custom).is_file() {
                return true;
            }
        }
        if let Some(env_path) = std::env::var("OLLAMA_BIN")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            if PathBuf::from(env_path).is_file() {
                return true;
            }
        }
        if find_workspace_ollama_binary(config).is_some() {
            return true;
        }
        find_system_ollama_binary().is_some()
    }

    pub(in crate::openhuman::local_ai::service) async fn ensure_models_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        let chat_model = model_ids::effective_chat_model_id(config);
        self.ensure_ollama_model_available(&chat_model, "chat")
            .await?;

        match presets::vision_mode_for_config(&config.local_ai) {
            VisionMode::Disabled => {
                self.status.lock().vision_state = "disabled".to_string();
            }
            VisionMode::Ondemand => {
                self.status.lock().vision_state = "idle".to_string();
            }
            VisionMode::Bundled => {
                let vision_model = model_ids::effective_vision_model_id(config);
                self.ensure_ollama_model_available(&vision_model, "vision")
                    .await?;
                self.status.lock().vision_state = "ready".to_string();
            }
        }

        let embedding_model = model_ids::effective_embedding_model_id(config);
        if config.local_ai.preload_embedding_model {
            self.ensure_ollama_model_available(&embedding_model, "embedding")
                .await?;
            self.status.lock().embedding_state = "ready".to_string();
        }

        if config.local_ai.preload_stt_model {
            self.ensure_stt_asset_available(config).await?;
        }

        if config.local_ai.preload_tts_voice {
            self.ensure_tts_asset_available(config).await?;
        }

        Ok(())
    }

    pub(in crate::openhuman::local_ai::service) async fn ensure_ollama_model_available(
        &self,
        model_id: &str,
        label: &str,
    ) -> Result<(), String> {
        if self.has_model(model_id).await? {
            return Ok(());
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!(
                "Pulling {} model `{}` from Ollama library",
                label, model_id
            ));
            match label {
                "vision" => status.vision_state = "downloading".to_string(),
                "embedding" => status.embedding_state = "downloading".to_string(),
                _ => {}
            }
            status.download_progress = Some(0.0);
            status.downloaded_bytes = Some(0);
            status.total_bytes = None;
            status.download_speed_bps = Some(0);
            status.eta_seconds = None;
        }

        const MAX_PULL_RETRIES: usize = 3;
        const PULL_RETRY_BACKOFF_MS: u64 = 1_500;
        const PULL_INTERRUPT_SETTLE_SECS: u64 = 20;
        let mut last_error: Option<String> = None;

        for attempt in 1..=MAX_PULL_RETRIES {
            if attempt > 1 {
                let retry_msg = format!(
                    "Ollama pull stream interrupted. Retrying {}/{}...",
                    attempt, MAX_PULL_RETRIES
                );
                {
                    let mut status = self.status.lock();
                    status.state = "downloading".to_string();
                    status.warning = Some(retry_msg.clone());
                }
                log::warn!(
                    "[local_ai] pull retry {}/{} for model `{}` after interruption",
                    attempt,
                    MAX_PULL_RETRIES,
                    model_id
                );
                tokio::time::sleep(std::time::Duration::from_millis(
                    PULL_RETRY_BACKOFF_MS * attempt as u64,
                ))
                .await;
            }

            let response = match self
                .http
                .post(format!("{}/api/pull", ollama_base_url()))
                .json(&OllamaPullRequest {
                    name: model_id.to_string(),
                    stream: true,
                })
                // Model pulls are long-running streaming responses; the default 30s
                // client timeout can interrupt healthy downloads mid-stream.
                .timeout(std::time::Duration::from_secs(30 * 60))
                .send()
                .await
            {
                Ok(response) => response,
                Err(e) => {
                    let err = format!("ollama pull request failed: {e}");
                    last_error = Some(err.clone());
                    if attempt < MAX_PULL_RETRIES {
                        continue;
                    }
                    return Err(format!("{err} after {MAX_PULL_RETRIES} attempts"));
                }
            };
            if !response.status().is_success() {
                let status = response.status();
                let body = response.text().await.unwrap_or_default();
                let detail = body.trim();
                return Err(format!(
                    "ollama pull failed with status {}{}",
                    status,
                    if detail.is_empty() {
                        String::new()
                    } else {
                        format!(": {detail}")
                    }
                ));
            }

            let mut stream = response.bytes_stream();
            let mut pending = String::new();
            let mut stream_error: Option<String> = None;
            let started_at = std::time::Instant::now();
            let mut progress = OllamaPullProgress::default();
            let mut observed_bytes = false;
            while let Some(item) = stream.next().await {
                let chunk = match item {
                    Ok(value) => value,
                    Err(e) => {
                        stream_error = Some(format!("ollama pull stream error: {e}"));
                        break;
                    }
                };
                pending.push_str(&String::from_utf8_lossy(&chunk));
                while let Some(pos) = pending.find('\n') {
                    let line = pending[..pos].trim().to_string();
                    pending = pending[pos + 1..].to_string();
                    if line.is_empty() {
                        continue;
                    }
                    let event: OllamaPullEvent = match serde_json::from_str(&line) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };
                    if let Some(err) = event.error {
                        return Err(format!("ollama pull error: {err}"));
                    }

                    progress.observe(&event);
                    let completed = progress.aggregate_downloaded();
                    let total = progress.aggregate_total();
                    let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                    let speed_bps = (completed as f64 / elapsed).round().max(0.0) as u64;
                    let eta_seconds = total.and_then(|t| {
                        if completed >= t || speed_bps == 0 {
                            None
                        } else {
                            Some((t.saturating_sub(completed)) / speed_bps.max(1))
                        }
                    });
                    observed_bytes |= completed > 0;

                    let mut status = self.status.lock();
                    if let Some(status_text) = event.status.as_deref() {
                        status.warning = Some(format!("Ollama pull: {status_text}"));
                        if status_text.eq_ignore_ascii_case("success") {
                            status.download_progress = Some(1.0);
                        }
                    }
                    status.downloaded_bytes = Some(completed);
                    status.total_bytes = total;
                    status.download_speed_bps = Some(speed_bps);
                    status.eta_seconds = eta_seconds;
                    status.download_progress = total
                        .map(|t| (completed as f32 / t as f32).clamp(0.0, 1.0))
                        .or(Some(0.0));
                }
            }

            if let Some(err) = stream_error {
                last_error = Some(err.clone());
                let resumed = self
                    .wait_for_model_after_pull_interruption(
                        model_id,
                        attempt,
                        MAX_PULL_RETRIES,
                        observed_bytes,
                        PULL_INTERRUPT_SETTLE_SECS,
                    )
                    .await?;
                if resumed {
                    break;
                }
                if attempt < MAX_PULL_RETRIES {
                    continue;
                }
                return Err(format!("{err} after {MAX_PULL_RETRIES} attempts"));
            }

            if self.has_model(model_id).await? {
                break;
            }

            last_error = Some(format!(
                "ollama pull finished but model `{}` was not found",
                model_id
            ));
            let resumed = self
                .wait_for_model_after_pull_interruption(
                    model_id,
                    attempt,
                    MAX_PULL_RETRIES,
                    observed_bytes,
                    PULL_INTERRUPT_SETTLE_SECS,
                )
                .await?;
            if resumed {
                break;
            }
            if attempt < MAX_PULL_RETRIES {
                continue;
            }
        }

        if !self.has_model(model_id).await? {
            return Err(last_error.unwrap_or_else(|| {
                format!(
                    "ollama pull finished but model `{}` was not found",
                    model_id
                )
            }));
        }

        match label {
            "vision" => self.status.lock().vision_state = "ready".to_string(),
            "embedding" => self.status.lock().embedding_state = "ready".to_string(),
            _ => {}
        }

        Ok(())
    }

    async fn wait_for_model_after_pull_interruption(
        &self,
        model_id: &str,
        attempt: usize,
        max_attempts: usize,
        observed_bytes: bool,
        settle_window_secs: u64,
    ) -> Result<bool, String> {
        let wait_secs = interrupted_pull_settle_window_secs(observed_bytes, settle_window_secs);
        if wait_secs == 0 {
            return Ok(false);
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!(
                "Ollama pull stream disconnected. Waiting up to {wait_secs}s for ongoing download to resume before retry {}/{}.",
                attempt + 1,
                max_attempts
            ));
        }
        log::warn!(
            "[local_ai] pull stream interrupted for model `{}`; waiting up to {}s before retry {}/{}",
            model_id,
            wait_secs,
            attempt + 1,
            max_attempts
        );

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(wait_secs);
        while std::time::Instant::now() < deadline {
            if self.has_model(model_id).await? {
                log::info!(
                    "[local_ai] model `{}` became available after interrupted pull stream",
                    model_id
                );
                return Ok(true);
            }
            tokio::time::sleep(std::time::Duration::from_secs(1)).await;
        }

        Ok(false)
    }

    /// Run full diagnostics: check Ollama server health, list installed models,
    /// and verify expected models are present. Returns a JSON-serializable report.
    pub async fn diagnostics(&self, config: &Config) -> Result<serde_json::Value, String> {
        if provider_from_config(config) == LocalAiProvider::LmStudio {
            return self.lm_studio_diagnostics(config).await;
        }

        let base_url = ollama_base_url();
        let healthy = self.ollama_healthy().await;

        log::debug!(
            "[local_ai] diagnostics: entry base_url={} healthy={}",
            base_url,
            healthy
        );

        let (models, tags_error) = if healthy {
            match self.list_models().await {
                Ok(models) => (models, None),
                Err(e) => (vec![], Some(e)),
            }
        } else {
            (vec![], None)
        };

        let expected_chat = model_ids::effective_chat_model_id(config);
        let expected_embedding = model_ids::effective_embedding_model_id(config);
        let expected_vision = model_ids::effective_vision_model_id(config);

        let model_names: Vec<String> = models.iter().map(|m| m.name.to_ascii_lowercase()).collect();
        let has = |target: &str| -> bool {
            let t = target.to_ascii_lowercase();
            model_names
                .iter()
                .any(|n| *n == t || n.starts_with(&(t.clone() + ":")))
        };

        let chat_found = has(&expected_chat);
        let embedding_found = has(&expected_embedding);
        let vision_found = has(&expected_vision);

        let binary_path = self.resolve_binary_path(config);

        let mut issues: Vec<String> = Vec::new();
        let mut repair_actions: Vec<serde_json::Value> = Vec::new();

        if !healthy {
            issues.push(format!(
                "Ollama server is not running or not reachable at {}",
                base_url
            ));
            if binary_path.is_none() {
                repair_actions.push(serde_json::json!({"action": "install_ollama"}));
            } else {
                repair_actions.push(serde_json::json!({
                    "action": "start_server",
                    "binary_path": binary_path,
                }));
            }
        }
        if healthy && !chat_found {
            issues.push(format!("Chat model `{}` is not installed", expected_chat));
            repair_actions.push(serde_json::json!({
                "action": "pull_model",
                "model": expected_chat,
            }));
        }
        if healthy && config.local_ai.preload_embedding_model && !embedding_found {
            issues.push(format!(
                "Embedding model `{}` is not installed",
                expected_embedding
            ));
            repair_actions.push(serde_json::json!({
                "action": "pull_model",
                "model": expected_embedding,
            }));
        }
        if healthy
            && matches!(
                presets::vision_mode_for_config(&config.local_ai),
                VisionMode::Bundled
            )
            && !vision_found
        {
            issues.push(format!(
                "Vision model `{}` is not installed",
                expected_vision
            ));
            repair_actions.push(serde_json::json!({
                "action": "pull_model",
                "model": expected_vision,
            }));
        }
        if let Some(ref e) = tags_error {
            issues.push(format!("Failed to list models: {e}"));
        }

        log::debug!(
            "[local_ai] diagnostics: healthy={} models={} issues={} repair_actions={}",
            healthy,
            models.len(),
            issues.len(),
            repair_actions.len(),
        );

        Ok(serde_json::json!({
            "ollama_running": healthy,
            "ollama_base_url": base_url,
            "ollama_binary_path": binary_path,
            "installed_models": models,
            "vision_mode": presets::vision_mode_for_config(&config.local_ai),
            "expected": {
                "chat_model": expected_chat,
                "chat_found": chat_found,
                "embedding_model": expected_embedding,
                "embedding_found": embedding_found,
                "vision_model": expected_vision,
                "vision_found": vision_found,
            },
            "issues": issues,
            "repair_actions": repair_actions,
            "ok": issues.is_empty(),
        }))
    }

    async fn list_models(&self) -> Result<Vec<OllamaModelTag>, String> {
        let base = ollama_base_url();
        let url = format!("{base}/api/tags");
        tracing::debug!(
            target: "local_ai::ollama_admin",
            %base,
            %url,
            "[local_ai:ollama_admin] list_models: sending GET"
        );

        let response = self
            .http
            .get(&url)
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| {
                tracing::error!(
                    target: "local_ai::ollama_admin",
                    %url,
                    error = %e,
                    "[local_ai:ollama_admin] list_models: request send failed"
                );
                format!("ollama tags request failed: {e}")
            })?;

        let status = response.status();
        tracing::debug!(
            target: "local_ai::ollama_admin",
            %url,
            %status,
            "[local_ai:ollama_admin] list_models: received response"
        );

        if !status.is_success() {
            let body = response.text().await.unwrap_or_default();
            tracing::error!(
                target: "local_ai::ollama_admin",
                %url,
                %status,
                body = %body,
                "[local_ai:ollama_admin] list_models: non-success response"
            );
            return Err(format!(
                "ollama tags failed with status {}: {}",
                status,
                body.trim()
            ));
        }

        // Read the body as text first so we can log it if JSON parsing fails.
        let body = response.text().await.map_err(|e| {
            tracing::error!(
                target: "local_ai::ollama_admin",
                %url,
                error = %e,
                "[local_ai:ollama_admin] list_models: failed to read response body"
            );
            format!("ollama tags body read failed: {e}")
        })?;

        let payload: OllamaTagsResponse = serde_json::from_str(&body).map_err(|e| {
            tracing::error!(
                target: "local_ai::ollama_admin",
                %url,
                body = %body,
                error = %e,
                "[local_ai:ollama_admin] list_models: JSON parse failed"
            );
            format!("ollama tags parse failed: {e}")
        })?;

        tracing::debug!(
            target: "local_ai::ollama_admin",
            %url,
            models = payload.models.len(),
            "[local_ai:ollama_admin] list_models: parsed successfully"
        );

        Ok(payload.models)
    }

    async fn lm_studio_diagnostics(&self, config: &Config) -> Result<serde_json::Value, String> {
        let base_url = lm_studio_base_url(config);
        let models_result = self.list_lm_studio_models(config).await;
        let (models, models_error, healthy) = match models_result {
            Ok(models) => (models, None, true),
            Err(err) => {
                let reachable = !lm_studio_models_error_means_unreachable(&err);
                (vec![], Some(err), reachable)
            }
        };

        let expected_chat = model_ids::effective_chat_model_id(config);
        let model_names: Vec<String> = models.iter().map(|m| m.name.to_ascii_lowercase()).collect();
        let chat_found = model_names
            .iter()
            .any(|name| name == &expected_chat.to_ascii_lowercase());

        let mut issues: Vec<String> = Vec::new();
        let mut repair_actions: Vec<serde_json::Value> = Vec::new();

        if !healthy {
            let detail = models_error
                .as_deref()
                .map(|err| format!(": {err}"))
                .unwrap_or_default();
            issues.push(format!(
                "LM Studio server is not running or not reachable at {}{}",
                base_url, detail
            ));
            repair_actions.push(serde_json::json!({
                "action": "start_lm_studio_server",
                "base_url": base_url,
            }));
        }
        if healthy && models_error.is_none() && models.is_empty() {
            issues.push("LM Studio is reachable but no models are loaded".to_string());
            repair_actions.push(serde_json::json!({
                "action": "load_lm_studio_model",
            }));
        } else if healthy && models_error.is_none() && !chat_found {
            issues.push(format!(
                "Chat model `{}` is not loaded in LM Studio",
                expected_chat
            ));
            repair_actions.push(serde_json::json!({
                "action": "load_lm_studio_model",
                "model": expected_chat,
            }));
        }
        if healthy {
            if let Some(ref err) = models_error {
                issues.push(format!("Failed to list LM Studio models: {err}"));
            }
        }

        tracing::debug!(
            provider = "lm_studio",
            %base_url,
            healthy,
            models = models.len(),
            issues = issues.len(),
            "[local_ai] diagnostics"
        );

        Ok(serde_json::json!({
            "provider": "lm_studio",
            "lm_studio_running": healthy,
            "lm_studio_base_url": base_url,
            "ollama_running": false,
            "ollama_base_url": serde_json::Value::Null,
            "ollama_binary_path": serde_json::Value::Null,
            "installed_models": models,
            "vision_mode": "disabled",
            "expected": {
                "chat_model": expected_chat,
                "chat_found": chat_found,
                "embedding_model": model_ids::effective_embedding_model_id(config),
                "embedding_found": false,
                "vision_model": model_ids::effective_vision_model_id(config),
                "vision_found": false,
            },
            "issues": issues,
            "repair_actions": repair_actions,
            "ok": issues.is_empty(),
        }))
    }

    fn resolve_binary_path(&self, config: &Config) -> Option<String> {
        // 1. Explicit user-configured path in Settings.
        if let Some(ref custom) = config.local_ai.ollama_binary_path {
            let p = PathBuf::from(custom);
            if p.is_file() {
                log::debug!(
                    "[local_ai] resolve_binary_path: using configured path {}",
                    p.display()
                );
                return Some(custom.clone());
            }
        }

        // 2. OLLAMA_BIN env var (mirrors bootstrap detection).
        if let Some(from_env) = std::env::var("OLLAMA_BIN")
            .ok()
            .filter(|v| !v.trim().is_empty())
        {
            let p = PathBuf::from(&from_env);
            if p.is_file() {
                log::debug!(
                    "[local_ai] resolve_binary_path: using OLLAMA_BIN {}",
                    p.display()
                );
                return Some(from_env);
            }
        }

        // 3. Workspace-managed binary installed by the app.
        let workspace_bin = workspace_ollama_binary(config);
        if workspace_bin.is_file() {
            log::debug!(
                "[local_ai] resolve_binary_path: using workspace binary {}",
                workspace_bin.display()
            );
            return Some(workspace_bin.display().to_string());
        }

        // 4. Bare `ollama` on PATH — same as bootstrap's `which ollama` step.
        let binary_name = if cfg!(windows) {
            "ollama.exe"
        } else {
            "ollama"
        };
        if let Some(path_var) = std::env::var_os("PATH") {
            for dir in std::env::split_paths(&path_var) {
                let candidate = dir.join(binary_name);
                if candidate.is_file() {
                    log::debug!(
                        "[local_ai] resolve_binary_path: found on PATH at {}",
                        candidate.display()
                    );
                    return Some(candidate.display().to_string());
                }
            }
        }

        // 5. Platform-specific well-known locations (macOS bundles, Windows, Linux).
        crate::openhuman::local_ai::install::find_system_ollama_binary()
            .map(|p| p.display().to_string())
    }

    /// Quick check that the Ollama runner can actually exec models.
    /// Sends a tiny generate request and checks for a 500 "fork/exec" error.
    async fn ollama_runner_ok(&self) -> bool {
        let resp = self
            .http
            .post(format!("{}/api/tags", ollama_base_url()))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                // Tags endpoint works — but the runner error only shows up on model exec.
                // Do a lightweight pull-status check (won't download, just checks).
                let check = self
                    .http
                    .post(format!("{}/api/show", ollama_base_url()))
                    .json(&serde_json::json!({"name": "___nonexistent_probe___"}))
                    .timeout(std::time::Duration::from_secs(3))
                    .send()
                    .await;
                match check {
                    Ok(r) => {
                        let status = r.status().as_u16();
                        let body = r.text().await.unwrap_or_default();
                        // 404 = model not found — runner is fine. 500 with fork/exec = broken.
                        if status == 500 && body.contains("fork/exec") {
                            log::warn!("[local_ai] ollama runner broken: {body}");
                            return false;
                        }
                        true
                    }
                    Err(_) => true, // network error, assume ok
                }
            }
            _ => false,
        }
    }

    /// Kill any running Ollama server process so we can restart with the correct binary.
    /// Kill the `ollama serve` daemon openhuman itself spawned, if any.
    ///
    /// **No-op when openhuman never spawned a daemon** (i.e. it adopted an
    /// externally-managed one via the `ollama_healthy()` fast-path, or no
    /// daemon was started at all). This avoids the friendly-fire bug from
    /// the previous blanket `taskkill /IM ollama.exe` / `pkill -f` which
    /// would terminate any Ollama on the host — including ones started by
    /// the user's CLI, tray app, or other tooling.
    ///
    /// External daemons can be replaced/restarted by the user; killing
    /// them out from under their owner is never the right move from inside
    /// a desktop app.
    async fn kill_ollama_server(&self) {
        let maybe_child = self.owned_ollama.lock().take();
        let Some(mut child) = maybe_child else {
            log::debug!(
                "[local_ai] kill_ollama_server: no openhuman-owned daemon; \
                 leaving any external Ollama on :11434 untouched"
            );
            return;
        };
        let pid = child.id().unwrap_or(0);
        match child.kill().await {
            Ok(()) => {
                log::info!("[local_ai] killed openhuman-owned ollama serve (pid={pid})");
                // Reap so the OS doesn't keep the zombie around on Unix.
                let _ = child.wait().await;
            }
            Err(err) => {
                log::warn!("[local_ai] kill of owned ollama serve pid={pid} failed: {err}");
            }
        }
        // Give the kernel a moment to release :11434 before any imminent
        // respawn races for the same port.
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
    }

    /// Public shutdown hook for the Tauri exit lifecycle.
    ///
    /// Kills the openhuman-owned `ollama serve` (if any) and clears the
    /// spawn marker so the next launch doesn't try to reclaim a daemon
    /// that's already dead. Idempotent — safe to call from both
    /// `RunEvent::ExitRequested` and window-close paths.
    pub async fn shutdown_owned_ollama(&self, config: &Config) {
        self.kill_ollama_server().await;
        spawn_marker::clear_marker(config);
    }

    pub(in crate::openhuman::local_ai::service) async fn has_model(
        &self,
        model: &str,
    ) -> Result<bool, String> {
        // Issue the /api/tags GET directly. We previously short-circuited via
        // ollama_healthy(), but that doubled the number of /api/tags round-trips
        // on healthy polls (one probe + one tags fetch). With three has_model()
        // calls per assets_status poll (chat, vision, embedding) that was 6
        // network calls instead of 3. The 500ms connect_timeout on the shared
        // reqwest client (set in bootstrap.rs) bounds the cost when the server
        // is down — the connect failure surfaces as Err, same as ollama_healthy()
        // would have surfaced as `false`.
        log::debug!("[local_ai] has_model: checking for model `{model}`");
        let response = self
            .http
            .get(format!("{}/api/tags", ollama_base_url()))
            // Per-request timeout matches list_models (5s). The shared client's
            // connect_timeout only bounds the TCP handshake; without this a
            // hung server (accepted connection, no response body) would block
            // assets_status polls indefinitely.
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| format!("ollama tags request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            let detail = body.trim();
            return Err(format!(
                "ollama tags failed with status {}{}",
                status,
                if detail.is_empty() {
                    String::new()
                } else {
                    format!(": {detail}")
                }
            ));
        }
        let payload: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama tags parse failed: {e}"))?;

        let target = model.to_ascii_lowercase();
        Ok(payload.models.iter().any(|m| {
            let name = m.name.to_ascii_lowercase();
            name == target || name.starts_with(&(target.clone() + ":"))
        }))
    }
}

fn interrupted_pull_settle_window_secs(observed_bytes: bool, settle_window_secs: u64) -> u64 {
    if observed_bytes {
        settle_window_secs.max(1)
    } else {
        0
    }
}

/// Kill a process by PID using `sysinfo`'s cross-platform `Process::kill`.
///
/// Used by `reclaim_orphan_if_ours` where we no longer have the original
/// `tokio::process::Child` handle (the spawning openhuman crashed) but
/// recorded the PID in the spawn marker.
fn kill_pid_by_id(pid: u32) {
    use sysinfo::{Pid, ProcessesToUpdate, System};
    let target = Pid::from_u32(pid);
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::Some(&[target]), true);
    match sys.process(target) {
        Some(proc) => {
            if proc.kill() {
                log::info!("[local_ai] killed reclaimed ollama orphan pid={pid}");
            } else {
                // sysinfo's kill returns false if the platform refused
                // (permissions, race with exit). The next ollama_healthy()
                // check will reveal whether the daemon is actually gone.
                log::warn!("[local_ai] sysinfo Process::kill returned false for pid={pid}");
            }
        }
        None => {
            log::debug!("[local_ai] kill_pid_by_id: pid={pid} no longer present");
        }
    }
}

#[cfg(test)]
#[path = "ollama_admin_tests.rs"]
mod tests;
