use std::path::{Path, PathBuf};

use futures_util::StreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::install::{find_system_ollama_binary, run_ollama_install_script};
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    ollama_base_url, OllamaModelTag, OllamaPullEvent, OllamaPullProgress, OllamaPullRequest,
    OllamaTagsResponse,
};
use crate::openhuman::local_ai::paths::{find_workspace_ollama_binary, workspace_ollama_binary};
use crate::openhuman::local_ai::presets::{self, VisionMode};

use super::LocalAiService;

impl LocalAiService {
    pub(in crate::openhuman::local_ai::service) async fn ensure_ollama_server(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        if self.ollama_healthy().await {
            // Server is running — verify it can actually execute models by checking
            // if the runner works. A stale server with a missing binary will 500.
            if self.ollama_runner_ok().await {
                return Ok(());
            }
            // Runner is broken (e.g. binary moved). Kill stale server and restart.
            log::warn!("[local_ai] Ollama server responds but runner is broken, restarting");
            self.kill_ollama_server().await;
        }

        let ollama_cmd = self.resolve_or_install_ollama_binary(config).await?;
        self.start_and_wait_for_server(&ollama_cmd).await
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
            return self.start_and_wait_for_server(&system_bin).await;
        };

        self.start_and_wait_for_server(&ollama_cmd).await
    }

    async fn start_and_wait_for_server(&self, ollama_cmd: &Path) -> Result<(), String> {
        if self.ollama_healthy().await {
            return Ok(());
        }

        if let Err(err) = tokio::process::Command::new(ollama_cmd)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
        {
            return Err(format!(
                "Ollama binary not available ({}; error: {err}).",
                ollama_cmd.display()
            ));
        }

        match tokio::process::Command::new(ollama_cmd)
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
        {
            Ok(_) => {
                log::debug!(
                    "[local_ai] spawned `ollama serve` from {}",
                    ollama_cmd.display()
                );
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
        }

        for _ in 0..20 {
            if self.ollama_healthy().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
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
        tokio::process::Command::new(command)
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    async fn download_and_install_ollama(&self, config: &Config) -> Result<(), String> {
        let install_dir = crate::openhuman::local_ai::paths::workspace_ollama_dir(config);
        tokio::fs::create_dir_all(&install_dir)
            .await
            .map_err(|e| format!("failed to create Ollama install directory: {e}"))?;

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
        let healthy = self.ollama_healthy().await;

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
        if !healthy {
            issues.push(format!(
                "Ollama server is not running or not reachable at {}",
                ollama_base_url()
            ));
        }
        if healthy && !chat_found {
            issues.push(format!("Chat model `{}` is not installed", expected_chat));
        }
        if healthy && config.local_ai.preload_embedding_model && !embedding_found {
            issues.push(format!(
                "Embedding model `{}` is not installed",
                expected_embedding
            ));
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
        }
        if let Some(ref e) = tags_error {
            issues.push(format!("Failed to list models: {e}"));
        }

        log::debug!(
            "[local_ai] diagnostics: healthy={} models={} issues={}",
            healthy,
            models.len(),
            issues.len(),
        );

        Ok(serde_json::json!({
            "ollama_running": healthy,
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

    fn resolve_binary_path(&self, config: &Config) -> Option<String> {
        if let Some(ref custom) = config.local_ai.ollama_binary_path {
            let p = PathBuf::from(custom);
            if p.is_file() {
                return Some(custom.clone());
            }
        }
        let workspace_bin = workspace_ollama_binary(config);
        if workspace_bin.is_file() {
            return Some(workspace_bin.display().to_string());
        }
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
    async fn kill_ollama_server(&self) {
        #[cfg(unix)]
        {
            let _ = tokio::process::Command::new("pkill")
                .arg("-f")
                .arg("ollama serve")
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            // Give it a moment to die.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
        #[cfg(windows)]
        {
            let _ = tokio::process::Command::new("taskkill")
                .args(["/F", "/IM", "ollama.exe"])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status()
                .await;
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }

    pub(in crate::openhuman::local_ai::service) async fn has_model(
        &self,
        model: &str,
    ) -> Result<bool, String> {
        let response = self
            .http
            .get(format!("{}/api/tags", ollama_base_url()))
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

#[cfg(test)]
mod tests {
    use super::interrupted_pull_settle_window_secs;

    #[test]
    fn interrupted_pull_waits_when_bytes_were_observed() {
        assert_eq!(interrupted_pull_settle_window_secs(true, 20), 20);
    }

    #[test]
    fn interrupted_pull_does_not_wait_before_any_progress() {
        assert_eq!(interrupted_pull_settle_window_secs(false, 20), 0);
    }

    use crate::openhuman::config::Config;
    use crate::openhuman::local_ai::service::LocalAiService;
    use axum::{routing::get, Json, Router};
    use serde_json::json;

    async fn spawn_mock(app: Router) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        format!("http://127.0.0.1:{}", addr.port())
    }

    #[tokio::test]
    async fn has_model_detects_exact_and_prefixed_tag() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let app = Router::new().route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [
                        {"name": "llama3:latest", "modified_at": "", "size": 1u64, "digest": "d"},
                        {"name": "nomic-embed-text:v1", "modified_at": "", "size": 2u64, "digest": "d"}
                    ]
                }))
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = Config::default();
        let service = LocalAiService::new(&config);
        assert!(service.has_model("llama3").await.unwrap());
        assert!(service.has_model("llama3:latest").await.unwrap());
        assert!(service.has_model("nomic-embed-text").await.unwrap());
        assert!(!service.has_model("__missing__").await.unwrap());

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn has_model_errors_on_non_success_tags_response() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let app = Router::new().route(
            "/api/tags",
            get(|| async { (axum::http::StatusCode::INTERNAL_SERVER_ERROR, "boom") }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = Config::default();
        let service = LocalAiService::new(&config);
        let err = service.has_model("any").await.unwrap_err();
        assert!(err.contains("500") || err.contains("tags failed"));

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn ollama_healthy_returns_true_on_200_tags_response() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let app = Router::new().route("/api/tags", get(|| async { Json(json!({ "models": [] })) }));
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = Config::default();
        let service = LocalAiService::new(&config);
        assert!(service.ollama_healthy().await);

        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn ollama_healthy_returns_false_on_unreachable_url() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        // Point at a port we never bind → connect fails → healthy = false.
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
        }
        let config = Config::default();
        let service = LocalAiService::new(&config);
        assert!(!service.ollama_healthy().await);
        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn diagnostics_reports_server_unreachable_when_url_unbound() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", "http://127.0.0.1:1");
        }
        let config = Config::default();
        let service = LocalAiService::new(&config);
        let diag = service.diagnostics(&config).await.expect("diagnostics");
        assert_eq!(diag["ollama_running"], false);
        let issues = diag["issues"].as_array().cloned().unwrap_or_default();
        assert!(
            !issues.is_empty(),
            "unreachable server must surface an issue"
        );
        assert!(issues
            .iter()
            .any(|v| v.as_str().unwrap_or("").contains("not running")));
        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn diagnostics_with_running_server_but_missing_models_flags_issues() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let app = Router::new().route("/api/tags", get(|| async { Json(json!({ "models": [] })) }));
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = Config::default();
        let service = LocalAiService::new(&config);
        let diag = service.diagnostics(&config).await.expect("diagnostics");
        assert_eq!(diag["ollama_running"], true);
        // No models are installed → expected chat model issue surfaces.
        let issues = diag["issues"].as_array().cloned().unwrap_or_default();
        assert!(!issues.is_empty());
        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn diagnostics_ok_when_expected_models_are_present() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let config = Config::default();
        let chat = crate::openhuman::local_ai::model_ids::effective_chat_model_id(&config);
        let chat_tag = format!("{}:latest", chat);
        let app = Router::new().route(
            "/api/tags",
            get(move || {
                let chat_tag = chat_tag.clone();
                async move {
                    Json(json!({
                        "models": [
                            { "name": chat_tag, "modified_at": "", "size": 1u64, "digest": "d" }
                        ]
                    }))
                }
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let service = LocalAiService::new(&config);
        let diag = service.diagnostics(&config).await.expect("diagnostics");
        assert_eq!(diag["ollama_running"], true);
        assert_eq!(diag["expected"]["chat_found"], true);
        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn list_models_returns_parsed_payload() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let app = Router::new().route(
            "/api/tags",
            get(|| async {
                Json(json!({
                    "models": [
                        { "name": "a:latest", "modified_at": "t", "size": 1u64, "digest": "d1" },
                        { "name": "b:v2", "modified_at": "t", "size": 2u64, "digest": "d2" }
                    ]
                }))
            }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = Config::default();
        let service = LocalAiService::new(&config);
        let models = service.list_models().await.expect("list_models");
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].name, "a:latest");
        assert_eq!(models[1].name, "b:v2");
        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }

    #[tokio::test]
    async fn list_models_errors_on_non_success() {
        let _guard = crate::openhuman::local_ai::LOCAL_AI_TEST_MUTEX
            .lock()
            .expect("local ai mutex");

        let app = Router::new().route(
            "/api/tags",
            get(|| async { (axum::http::StatusCode::SERVICE_UNAVAILABLE, "down") }),
        );
        let base = spawn_mock(app).await;
        unsafe {
            std::env::set_var("OPENHUMAN_OLLAMA_BASE_URL", &base);
        }

        let config = Config::default();
        let service = LocalAiService::new(&config);
        let err = service.list_models().await.unwrap_err();
        assert!(err.contains("503") || err.contains("tags failed"));
        unsafe {
            std::env::remove_var("OPENHUMAN_OLLAMA_BASE_URL");
        }
    }
}
