use super::{kill_shared, new_shared_process, SharedProcess, Tunnel, TunnelProcess};
use anyhow::{bail, Result};
use tokio::io::AsyncBufReadExt;
use tokio::process::Command;

/// ngrok Tunnel — wraps the `ngrok` binary.
///
/// Requires `ngrok` installed. Optionally set a custom domain
/// (requires ngrok paid plan).
pub struct NgrokTunnel {
    auth_token: String,
    domain: Option<String>,
    proc: SharedProcess,
}

impl NgrokTunnel {
    pub fn new(auth_token: String, domain: Option<String>) -> Self {
        Self {
            auth_token,
            domain,
            proc: new_shared_process(),
        }
    }
}

#[async_trait::async_trait]
impl Tunnel for NgrokTunnel {
    fn name(&self) -> &str {
        "ngrok"
    }

    async fn start(&self, _local_host: &str, local_port: u16) -> Result<String> {
        // Set auth token
        Command::new("ngrok")
            .args(["config", "add-authtoken", &self.auth_token])
            .output()
            .await?;

        // Build command: ngrok http <port> [--domain <domain>]
        let mut args = vec!["http".to_string(), local_port.to_string()];
        if let Some(ref domain) = self.domain {
            args.push("--domain".into());
            args.push(domain.clone());
        }
        // Output log to stdout for URL extraction
        args.push("--log".into());
        args.push("stdout".into());
        args.push("--log-format".into());
        args.push("logfmt".into());

        let mut child = Command::new("ngrok")
            .args(&args)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| anyhow::anyhow!("Failed to capture ngrok stdout"))?;

        let mut reader = tokio::io::BufReader::new(stdout).lines();
        let mut public_url = String::new();

        // Wait up to 15s for the tunnel URL
        let deadline = tokio::time::Instant::now() + tokio::time::Duration::from_secs(15);
        while tokio::time::Instant::now() < deadline {
            let line =
                tokio::time::timeout(tokio::time::Duration::from_secs(3), reader.next_line()).await;

            match line {
                Ok(Ok(Some(l))) => {
                    tracing::debug!("ngrok: {l}");
                    // ngrok logfmt: url=https://xxxx.ngrok-free.app
                    if let Some(idx) = l.find("url=https://") {
                        let url_start = idx + 4; // skip "url="
                        let url_part = &l[url_start..];
                        let end = url_part
                            .find(|c: char| c.is_whitespace())
                            .unwrap_or(url_part.len());
                        public_url = url_part[..end].to_string();
                        break;
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(e)) => bail!("Error reading ngrok output: {e}"),
                Err(_) => {}
            }
        }

        if public_url.is_empty() {
            child.kill().await.ok();
            bail!("ngrok did not produce a public URL within 15s. Is the auth token valid?");
        }

        let mut guard = self.proc.lock().await;
        *guard = Some(TunnelProcess {
            child,
            public_url: public_url.clone(),
        });

        Ok(public_url)
    }

    async fn stop(&self) -> Result<()> {
        kill_shared(&self.proc).await
    }

    async fn health_check(&self) -> bool {
        let guard = self.proc.lock().await;
        guard.as_ref().is_some_and(|tp| tp.child.id().is_some())
    }

    fn public_url(&self) -> Option<String> {
        self.proc
            .try_lock()
            .ok()
            .and_then(|g| g.as_ref().map(|tp| tp.public_url.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constructor_stores_domain() {
        let tunnel = NgrokTunnel::new("ngrok-token".into(), Some("my.ngrok.app".into()));
        assert_eq!(tunnel.domain.as_deref(), Some("my.ngrok.app"));
    }

    #[test]
    fn public_url_is_none_before_start() {
        let tunnel = NgrokTunnel::new("ngrok-token".into(), None);
        assert!(tunnel.public_url().is_none());
    }

    #[tokio::test]
    async fn stop_without_started_process_is_ok() {
        let tunnel = NgrokTunnel::new("ngrok-token".into(), None);
        let result = tunnel.stop().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn health_check_is_false_before_start() {
        let tunnel = NgrokTunnel::new("ngrok-token".into(), None);
        assert!(!tunnel.health_check().await);
    }
}
