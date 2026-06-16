use std::path::PathBuf;

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::install::find_system_ollama_binary;
use crate::openhuman::inference::local::ollama::{ollama_base_url, ollama_base_url_from_config};
use crate::openhuman::inference::paths::{find_workspace_ollama_binary, workspace_ollama_binary};

use super::super::spawn_marker;
use super::super::LocalAiService;

impl LocalAiService {
    /// Check Ollama health against the given base URL.
    pub(in crate::openhuman::inference::local::service) async fn ollama_healthy_at(
        &self,
        base_url: &str,
    ) -> bool {
        tracing::debug!(
            target: "local_ai::ollama_admin",
            %base_url,
            "[local_ai:ollama_admin] ollama_healthy_at: checking"
        );
        self.http
            .get(format!("{base_url}/api/tags"))
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .map(|r| r.status().is_success())
            .unwrap_or(false)
    }

    /// Backward-compat wrapper — resolves the URL from env vars only (no config).
    /// Prefer [`ollama_healthy_at`] when a `Config` is available.
    pub(in crate::openhuman::inference::local::service) async fn ollama_healthy(&self) -> bool {
        self.ollama_healthy_at(&ollama_base_url()).await
    }

    /// Filesystem-only precondition: is *any* Ollama binary discoverable?
    ///
    /// This is the cheapest possible check — no process spawns, no HTTP, no
    /// timeouts. Callers that need to decide whether it's even worth talking
    /// to `/api/tags` should consult this first. Returning `false` here means
    /// the UI should drive the user to install Ollama instead of polling for
    /// model state that can never appear.
    pub(in crate::openhuman::inference::local::service) fn ollama_binary_present(
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

    /// Quick check that the Ollama runner can actually exec models against the given URL.
    pub(in crate::openhuman::inference::local::service) async fn ollama_runner_ok_at(
        &self,
        base_url: &str,
    ) -> bool {
        let resp = self
            .http
            .get(format!("{base_url}/api/tags"))
            .timeout(std::time::Duration::from_secs(3))
            .send()
            .await;
        match resp {
            Ok(r) if r.status().is_success() => {
                // Tags endpoint works — but the runner error only shows up on model exec.
                // Do a lightweight pull-status check (won't download, just checks).
                let check = self
                    .http
                    .post(format!("{base_url}/api/show"))
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
    pub(in crate::openhuman::inference::local::service) async fn kill_ollama_server(&self) {
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

    pub(in crate::openhuman::inference::local::service) fn resolve_binary_path(
        &self,
        config: &Config,
    ) -> Option<String> {
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
        crate::openhuman::inference::local::install::find_system_ollama_binary()
            .map(|p| p.display().to_string())
    }

    pub(in crate::openhuman::inference::local::service) async fn has_model(
        &self,
        model: &str,
    ) -> Result<bool, String> {
        self.has_model_at(&ollama_base_url(), model).await
    }

    pub(in crate::openhuman::inference::local::service) async fn has_model_for_config(
        &self,
        config: &Config,
        model: &str,
    ) -> Result<bool, String> {
        self.has_model_at(&ollama_base_url_from_config(config), model)
            .await
    }

    pub(in crate::openhuman::inference::local::service) async fn has_model_at(
        &self,
        base_url: &str,
        model: &str,
    ) -> Result<bool, String> {
        use crate::openhuman::inference::local::ollama::OllamaTagsResponse;
        // Issue the /api/tags GET directly. We previously short-circuited via
        // ollama_healthy(), but that doubled the number of /api/tags round-trips
        // on healthy polls (one probe + one tags fetch). With three has_model()
        // calls per assets_status poll (chat, vision, embedding) that was 6
        // network calls instead of 3. The 500ms connect_timeout on the shared
        // reqwest client (set in bootstrap.rs) bounds the cost when the server
        // is down — the connect failure surfaces as Err, same as ollama_healthy()
        // would have surfaced as `false`.
        log::debug!("[local_ai] has_model_at: checking for model `{model}` at {base_url}");
        let response = self
            .http
            .get(format!("{base_url}/api/tags"))
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
