use std::path::Path;

use crate::openhuman::config::Config;
use crate::openhuman::inference::local::ollama::ollama_base_url_from_config;
use crate::openhuman::inference::local::process_util::apply_no_window;

use super::super::spawn_marker::{self, OllamaSpawnMarker};
use super::super::LocalAiService;
use super::util::kill_pid_by_id;

impl LocalAiService {
    pub(in crate::openhuman::inference::local::service) async fn ensure_ollama_server(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        let base_url = ollama_base_url_from_config(config);
        if self.ollama_healthy_at(&base_url).await {
            if self.ollama_runner_ok_at(&base_url).await {
                return Ok(());
            }
            log::warn!("[local_ai] Ollama server responds but runner is broken");
            return Err(
                "Configured Ollama runtime is reachable but cannot execute models. Restart the external runtime and retry."
                    .to_string(),
            );
        }
        Err(format!(
            "OpenHuman no longer starts or installs Ollama automatically. Start your inference runtime yourself and make sure it is reachable at {base_url}."
        ))
    }

    /// Alias of `ensure_ollama_server` in external-runtime mode.
    /// OpenHuman no longer installs or starts Ollama automatically; the
    /// "fresh" retry path is a no-op that defers to the standard check.
    pub(in crate::openhuman::inference::local::service) async fn ensure_ollama_server_fresh(
        &self,
        config: &Config,
    ) -> Result<(), String> {
        self.ensure_ollama_server(config).await
    }

    /// Check if a healthy daemon on `:11434` is actually openhuman's own
    /// orphan from a prior session (i.e. we crashed before the graceful
    /// shutdown hook fired). If so, kill it so the upcoming spawn can
    /// resume owned-child tracking. External daemons are never touched.
    pub(in crate::openhuman::inference::local::service) async fn reclaim_orphan_if_ours(
        &self,
        config: &Config,
    ) {
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
        let base_url = ollama_base_url_from_config(config);
        if !self.ollama_healthy_at(&base_url).await {
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

    pub(in crate::openhuman::inference::local::service) async fn start_and_wait_for_server(
        &self,
        config: &Config,
        ollama_cmd: &Path,
    ) -> Result<(), String> {
        let base_url = ollama_base_url_from_config(config);
        if self.ollama_healthy_at(&base_url).await {
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
            if self.ollama_healthy_at(&base_url).await {
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
}
