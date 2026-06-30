use super::BrowserAction;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::time::{timeout, Duration};

const RUNNER_JS: &str = include_str!("playwright_runner.mjs");
const PLAYWRIGHT_PROBE_TIMEOUT: Duration = Duration::from_secs(20);
const PLAYWRIGHT_RESPONSE_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Default)]
pub struct PlaywrightBrowserState {
    daemon: Option<PlaywrightDaemon>,
    next_id: u64,
}

struct PlaywrightDaemon {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

#[derive(Debug, Deserialize)]
struct PlaywrightResponse {
    #[serde(default)]
    id: Option<u64>,
    success: bool,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct BrowserUrlPolicy {
    pub(crate) allowed_domains: Vec<String>,
    pub(crate) allow_all: bool,
}

impl Drop for PlaywrightDaemon {
    fn drop(&mut self) {
        tracing::debug!("[browser::playwright] dropping backend daemon");
        let _ = self.child.start_kill();
    }
}

impl PlaywrightBrowserState {
    pub async fn is_available() -> bool {
        tracing::debug!("[browser::playwright] probing playwright runtime availability");
        let mut command = node_command();
        command
            .args([
                "-e",
                "const load=()=>{try{return require('playwright')}catch(_){return require('@playwright/test')}};(async()=>{const {chromium}=load();const browser=await chromium.launch({headless:true});await browser.close();})().then(()=>process.exit(0)).catch(()=>process.exit(1));",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        apply_node_cwd(&mut command);

        match timeout(PLAYWRIGHT_PROBE_TIMEOUT, command.status()).await {
            Ok(Ok(status)) => {
                let available = status.success();
                tracing::debug!(
                    available,
                    status = ?status.code(),
                    "[browser::playwright] runtime availability probe finished"
                );
                available
            }
            Ok(Err(error)) => {
                tracing::debug!(
                    error = %error,
                    "[browser::playwright] runtime availability probe failed to start"
                );
                false
            }
            Err(_) => {
                tracing::debug!(
                    timeout_ms = PLAYWRIGHT_PROBE_TIMEOUT.as_millis() as u64,
                    "[browser::playwright] runtime availability probe timed out"
                );
                false
            }
        }
    }

    pub async fn execute_action(
        &mut self,
        action: BrowserAction,
        headless: bool,
        url_policy: Option<BrowserUrlPolicy>,
    ) -> Result<Value> {
        tracing::trace!("[browser::playwright] preparing action request");
        let args = action_to_args(action, url_policy);
        self.execute_args(args, headless).await
    }

    async fn execute_args(&mut self, args: Value, headless: bool) -> Result<Value> {
        if self.daemon.is_none() {
            tracing::debug!("[browser::playwright] starting playwright backend daemon");
            self.daemon = Some(start_daemon(headless).await?);
        }

        let id = self.next_id;
        self.next_id = self.next_id.saturating_add(1);
        let action = args
            .get("action")
            .and_then(Value::as_str)
            .unwrap_or("<unknown>")
            .to_string();

        let request = json!({
            "id": id,
            "args": args,
        });
        let line = serde_json::to_vec(&request).context("Failed to encode Playwright request")?;
        tracing::debug!(
            request_id = id,
            action = %action,
            "[browser::playwright] dispatching action"
        );

        let daemon = self.daemon.as_mut().expect("daemon just initialized");
        if let Err(error) = write_request(daemon, &line).await {
            tracing::debug!(
                error = %error,
                request_id = id,
                "[browser::playwright] daemon write failed; restarting once"
            );
            self.daemon = Some(start_daemon(headless).await?);
            let daemon = self.daemon.as_mut().expect("daemon restarted");
            if let Err(error) = write_request(daemon, &line).await {
                tracing::debug!(
                    error = %error,
                    request_id = id,
                    "[browser::playwright] daemon write retry failed; dropping daemon"
                );
                self.daemon = None;
                return Err(error);
            }
        }

        let daemon = self.daemon.as_mut().expect("daemon available");
        let response = match timeout(PLAYWRIGHT_RESPONSE_TIMEOUT, read_response(daemon)).await {
            Ok(Ok(response)) => response,
            Ok(Err(error)) => {
                tracing::debug!(
                    error = %error,
                    request_id = id,
                    "[browser::playwright] daemon read failed; dropping daemon"
                );
                self.daemon = None;
                return Err(error).context("Failed to read Playwright response");
            }
            Err(_) => {
                tracing::debug!(
                    request_id = id,
                    timeout_ms = PLAYWRIGHT_RESPONSE_TIMEOUT.as_millis() as u64,
                    "[browser::playwright] daemon response timed out; dropping daemon"
                );
                self.daemon = None;
                anyhow::bail!("Timed out waiting for Playwright response");
            }
        };

        if response.id != Some(id) {
            tracing::debug!(
                expected_id = id,
                response_id = ?response.id,
                "[browser::playwright] daemon response id mismatch; dropping daemon"
            );
            self.daemon = None;
            anyhow::bail!("Playwright daemon response id mismatch");
        }

        if response.success {
            tracing::debug!(
                request_id = id,
                action = %action,
                "[browser::playwright] action completed"
            );
            Ok(response.data.unwrap_or_else(|| json!({ "ok": true })))
        } else {
            tracing::debug!(
                request_id = id,
                action = %action,
                error = ?response.error,
                "[browser::playwright] action failed"
            );
            anyhow::bail!(
                "{}",
                response
                    .error
                    .unwrap_or_else(|| "Playwright backend failed".to_string())
            )
        }
    }
}

async fn write_request(daemon: &mut PlaywrightDaemon, line: &[u8]) -> Result<()> {
    tracing::trace!("[browser::playwright] writing request to daemon");
    daemon
        .stdin
        .write_all(line)
        .await
        .context("Failed to write Playwright request")?;
    daemon
        .stdin
        .write_all(b"\n")
        .await
        .context("Failed to terminate Playwright request")?;
    daemon
        .stdin
        .flush()
        .await
        .context("Failed to flush Playwright request")?;
    Ok(())
}

async fn read_response(daemon: &mut PlaywrightDaemon) -> Result<PlaywrightResponse> {
    tracing::trace!("[browser::playwright] reading response from daemon");
    let mut line = String::new();
    let read = daemon
        .stdout
        .read_line(&mut line)
        .await
        .context("Failed to read Playwright stdout")?;
    if read == 0 {
        anyhow::bail!("Playwright daemon exited without a response");
    }
    tracing::trace!("[browser::playwright] received response from daemon");
    serde_json::from_str(&line).context("Playwright daemon returned invalid JSON")
}

async fn start_daemon(headless: bool) -> Result<PlaywrightDaemon> {
    tracing::debug!(
        headless,
        "[browser::playwright] spawning playwright backend daemon"
    );
    let mut command = node_command();
    command
        .arg("-e")
        .arg(RUNNER_JS)
        .env(
            "OPENHUMAN_PLAYWRIGHT_HEADLESS",
            if headless { "1" } else { "0" },
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    apply_node_cwd(&mut command);

    let mut child = command
        .spawn()
        .map_err(|error| {
            tracing::debug!(
                error = %error,
                "[browser::playwright] failed to spawn backend daemon"
            );
            error
        })
        .context(
            "Failed to start Playwright backend. Ensure Node.js and the Playwright package are installed.",
        )?;
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| anyhow::anyhow!("Playwright daemon stdin unavailable"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("Playwright daemon stdout unavailable"))?;

    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(async move {
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!("[browser::playwright] stderr: {line}");
            }
        });
    }

    tracing::debug!("[browser::playwright] backend daemon spawned");
    Ok(PlaywrightDaemon {
        child,
        stdin,
        stdout: BufReader::new(stdout),
    })
}

fn node_command() -> Command {
    let binary = std::env::var("OPENHUMAN_PLAYWRIGHT_NODE").unwrap_or_else(|_| "node".to_string());
    Command::new(binary)
}

fn apply_node_cwd(command: &mut Command) {
    if let Some(cwd) = playwright_node_cwd() {
        command.current_dir(cwd);
    }
}

fn playwright_node_cwd() -> Option<PathBuf> {
    if let Ok(raw) = std::env::var("OPENHUMAN_PLAYWRIGHT_CWD") {
        let path = PathBuf::from(raw);
        if path.exists() {
            return Some(path);
        }
    }

    let app = Path::new("app");
    if app.join("node_modules").exists() {
        return Some(app.to_path_buf());
    }

    None
}

fn action_to_args(action: BrowserAction, url_policy: Option<BrowserUrlPolicy>) -> Value {
    match action {
        BrowserAction::Open { url } => {
            let policy = url_policy.expect("playwright open actions require a URL policy");
            json!({
                "action": "open",
                "url": url,
                "url_policy": {
                    "allowed_domains": policy.allowed_domains,
                    "allow_all": policy.allow_all,
                },
            })
        }
        BrowserAction::Snapshot {
            interactive_only,
            compact,
            depth,
        } => json!({
            "action": "snapshot",
            "interactive_only": interactive_only,
            "compact": compact,
            "depth": depth,
        }),
        BrowserAction::Click { selector } => json!({ "action": "click", "selector": selector }),
        BrowserAction::Fill { selector, value } => {
            json!({ "action": "fill", "selector": selector, "value": value })
        }
        BrowserAction::Type { selector, text } => {
            json!({ "action": "type", "selector": selector, "text": text })
        }
        BrowserAction::GetText { selector } => {
            json!({ "action": "get_text", "selector": selector })
        }
        BrowserAction::GetTitle => json!({ "action": "get_title" }),
        BrowserAction::GetUrl => json!({ "action": "get_url" }),
        BrowserAction::Screenshot { path, full_page } => {
            json!({ "action": "screenshot", "path": path, "full_page": full_page })
        }
        BrowserAction::Wait { selector, ms, text } => {
            json!({ "action": "wait", "selector": selector, "ms": ms, "text": text })
        }
        BrowserAction::Press { key } => json!({ "action": "press", "key": key }),
        BrowserAction::Hover { selector } => json!({ "action": "hover", "selector": selector }),
        BrowserAction::Scroll { direction, pixels } => {
            json!({ "action": "scroll", "direction": direction, "pixels": pixels })
        }
        BrowserAction::IsVisible { selector } => {
            json!({ "action": "is_visible", "selector": selector })
        }
        BrowserAction::Close => json!({ "action": "close" }),
        BrowserAction::Find {
            by,
            value,
            action,
            fill_value,
        } => json!({
            "action": "find",
            "by": by,
            "value": value,
            "find_action": action,
            "fill_value": fill_value,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn action_to_args_preserves_find_shape() {
        let args = action_to_args(
            BrowserAction::Find {
                by: "label".into(),
                value: "Email".into(),
                action: "fill".into(),
                fill_value: Some("a@example.com".into()),
            },
            None,
        );

        assert_eq!(args["action"], "find");
        assert_eq!(args["find_action"], "fill");
        assert_eq!(args["fill_value"], "a@example.com");
    }
}
