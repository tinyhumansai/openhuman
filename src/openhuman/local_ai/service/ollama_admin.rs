use std::path::{Path, PathBuf};

use futures_util::StreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::install::{find_system_ollama_binary, run_ollama_install_script};
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    OllamaModelTag, OllamaPullEvent, OllamaPullRequest, OllamaTagsResponse, OLLAMA_BASE_URL,
};
use crate::openhuman::local_ai::paths::workspace_ollama_binary;

use super::LocalAiService;

impl LocalAiService {
    pub(in crate::openhuman::local_ai::service) async fn ensure_ollama_server(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        if self.ollama_healthy().await {
            return Ok(());
        }

        let ollama_cmd = self.resolve_or_install_ollama_binary(config).await?;

        if let Err(err) = tokio::process::Command::new(&ollama_cmd)
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

        let _ = tokio::process::Command::new(&ollama_cmd)
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

        for _ in 0..20 {
            if self.ollama_healthy().await {
                return Ok(());
            }
            tokio::time::sleep(std::time::Duration::from_millis(300)).await;
        }

        Err("Ollama runtime is not reachable at http://127.0.0.1:11434. Start `ollama serve` and retry.".to_string())
    }

    /// Like `ensure_ollama_server`, but forces a fresh install of the Ollama binary
    /// (ignoring cached/workspace binaries). Used as a retry after the first attempt fails.
    pub(in crate::openhuman::local_ai::service) async fn ensure_ollama_server_fresh(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        // Force a fresh download regardless of existing binaries.
        self.download_and_install_ollama(config).await?;

        let ollama_cmd = workspace_ollama_binary(config);
        if !ollama_cmd.is_file() {
            // Also check system path after install.
            let system_bin = find_system_ollama_binary()
                .ok_or_else(|| "Ollama installed but binary not found on system".to_string())?;
            // Try to use the system binary directly.
            return self.start_and_wait_for_server(&system_bin).await;
        }

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

        let _ = tokio::process::Command::new(ollama_cmd)
            .arg("serve")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();

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

        let workspace_bin = workspace_ollama_binary(config);
        if workspace_bin.is_file() {
            return Ok(workspace_bin);
        }

        if self.command_works(Path::new("ollama")).await {
            return Ok(PathBuf::from("ollama"));
        }

        self.download_and_install_ollama(config).await?;
        let installed = workspace_ollama_binary(config);
        if installed.is_file() {
            Ok(installed)
        } else {
            Err("Ollama download completed but executable is missing.".to_string())
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

        let result = run_ollama_install_script().await?;
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
                    result.stdout.lines().rev().take(20).collect::<Vec<_>>()
                        .into_iter().rev().collect::<Vec<_>>().join("\n")
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

        let installed = find_system_ollama_binary()
            .ok_or_else(|| "Ollama installer finished but binary was not found".to_string())?;
        let dest = workspace_ollama_binary(config);
        tokio::fs::copy(&installed, &dest)
            .await
            .map_err(|e| format!("failed to copy Ollama binary into workspace: {e}"))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))
                .map_err(|e| format!("failed to set Ollama binary permissions: {e}"))?;
        }

        {
            let mut status = self.status.lock();
            status.warning = Some("Ollama runtime installed".to_string());
            status.download_progress = Some(1.0);
        }
        Ok(())
    }

    async fn ollama_healthy(&self) -> bool {
        self.http
            .get(format!("{OLLAMA_BASE_URL}/api/tags"))
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

        let vision_model = model_ids::effective_vision_model_id(config);
        if config.local_ai.preload_vision_model {
            self.ensure_ollama_model_available(&vision_model, "vision")
                .await?;
            self.status.lock().vision_state = "ready".to_string();
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

        let started_at = std::time::Instant::now();
        let response = self
            .http
            .post(format!("{OLLAMA_BASE_URL}/api/pull"))
            .json(&OllamaPullRequest {
                name: model_id.to_string(),
                stream: true,
            })
            .send()
            .await
            .map_err(|e| format!("ollama pull request failed: {e}"))?;
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
        while let Some(item) = stream.next().await {
            let chunk = item.map_err(|e| format!("ollama pull stream error: {e}"))?;
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

                let completed = event.completed.unwrap_or(0);
                let total = event.total;
                let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
                let speed_bps = (completed as f64 / elapsed).round().max(0.0) as u64;
                let eta_seconds = total.and_then(|t| {
                    if completed >= t || speed_bps == 0 {
                        None
                    } else {
                        Some((t.saturating_sub(completed)) / speed_bps.max(1))
                    }
                });

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

        if !self.has_model(model_id).await? {
            return Err(format!(
                "ollama pull finished but model `{}` was not found",
                model_id
            ));
        }

        match label {
            "vision" => self.status.lock().vision_state = "ready".to_string(),
            "embedding" => self.status.lock().embedding_state = "ready".to_string(),
            _ => {}
        }

        Ok(())
    }

    /// Run full diagnostics: check Ollama server health, list installed models,
    /// and verify expected models are present. Returns a JSON-serializable report.
    pub async fn diagnostics(
        &self,
        config: &Config,
    ) -> Result<serde_json::Value, String> {
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
            model_names.iter().any(|n| *n == t || n.starts_with(&(t.clone() + ":")))
        };

        let chat_found = has(&expected_chat);
        let embedding_found = has(&expected_embedding);
        let vision_found = has(&expected_vision);

        let binary_path = self.resolve_binary_path(config);

        let mut issues: Vec<String> = Vec::new();
        if !healthy {
            issues.push("Ollama server is not running or not reachable at http://127.0.0.1:11434".to_string());
        }
        if healthy && !chat_found {
            issues.push(format!("Chat model `{}` is not installed", expected_chat));
        }
        if healthy && config.local_ai.preload_embedding_model && !embedding_found {
            issues.push(format!("Embedding model `{}` is not installed", expected_embedding));
        }
        if healthy && config.local_ai.preload_vision_model && !vision_found {
            issues.push(format!("Vision model `{}` is not installed", expected_vision));
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
        let response = self
            .http
            .get(format!("{OLLAMA_BASE_URL}/api/tags"))
            .timeout(std::time::Duration::from_secs(5))
            .send()
            .await
            .map_err(|e| format!("ollama tags request failed: {e}"))?;
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(format!("ollama tags failed with status {}: {}", status, body.trim()));
        }
        let payload: OllamaTagsResponse = response
            .json()
            .await
            .map_err(|e| format!("ollama tags parse failed: {e}"))?;
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

    pub(in crate::openhuman::local_ai::service) async fn has_model(
        &self,
        model: &str,
    ) -> Result<bool, String> {
        let response = self
            .http
            .get(format!("{OLLAMA_BASE_URL}/api/tags"))
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
