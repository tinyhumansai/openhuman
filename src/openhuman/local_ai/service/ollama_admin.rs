use std::path::{Path, PathBuf};

use futures_util::StreamExt;

use crate::openhuman::config::Config;
use crate::openhuman::local_ai::install::{find_system_ollama_binary, run_ollama_install_script};
use crate::openhuman::local_ai::model_ids;
use crate::openhuman::local_ai::ollama_api::{
    OllamaPullEvent, OllamaPullRequest, OllamaTagsResponse, OLLAMA_BASE_URL,
};
use crate::openhuman::local_ai::paths::{
    resolve_stt_model_path, resolve_tts_voice_path, workspace_ollama_binary,
};

use super::LocalAiService;

impl LocalAiService {
    pub(super) async fn ensure_ollama_server(&self, config: &Config) -> Result<(), String> {
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

    async fn resolve_or_install_ollama_binary(&self, config: &Config) -> Result<PathBuf, String> {
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
            status.state = "downloading".to_string();
            status.warning = Some("Installing Ollama runtime (first run)".to_string());
            status.download_progress = None;
            status.downloaded_bytes = None;
            status.total_bytes = None;
            status.download_speed_bps = None;
            status.eta_seconds = None;
        }

        let install_status = run_ollama_install_script().await?;
        if !install_status.success() {
            return Err("Ollama install script failed".to_string());
        }

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

    pub(super) async fn ensure_models_available(&self, config: &Config) -> Result<(), String> {
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
            self.status.lock().stt_state = if resolve_stt_model_path(config).is_ok() {
                "ready".to_string()
            } else {
                "degraded".to_string()
            };
        }

        if config.local_ai.preload_tts_voice {
            self.status.lock().tts_state = if resolve_tts_voice_path(config).is_ok() {
                "ready".to_string()
            } else {
                "degraded".to_string()
            };
        }

        Ok(())
    }

    pub(super) async fn ensure_ollama_model_available(
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

    async fn has_model(&self, model: &str) -> Result<bool, String> {
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
