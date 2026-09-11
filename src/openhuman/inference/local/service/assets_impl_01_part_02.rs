
impl LocalAiService {
    pub(in crate::openhuman::inference::local::service) async fn ensure_tts_asset_available(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        if resolve_tts_voice_path(config).is_ok() {
            self.status.lock().tts_state = "ready".to_string();
            return Ok(());
        }

        let url = config
            .local_ai
            .tts_download_url
            .as_deref()
            .filter(|v| !v.trim().is_empty())
            .ok_or_else(|| {
                "TTS voice missing and no local_ai.tts_download_url configured".to_string()
            })?;
        let dest = tts_model_target_path(config);
        self.download_file_with_progress(url, &dest, "tts").await?;

        if let Some(config_url) = config
            .local_ai
            .tts_config_download_url
            .as_deref()
            .filter(|v| !v.trim().is_empty())
        {
            let config_dest = std::path::PathBuf::from(format!("{}.json", dest.display()));
            let _ = self
                .download_file_with_progress(config_url, &config_dest, "tts-config")
                .await;
        }

        self.status.lock().tts_state = "ready".to_string();
        Ok(())
    }

    async fn download_file_with_progress(
        &self,
        url: &str,
        dest: &Path,
        label: &str,
    ) -> Result<(), String> {
        if let Some(parent) = dest.parent() {
            tokio::fs::create_dir_all(parent)
                .await
                .map_err(|e| format!("failed to create destination directory: {e}"))?;
        }

        let response = self
            .http
            .get(url)
            // Large model assets (STT/TTS) can take minutes on slower links.
            // Avoid inheriting the short default client timeout for these streams.
            .timeout(std::time::Duration::from_secs(30 * 60))
            .send()
            .await
            .map_err(|e| format!("failed to start {label} download: {e}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "failed to download {label} asset, status {}",
                response.status()
            ));
        }

        {
            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!("Downloading {label} asset"));
            match label {
                "stt" => status.stt_state = "downloading".to_string(),
                "tts" | "tts-config" => status.tts_state = "downloading".to_string(),
                _ => {}
            }
            status.download_progress = Some(0.0);
            status.downloaded_bytes = Some(0);
            status.total_bytes = response.content_length();
            status.download_speed_bps = Some(0);
            status.eta_seconds = None;
        }

        let total = response.content_length();
        let mut downloaded: u64 = 0;
        let started_at = std::time::Instant::now();
        let mut file = tokio::fs::File::create(dest)
            .await
            .map_err(|e| format!("failed to create destination file: {e}"))?;
        let mut stream = response.bytes_stream();
        while let Some(chunk) = stream
            .try_next()
            .await
            .map_err(|e| format!("download stream error for {label}: {e}"))?
        {
            use tokio::io::AsyncWriteExt;
            file.write_all(&chunk)
                .await
                .map_err(|e| format!("failed writing {label} file: {e}"))?;
            downloaded = downloaded.saturating_add(chunk.len() as u64);
            let elapsed = started_at.elapsed().as_secs_f64().max(0.001);
            let speed_bps = (downloaded as f64 / elapsed).round().max(0.0) as u64;
            let eta_seconds = total.and_then(|t| {
                if downloaded >= t || speed_bps == 0 {
                    None
                } else {
                    Some((t.saturating_sub(downloaded)) / speed_bps.max(1))
                }
            });

            let mut status = self.status.lock();
            status.state = "downloading".to_string();
            status.warning = Some(format!("Downloading {label} asset"));
            match label {
                "stt" => status.stt_state = "downloading".to_string(),
                "tts" | "tts-config" => status.tts_state = "downloading".to_string(),
                _ => {}
            }
            status.downloaded_bytes = Some(downloaded);
            status.total_bytes = total;
            status.download_speed_bps = Some(speed_bps);
            status.eta_seconds = eta_seconds;
            status.download_progress = total
                .map(|t| (downloaded as f32 / t as f32).clamp(0.0, 1.0))
                .or(Some(0.0));
        }

        Ok(())
    }
}
