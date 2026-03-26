//! Browser automation tool with pluggable backends.
//!
//! By default this uses Vercel's `agent-browser` tool for automation.
//! Optionally, a Rust-native backend can be enabled at build time via
//! `--features browser-native` and selected through config.
//! Computer-use (OS-level) actions are supported via an optional sidecar endpoint.

use super::traits::{Tool, ToolResult};
use crate::openhuman::security::SecurityPolicy;
use anyhow::Context;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::net::ToSocketAddrs;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;
use tokio::process::Command;
use tracing::debug;

/// Computer-use sidecar settings.
#[derive(Clone)]
pub struct ComputerUseConfig {
    pub endpoint: String,
    pub api_key: Option<String>,
    pub timeout_ms: u64,
    pub allow_remote_endpoint: bool,
    pub window_allowlist: Vec<String>,
    pub max_coordinate_x: Option<i64>,
    pub max_coordinate_y: Option<i64>,
}

impl std::fmt::Debug for ComputerUseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ComputerUseConfig")
            .field("endpoint", &self.endpoint)
            .field("api_key", &self.api_key.as_ref().map(|_| "[REDACTED]"))
            .field("timeout_ms", &self.timeout_ms)
            .field("allow_remote_endpoint", &self.allow_remote_endpoint)
            .field("window_allowlist", &self.window_allowlist)
            .field("max_coordinate_x", &self.max_coordinate_x)
            .field("max_coordinate_y", &self.max_coordinate_y)
            .finish()
    }
}

impl Default for ComputerUseConfig {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:8787/v1/actions".into(),
            api_key: None,
            timeout_ms: 15_000,
            allow_remote_endpoint: false,
            window_allowlist: Vec::new(),
            max_coordinate_x: None,
            max_coordinate_y: None,
        }
    }
}

/// Browser automation tool using pluggable backends.
pub struct BrowserTool {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    session_name: Option<String>,
    backend: String,
    native_headless: bool,
    native_webdriver_url: String,
    native_chrome_path: Option<String>,
    computer_use: ComputerUseConfig,
    #[cfg(feature = "browser-native")]
    native_state: tokio::sync::Mutex<native_backend::NativeBrowserState>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserBackendKind {
    AgentBrowser,
    RustNative,
    ComputerUse,
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResolvedBackend {
    AgentBrowser,
    RustNative,
    ComputerUse,
}

impl BrowserBackendKind {
    fn parse(raw: &str) -> anyhow::Result<Self> {
        let key = raw.trim().to_ascii_lowercase().replace('-', "_");
        match key.as_str() {
            "agent_browser" | "agentbrowser" => Ok(Self::AgentBrowser),
            "rust_native" | "native" => Ok(Self::RustNative),
            "computer_use" | "computeruse" => Ok(Self::ComputerUse),
            "auto" => Ok(Self::Auto),
            _ => anyhow::bail!(
                "Unsupported browser backend '{raw}'. Use 'agent_browser', 'rust_native', 'computer_use', or 'auto'"
            ),
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::AgentBrowser => "agent_browser",
            Self::RustNative => "rust_native",
            Self::ComputerUse => "computer_use",
            Self::Auto => "auto",
        }
    }
}

/// Response from agent-browser --json commands
#[derive(Debug, Deserialize)]
struct AgentBrowserResponse {
    success: bool,
    data: Option<Value>,
    error: Option<String>,
}

/// Response format from computer-use sidecar.
#[derive(Debug, Deserialize)]
struct ComputerUseResponse {
    #[serde(default)]
    success: Option<bool>,
    #[serde(default)]
    data: Option<Value>,
    #[serde(default)]
    error: Option<String>,
}

/// Supported browser actions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BrowserAction {
    /// Navigate to a URL
    Open { url: String },
    /// Get accessibility snapshot with refs
    Snapshot {
        #[serde(default)]
        interactive_only: bool,
        #[serde(default)]
        compact: bool,
        #[serde(default)]
        depth: Option<u32>,
    },
    /// Click an element by ref or selector
    Click { selector: String },
    /// Fill a form field
    Fill { selector: String, value: String },
    /// Type text into focused element
    Type { selector: String, text: String },
    /// Get text content of element
    GetText { selector: String },
    /// Get page title
    GetTitle,
    /// Get current URL
    GetUrl,
    /// Take screenshot
    Screenshot {
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        full_page: bool,
    },
    /// Wait for element or time
    Wait {
        #[serde(default)]
        selector: Option<String>,
        #[serde(default)]
        ms: Option<u64>,
        #[serde(default)]
        text: Option<String>,
    },
    /// Press a key
    Press { key: String },
    /// Hover over element
    Hover { selector: String },
    /// Scroll page
    Scroll {
        direction: String,
        #[serde(default)]
        pixels: Option<u32>,
    },
    /// Check if element is visible
    IsVisible { selector: String },
    /// Close browser
    Close,
    /// Find element by semantic locator
    Find {
        by: String, // role, text, label, placeholder, testid
        value: String,
        action: String, // click, fill, text, hover
        #[serde(default)]
        fill_value: Option<String>,
    },
}

impl BrowserTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        allowed_domains: Vec<String>,
        session_name: Option<String>,
    ) -> Self {
        Self::new_with_backend(
            security,
            allowed_domains,
            session_name,
            "agent_browser".into(),
            true,
            "http://127.0.0.1:9515".into(),
            None,
            ComputerUseConfig::default(),
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn new_with_backend(
        security: Arc<SecurityPolicy>,
        allowed_domains: Vec<String>,
        session_name: Option<String>,
        backend: String,
        native_headless: bool,
        native_webdriver_url: String,
        native_chrome_path: Option<String>,
        computer_use: ComputerUseConfig,
    ) -> Self {
        Self {
            security,
            allowed_domains: normalize_domains(allowed_domains),
            session_name,
            backend,
            native_headless,
            native_webdriver_url,
            native_chrome_path,
            computer_use,
            #[cfg(feature = "browser-native")]
            native_state: tokio::sync::Mutex::new(native_backend::NativeBrowserState::default()),
        }
    }

    /// Check if agent-browser tool is available
    pub async fn is_agent_browser_available() -> bool {
        Command::new("agent-browser")
            .arg("--version")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .await
            .map(|s| s.success())
            .unwrap_or(false)
    }

    /// Backward-compatible alias.
    pub async fn is_available() -> bool {
        Self::is_agent_browser_available().await
    }

    fn configured_backend(&self) -> anyhow::Result<BrowserBackendKind> {
        BrowserBackendKind::parse(&self.backend)
    }

    fn rust_native_compiled() -> bool {
        cfg!(feature = "browser-native")
    }

    fn rust_native_available(&self) -> bool {
        #[cfg(feature = "browser-native")]
        {
            native_backend::NativeBrowserState::is_available(
                self.native_headless,
                &self.native_webdriver_url,
                self.native_chrome_path.as_deref(),
            )
        }
        #[cfg(not(feature = "browser-native"))]
        {
            false
        }
    }

    fn computer_use_endpoint_url(&self) -> anyhow::Result<reqwest::Url> {
        if self.computer_use.timeout_ms == 0 {
            anyhow::bail!("browser.computer_use.timeout_ms must be > 0");
        }

        let endpoint = self.computer_use.endpoint.trim();
        if endpoint.is_empty() {
            anyhow::bail!("browser.computer_use.endpoint cannot be empty");
        }

        let parsed = reqwest::Url::parse(endpoint).map_err(|_| {
            anyhow::anyhow!(
                "Invalid browser.computer_use.endpoint: '{endpoint}'. Expected http(s) URL"
            )
        })?;

        let scheme = parsed.scheme();
        if scheme != "http" && scheme != "https" {
            anyhow::bail!("browser.computer_use.endpoint must use http:// or https://");
        }

        let host = parsed
            .host_str()
            .ok_or_else(|| anyhow::anyhow!("browser.computer_use.endpoint must include host"))?;

        let host_is_private = is_private_host(host);
        if !self.computer_use.allow_remote_endpoint && !host_is_private {
            anyhow::bail!(
                "browser.computer_use.endpoint host '{host}' is public. Set browser.computer_use.allow_remote_endpoint=true to allow it"
            );
        }

        if self.computer_use.allow_remote_endpoint && !host_is_private && scheme != "https" {
            anyhow::bail!(
                "browser.computer_use.endpoint must use https:// when allow_remote_endpoint=true and host is public"
            );
        }

        Ok(parsed)
    }

    fn computer_use_available(&self) -> anyhow::Result<bool> {
        let endpoint = self.computer_use_endpoint_url()?;
        Ok(endpoint_reachable(&endpoint, Duration::from_millis(500)))
    }

    async fn resolve_backend(&self) -> anyhow::Result<ResolvedBackend> {
        let configured = self.configured_backend()?;

        match configured {
            BrowserBackendKind::AgentBrowser => {
                if Self::is_agent_browser_available().await {
                    Ok(ResolvedBackend::AgentBrowser)
                } else {
                    anyhow::bail!(
                        "browser.backend='{}' but agent-browser is unavailable. Install it in your environment.",
                        configured.as_str()
                    )
                }
            }
            BrowserBackendKind::RustNative => {
                if !Self::rust_native_compiled() {
                    anyhow::bail!(
                        "browser.backend='rust_native' requires build feature 'browser-native'"
                    );
                }
                if !self.rust_native_available() {
                    anyhow::bail!(
                        "Rust-native browser backend is enabled but WebDriver endpoint is unreachable. Set browser.native_webdriver_url and start a compatible driver"
                    );
                }
                Ok(ResolvedBackend::RustNative)
            }
            BrowserBackendKind::ComputerUse => {
                if !self.computer_use_available()? {
                    anyhow::bail!(
                        "browser.backend='computer_use' but sidecar endpoint is unreachable. Check browser.computer_use.endpoint and sidecar status"
                    );
                }
                Ok(ResolvedBackend::ComputerUse)
            }
            BrowserBackendKind::Auto => {
                if Self::rust_native_compiled() && self.rust_native_available() {
                    return Ok(ResolvedBackend::RustNative);
                }
                if Self::is_agent_browser_available().await {
                    return Ok(ResolvedBackend::AgentBrowser);
                }

                let computer_use_err = match self.computer_use_available() {
                    Ok(true) => return Ok(ResolvedBackend::ComputerUse),
                    Ok(false) => None,
                    Err(err) => Some(err.to_string()),
                };

                if Self::rust_native_compiled() {
                    if let Some(err) = computer_use_err {
                        anyhow::bail!(
                            "browser.backend='auto' found no usable backend (agent-browser missing, rust-native unavailable, computer-use invalid: {err})"
                        );
                    }
                    anyhow::bail!(
                        "browser.backend='auto' found no usable backend (agent-browser missing, rust-native unavailable, computer-use sidecar unreachable)"
                    )
                }

                if let Some(err) = computer_use_err {
                    anyhow::bail!(
                        "browser.backend='auto' needs agent-browser tool, browser-native, or valid computer-use sidecar (error: {err})"
                    );
                }

                anyhow::bail!(
                    "browser.backend='auto' needs agent-browser tool, browser-native, or computer-use sidecar"
                )
            }
        }
    }

    /// Validate URL against allowlist
    fn validate_url(&self, url: &str) -> anyhow::Result<()> {
        let url = url.trim();

        if url.is_empty() {
            anyhow::bail!("URL cannot be empty");
        }

        // Block file:// URLs — browser file access bypasses all SSRF and
        // domain-allowlist controls and can exfiltrate arbitrary local files.
        if url.starts_with("file://") {
            anyhow::bail!("file:// URLs are not allowed in browser automation");
        }

        if !url.starts_with("https://") && !url.starts_with("http://") {
            anyhow::bail!("Only http:// and https:// URLs are allowed");
        }

        if self.allowed_domains.is_empty() && !allow_all_browser_domains() {
            anyhow::bail!(
                "Browser tool enabled but no allowed_domains configured. \
                Add [browser].allowed_domains in config.toml or set OPENHUMAN_BROWSER_ALLOW_ALL=1"
            );
        }

        let host = extract_host(url)?;

        if is_private_host(&host) {
            anyhow::bail!("Blocked local/private host: {host}");
        }

        if !self.allowed_domains.is_empty()
            && !host_matches_allowlist(&host, &self.allowed_domains)
        {
            anyhow::bail!("Host '{host}' not in browser.allowed_domains");
        }

        Ok(())
    }

    /// Execute an agent-browser command
    async fn run_command(&self, args: &[&str]) -> anyhow::Result<AgentBrowserResponse> {
        let mut cmd = Command::new("agent-browser");

        // Add session if configured
        if let Some(ref session) = self.session_name {
            cmd.arg("--session").arg(session);
        }

        // Add --json for machine-readable output
        cmd.args(args).arg("--json");

        debug!("Running: agent-browser {} --json", args.join(" "));

        let output = cmd
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if !stderr.is_empty() {
            debug!("agent-browser stderr: {}", stderr);
        }

        // Parse JSON response
        if let Ok(resp) = serde_json::from_str::<AgentBrowserResponse>(&stdout) {
            return Ok(resp);
        }

        // Fallback for non-JSON output
        if output.status.success() {
            Ok(AgentBrowserResponse {
                success: true,
                data: Some(json!({ "output": stdout.trim() })),
                error: None,
            })
        } else {
            Ok(AgentBrowserResponse {
                success: false,
                data: None,
                error: Some(stderr.trim().to_string()),
            })
        }
    }

    /// Execute a browser action via agent-browser tool
    #[allow(clippy::too_many_lines)]
    async fn execute_agent_browser_action(
        &self,
        action: BrowserAction,
    ) -> anyhow::Result<ToolResult> {
        match action {
            BrowserAction::Open { url } => {
                self.validate_url(&url)?;
                let resp = self.run_command(&["open", &url]).await?;
                self.to_result(resp)
            }

            BrowserAction::Snapshot {
                interactive_only,
                compact,
                depth,
            } => {
                let mut args = vec!["snapshot"];
                if interactive_only {
                    args.push("-i");
                }
                if compact {
                    args.push("-c");
                }
                let depth_str;
                if let Some(d) = depth {
                    args.push("-d");
                    depth_str = d.to_string();
                    args.push(&depth_str);
                }
                let resp = self.run_command(&args).await?;
                self.to_result(resp)
            }

            BrowserAction::Click { selector } => {
                let resp = self.run_command(&["click", &selector]).await?;
                self.to_result(resp)
            }

            BrowserAction::Fill { selector, value } => {
                let resp = self.run_command(&["fill", &selector, &value]).await?;
                self.to_result(resp)
            }

            BrowserAction::Type { selector, text } => {
                let resp = self.run_command(&["type", &selector, &text]).await?;
                self.to_result(resp)
            }

            BrowserAction::GetText { selector } => {
                let resp = self.run_command(&["get", "text", &selector]).await?;
                self.to_result(resp)
            }

            BrowserAction::GetTitle => {
                let resp = self.run_command(&["get", "title"]).await?;
                self.to_result(resp)
            }

            BrowserAction::GetUrl => {
                let resp = self.run_command(&["get", "url"]).await?;
                self.to_result(resp)
            }

            BrowserAction::Screenshot { path, full_page } => {
                let mut args = vec!["screenshot"];
                if let Some(ref p) = path {
                    args.push(p);
                }
                if full_page {
                    args.push("--full");
                }
                let resp = self.run_command(&args).await?;
                self.to_result(resp)
            }

            BrowserAction::Wait { selector, ms, text } => {
                let mut args = vec!["wait"];
                let ms_str;
                if let Some(sel) = selector.as_ref() {
                    args.push(sel);
                } else if let Some(millis) = ms {
                    ms_str = millis.to_string();
                    args.push(&ms_str);
                } else if let Some(ref t) = text {
                    args.push("--text");
                    args.push(t);
                }
                let resp = self.run_command(&args).await?;
                self.to_result(resp)
            }

            BrowserAction::Press { key } => {
                let resp = self.run_command(&["press", &key]).await?;
                self.to_result(resp)
            }

            BrowserAction::Hover { selector } => {
                let resp = self.run_command(&["hover", &selector]).await?;
                self.to_result(resp)
            }

            BrowserAction::Scroll { direction, pixels } => {
                let mut args = vec!["scroll", &direction];
                let px_str;
                if let Some(px) = pixels {
                    px_str = px.to_string();
                    args.push(&px_str);
                }
                let resp = self.run_command(&args).await?;
                self.to_result(resp)
            }

            BrowserAction::IsVisible { selector } => {
                let resp = self.run_command(&["is", "visible", &selector]).await?;
                self.to_result(resp)
            }

            BrowserAction::Close => {
                let resp = self.run_command(&["close"]).await?;
                self.to_result(resp)
            }

            BrowserAction::Find {
                by,
                value,
                action,
                fill_value,
            } => {
                let mut args = vec!["find", &by, &value, &action];
                if let Some(ref fv) = fill_value {
                    args.push(fv);
                }
                let resp = self.run_command(&args).await?;
                self.to_result(resp)
            }
        }
    }

    #[allow(clippy::unused_async)]
    async fn execute_rust_native_action(
        &self,
        action: BrowserAction,
    ) -> anyhow::Result<ToolResult> {
        #[cfg(feature = "browser-native")]
        {
            let mut state = self.native_state.lock().await;

            let output = state
                .execute_action(
                    action,
                    self.native_headless,
                    &self.native_webdriver_url,
                    self.native_chrome_path.as_deref(),
                )
                .await?;

            Ok(ToolResult {
                success: true,
                output: serde_json::to_string_pretty(&output).unwrap_or_default(),
                error: None,
            })
        }

        #[cfg(not(feature = "browser-native"))]
        {
            let _ = action;
            anyhow::bail!(
                "Rust-native browser backend is not compiled. Rebuild with --features browser-native"
            )
        }
    }

    fn validate_coordinate(&self, key: &str, value: i64, max: Option<i64>) -> anyhow::Result<()> {
        if value < 0 {
            anyhow::bail!("'{key}' must be >= 0")
        }
        if let Some(limit) = max {
            if limit < 0 {
                anyhow::bail!("Configured coordinate limit for '{key}' must be >= 0")
            }
            if value > limit {
                anyhow::bail!("'{key}'={value} exceeds configured limit {limit}")
            }
        }
        Ok(())
    }

    fn read_required_i64(
        &self,
        params: &serde_json::Map<String, Value>,
        key: &str,
    ) -> anyhow::Result<i64> {
        params
            .get(key)
            .and_then(Value::as_i64)
            .ok_or_else(|| anyhow::anyhow!("Missing or invalid '{key}' parameter"))
    }

    fn validate_computer_use_action(
        &self,
        action: &str,
        params: &serde_json::Map<String, Value>,
    ) -> anyhow::Result<()> {
        match action {
            "open" => {
                let url = params
                    .get("url")
                    .and_then(Value::as_str)
                    .ok_or_else(|| anyhow::anyhow!("Missing 'url' for open action"))?;
                self.validate_url(url)?;
            }
            "mouse_move" | "mouse_click" => {
                let x = self.read_required_i64(params, "x")?;
                let y = self.read_required_i64(params, "y")?;
                self.validate_coordinate("x", x, self.computer_use.max_coordinate_x)?;
                self.validate_coordinate("y", y, self.computer_use.max_coordinate_y)?;
            }
            "mouse_drag" => {
                let from_x = self.read_required_i64(params, "from_x")?;
                let from_y = self.read_required_i64(params, "from_y")?;
                let to_x = self.read_required_i64(params, "to_x")?;
                let to_y = self.read_required_i64(params, "to_y")?;
                self.validate_coordinate("from_x", from_x, self.computer_use.max_coordinate_x)?;
                self.validate_coordinate("to_x", to_x, self.computer_use.max_coordinate_x)?;
                self.validate_coordinate("from_y", from_y, self.computer_use.max_coordinate_y)?;
                self.validate_coordinate("to_y", to_y, self.computer_use.max_coordinate_y)?;
            }
            _ => {}
        }
        Ok(())
    }

    async fn execute_computer_use_action(
        &self,
        action: &str,
        args: &Value,
    ) -> anyhow::Result<ToolResult> {
        let endpoint = self.computer_use_endpoint_url()?;

        let mut params = args
            .as_object()
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("browser args must be a JSON object"))?;
        params.remove("action");

        self.validate_computer_use_action(action, &params)?;

        let payload = json!({
            "action": action,
            "params": params,
            "policy": {
                "allowed_domains": self.allowed_domains,
                "window_allowlist": self.computer_use.window_allowlist,
                "max_coordinate_x": self.computer_use.max_coordinate_x,
                "max_coordinate_y": self.computer_use.max_coordinate_y,
            },
            "metadata": {
                "session_name": self.session_name,
                "source": "openhuman.browser",
                "version": env!("CARGO_PKG_VERSION"),
            }
        });

        let client = crate::openhuman::config::build_runtime_proxy_client("tool.browser");
        let mut request = client
            .post(endpoint)
            .timeout(Duration::from_millis(self.computer_use.timeout_ms))
            .json(&payload);

        if let Some(api_key) = self.computer_use.api_key.as_deref() {
            let token = api_key.trim();
            if !token.is_empty() {
                request = request.bearer_auth(token);
            }
        }

        let response = request.send().await.with_context(|| {
            format!(
                "Failed to call computer-use sidecar at {}",
                self.computer_use.endpoint
            )
        })?;

        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read computer-use sidecar response body")?;

        if let Ok(parsed) = serde_json::from_str::<ComputerUseResponse>(&body) {
            if status.is_success() && parsed.success.unwrap_or(true) {
                let output = parsed
                    .data
                    .map(|data| serde_json::to_string_pretty(&data).unwrap_or_default())
                    .unwrap_or_else(|| {
                        serde_json::to_string_pretty(&json!({
                            "backend": "computer_use",
                            "action": action,
                            "ok": true,
                        }))
                        .unwrap_or_default()
                    });

                return Ok(ToolResult {
                    success: true,
                    output,
                    error: None,
                });
            }

            let error = parsed.error.or_else(|| {
                if status.is_success() && parsed.success == Some(false) {
                    Some("computer-use sidecar returned success=false".to_string())
                } else {
                    Some(format!(
                        "computer-use sidecar request failed with status {status}"
                    ))
                }
            });

            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error,
            });
        }

        if status.is_success() {
            return Ok(ToolResult {
                success: true,
                output: body,
                error: None,
            });
        }

        Ok(ToolResult {
            success: false,
            output: String::new(),
            error: Some(format!(
                "computer-use sidecar request failed with status {status}: {}",
                body.trim()
            )),
        })
    }

    async fn execute_action(
        &self,
        action: BrowserAction,
        backend: ResolvedBackend,
    ) -> anyhow::Result<ToolResult> {
        match backend {
            ResolvedBackend::AgentBrowser => self.execute_agent_browser_action(action).await,
            ResolvedBackend::RustNative => self.execute_rust_native_action(action).await,
            ResolvedBackend::ComputerUse => anyhow::bail!(
                "Internal error: computer_use backend must be handled before BrowserAction parsing"
            ),
        }
    }

    #[allow(clippy::unnecessary_wraps, clippy::unused_self)]
    fn to_result(&self, resp: AgentBrowserResponse) -> anyhow::Result<ToolResult> {
        if resp.success {
            let output = resp
                .data
                .map(|d| serde_json::to_string_pretty(&d).unwrap_or_default())
                .unwrap_or_default();
            Ok(ToolResult {
                success: true,
                output,
                error: None,
            })
        } else {
            Ok(ToolResult {
                success: false,
                output: String::new(),
                error: resp.error,
            })
        }
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn name(&self) -> &str {
        "browser"
    }

    fn description(&self) -> &str {
        concat!(
            "Web/browser automation with pluggable backends (agent-browser, rust-native, computer_use). ",
            "Supports DOM actions plus optional OS-level actions (mouse_move, mouse_click, mouse_drag, ",
            "key_type, key_press, screen_capture) through a computer-use sidecar. Use 'snapshot' to map ",
            "interactive elements to refs (@e1, @e2). Enforces browser.allowed_domains for open actions."
        )
    }

    fn parameters_schema(&self) -> Value {
        json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["open", "snapshot", "click", "fill", "type", "get_text",
                             "get_title", "get_url", "screenshot", "wait", "press",
                             "hover", "scroll", "is_visible", "close", "find",
                             "mouse_move", "mouse_click", "mouse_drag", "key_type",
                             "key_press", "screen_capture"],
                    "description": "Browser action to perform (OS-level actions require backend=computer_use)"
                },
                "url": {
                    "type": "string",
                    "description": "URL to navigate to (for 'open' action)"
                },
                "selector": {
                    "type": "string",
                    "description": "Element selector: @ref (e.g. @e1), CSS (#id, .class), or text=..."
                },
                "value": {
                    "type": "string",
                    "description": "Value to fill or type"
                },
                "text": {
                    "type": "string",
                    "description": "Text to type or wait for"
                },
                "key": {
                    "type": "string",
                    "description": "Key to press (Enter, Tab, Escape, etc.)"
                },
                "x": {
                    "type": "integer",
                    "description": "Screen X coordinate (computer_use: mouse_move/mouse_click)"
                },
                "y": {
                    "type": "integer",
                    "description": "Screen Y coordinate (computer_use: mouse_move/mouse_click)"
                },
                "from_x": {
                    "type": "integer",
                    "description": "Drag source X coordinate (computer_use: mouse_drag)"
                },
                "from_y": {
                    "type": "integer",
                    "description": "Drag source Y coordinate (computer_use: mouse_drag)"
                },
                "to_x": {
                    "type": "integer",
                    "description": "Drag target X coordinate (computer_use: mouse_drag)"
                },
                "to_y": {
                    "type": "integer",
                    "description": "Drag target Y coordinate (computer_use: mouse_drag)"
                },
                "button": {
                    "type": "string",
                    "enum": ["left", "right", "middle"],
                    "description": "Mouse button for computer_use mouse_click"
                },
                "direction": {
                    "type": "string",
                    "enum": ["up", "down", "left", "right"],
                    "description": "Scroll direction"
                },
                "pixels": {
                    "type": "integer",
                    "description": "Pixels to scroll"
                },
                "interactive_only": {
                    "type": "boolean",
                    "description": "For snapshot: only show interactive elements"
                },
                "compact": {
                    "type": "boolean",
                    "description": "For snapshot: remove empty structural elements"
                },
                "depth": {
                    "type": "integer",
                    "description": "For snapshot: limit tree depth"
                },
                "full_page": {
                    "type": "boolean",
                    "description": "For screenshot: capture full page"
                },
                "path": {
                    "type": "string",
                    "description": "File path for screenshot"
                },
                "ms": {
                    "type": "integer",
                    "description": "Milliseconds to wait"
                },
                "by": {
                    "type": "string",
                    "enum": ["role", "text", "label", "placeholder", "testid"],
                    "description": "For find: semantic locator type"
                },
                "find_action": {
                    "type": "string",
                    "enum": ["click", "fill", "text", "hover", "check"],
                    "description": "For find: action to perform on found element"
                },
                "fill_value": {
                    "type": "string",
                    "description": "For find with fill action: value to fill"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, args: Value) -> anyhow::Result<ToolResult> {
        // Security checks
        if !self.security.can_act() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: autonomy is read-only".into()),
            });
        }

        if !self.security.record_action() {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some("Action blocked: rate limit exceeded".into()),
            });
        }

        let backend = match self.resolve_backend().await {
            Ok(selected) => selected,
            Err(error) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(error.to_string()),
                });
            }
        };

        // Parse action from args
        let action_str = args
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'action' parameter"))?;

        if !is_supported_browser_action(action_str) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Unknown action: {action_str}")),
            });
        }

        if backend == ResolvedBackend::ComputerUse {
            return self.execute_computer_use_action(action_str, &args).await;
        }

        if is_computer_use_only_action(action_str) {
            return Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(unavailable_action_for_backend_error(action_str, backend)),
            });
        }

        let action = match parse_browser_action(action_str, &args) {
            Ok(a) => a,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e.to_string()),
                });
            }
        };

        self.execute_action(action, backend).await
    }
}

#[cfg(feature = "browser-native")]
mod native_backend {
    use super::BrowserAction;
    use anyhow::{Context, Result};
    use base64::Engine;
    use fantoccini::actions::{InputSource, MouseActions, PointerAction};
    use fantoccini::key::Key;
    use fantoccini::{Client, ClientBuilder, Locator};
    use serde_json::{json, Map, Value};
    use std::net::{TcpStream, ToSocketAddrs};
    use std::time::Duration;

    #[derive(Default)]
    pub struct NativeBrowserState {
        client: Option<Client>,
    }

    impl NativeBrowserState {
        pub fn is_available(
            _headless: bool,
            webdriver_url: &str,
            _chrome_path: Option<&str>,
        ) -> bool {
            webdriver_endpoint_reachable(webdriver_url, Duration::from_millis(500))
        }

        #[allow(clippy::too_many_lines)]
        pub async fn execute_action(
            &mut self,
            action: BrowserAction,
            headless: bool,
            webdriver_url: &str,
            chrome_path: Option<&str>,
        ) -> Result<Value> {
            match action {
                BrowserAction::Open { url } => {
                    self.ensure_session(headless, webdriver_url, chrome_path)
                        .await?;
                    let client = self.active_client()?;
                    client
                        .goto(&url)
                        .await
                        .with_context(|| format!("Failed to open URL: {url}"))?;
                    let current_url = client
                        .current_url()
                        .await
                        .context("Failed to read current URL after navigation")?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "open",
                        "url": current_url.as_str(),
                    }))
                }
                BrowserAction::Snapshot {
                    interactive_only,
                    compact,
                    depth,
                } => {
                    let client = self.active_client()?;
                    let snapshot = client
                        .execute(
                            &snapshot_script(interactive_only, compact, depth.map(i64::from)),
                            vec![],
                        )
                        .await
                        .context("Failed to evaluate snapshot script")?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "snapshot",
                        "data": snapshot,
                    }))
                }
                BrowserAction::Click { selector } => {
                    let client = self.active_client()?;
                    find_element(client, &selector).await?.click().await?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "click",
                        "selector": selector,
                    }))
                }
                BrowserAction::Fill { selector, value } => {
                    let client = self.active_client()?;
                    let element = find_element(client, &selector).await?;
                    let _ = element.clear().await;
                    element.send_keys(&value).await?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "fill",
                        "selector": selector,
                    }))
                }
                BrowserAction::Type { selector, text } => {
                    let client = self.active_client()?;
                    find_element(client, &selector)
                        .await?
                        .send_keys(&text)
                        .await?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "type",
                        "selector": selector,
                        "typed": text.len(),
                    }))
                }
                BrowserAction::GetText { selector } => {
                    let client = self.active_client()?;
                    let text = find_element(client, &selector).await?.text().await?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "get_text",
                        "selector": selector,
                        "text": text,
                    }))
                }
                BrowserAction::GetTitle => {
                    let client = self.active_client()?;
                    let title = client.title().await.context("Failed to read page title")?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "get_title",
                        "title": title,
                    }))
                }
                BrowserAction::GetUrl => {
                    let client = self.active_client()?;
                    let url = client
                        .current_url()
                        .await
                        .context("Failed to read current URL")?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "get_url",
                        "url": url.as_str(),
                    }))
                }
                BrowserAction::Screenshot { path, full_page } => {
                    let client = self.active_client()?;
                    let png = client
                        .screenshot()
                        .await
                        .context("Failed to capture screenshot")?;
                    let mut payload = json!({
                        "backend": "rust_native",
                        "action": "screenshot",
                        "full_page": full_page,
                        "bytes": png.len(),
                    });

                    if let Some(path_str) = path {
                        tokio::fs::write(&path_str, &png)
                            .await
                            .with_context(|| format!("Failed to write screenshot to {path_str}"))?;
                        payload["path"] = Value::String(path_str);
                    } else {
                        payload["png_base64"] =
                            Value::String(base64::engine::general_purpose::STANDARD.encode(&png));
                    }

                    Ok(payload)
                }
                BrowserAction::Wait { selector, ms, text } => {
                    let client = self.active_client()?;
                    if let Some(sel) = selector.as_ref() {
                        wait_for_selector(client, sel).await?;
                        Ok(json!({
                            "backend": "rust_native",
                            "action": "wait",
                            "selector": sel,
                        }))
                    } else if let Some(duration_ms) = ms {
                        tokio::time::sleep(Duration::from_millis(duration_ms)).await;
                        Ok(json!({
                            "backend": "rust_native",
                            "action": "wait",
                            "ms": duration_ms,
                        }))
                    } else if let Some(needle) = text.as_ref() {
                        let xpath = xpath_contains_text(needle);
                        client
                            .wait()
                            .for_element(Locator::XPath(&xpath))
                            .await
                            .with_context(|| {
                                format!("Timed out waiting for text to appear: {needle}")
                            })?;
                        Ok(json!({
                            "backend": "rust_native",
                            "action": "wait",
                            "text": needle,
                        }))
                    } else {
                        tokio::time::sleep(Duration::from_millis(250)).await;
                        Ok(json!({
                            "backend": "rust_native",
                            "action": "wait",
                            "ms": 250,
                        }))
                    }
                }
                BrowserAction::Press { key } => {
                    let client = self.active_client()?;
                    let key_input = webdriver_key(&key);
                    match client.active_element().await {
                        Ok(element) => {
                            element.send_keys(&key_input).await?;
                        }
                        Err(_) => {
                            find_element(client, "body")
                                .await?
                                .send_keys(&key_input)
                                .await?;
                        }
                    }

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "press",
                        "key": key,
                    }))
                }
                BrowserAction::Hover { selector } => {
                    let client = self.active_client()?;
                    let element = find_element(client, &selector).await?;
                    hover_element(client, &element).await?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "hover",
                        "selector": selector,
                    }))
                }
                BrowserAction::Scroll { direction, pixels } => {
                    let client = self.active_client()?;
                    let amount = i64::from(pixels.unwrap_or(600));
                    let (dx, dy) = match direction.as_str() {
                        "up" => (0, -amount),
                        "down" => (0, amount),
                        "left" => (-amount, 0),
                        "right" => (amount, 0),
                        _ => anyhow::bail!(
                            "Unsupported scroll direction '{direction}'. Use up/down/left/right"
                        ),
                    };

                    let position = client
                        .execute(
                            "window.scrollBy(arguments[0], arguments[1]); return { x: window.scrollX, y: window.scrollY };",
                            vec![json!(dx), json!(dy)],
                        )
                        .await
                        .context("Failed to execute scroll script")?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "scroll",
                        "position": position,
                    }))
                }
                BrowserAction::IsVisible { selector } => {
                    let client = self.active_client()?;
                    let visible = find_element(client, &selector)
                        .await?
                        .is_displayed()
                        .await?;

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "is_visible",
                        "selector": selector,
                        "visible": visible,
                    }))
                }
                BrowserAction::Close => {
                    if let Some(client) = self.client.take() {
                        let _ = client.close().await;
                    }

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "close",
                        "closed": true,
                    }))
                }
                BrowserAction::Find {
                    by,
                    value,
                    action,
                    fill_value,
                } => {
                    let client = self.active_client()?;
                    let selector = selector_for_find(&by, &value);
                    let element = find_element(client, &selector).await?;

                    let payload = match action.as_str() {
                        "click" => {
                            element.click().await?;
                            json!({"result": "clicked"})
                        }
                        "fill" => {
                            let fill = fill_value.ok_or_else(|| {
                                anyhow::anyhow!("find_action='fill' requires fill_value")
                            })?;
                            let _ = element.clear().await;
                            element.send_keys(&fill).await?;
                            json!({"result": "filled", "typed": fill.len()})
                        }
                        "text" => {
                            let text = element.text().await?;
                            json!({"result": "text", "text": text})
                        }
                        "hover" => {
                            hover_element(client, &element).await?;
                            json!({"result": "hovered"})
                        }
                        "check" => {
                            let checked_before = element_checked(&element).await?;
                            if !checked_before {
                                element.click().await?;
                            }
                            let checked_after = element_checked(&element).await?;
                            json!({
                                "result": "checked",
                                "checked_before": checked_before,
                                "checked_after": checked_after,
                            })
                        }
                        _ => anyhow::bail!(
                            "Unsupported find_action '{action}'. Use click/fill/text/hover/check"
                        ),
                    };

                    Ok(json!({
                        "backend": "rust_native",
                        "action": "find",
                        "by": by,
                        "value": value,
                        "selector": selector,
                        "data": payload,
                    }))
                }
            }
        }

        async fn ensure_session(
            &mut self,
            headless: bool,
            webdriver_url: &str,
            chrome_path: Option<&str>,
        ) -> Result<()> {
            if self.client.is_some() {
                return Ok(());
            }

            let mut capabilities: Map<String, Value> = Map::new();
            let mut chrome_options: Map<String, Value> = Map::new();
            let mut args: Vec<Value> = Vec::new();

            if headless {
                args.push(Value::String("--headless=new".to_string()));
                args.push(Value::String("--disable-gpu".to_string()));
            }

            if !args.is_empty() {
                chrome_options.insert("args".to_string(), Value::Array(args));
            }

            if let Some(path) = chrome_path {
                let trimmed = path.trim();
                if !trimmed.is_empty() {
                    chrome_options.insert("binary".to_string(), Value::String(trimmed.to_string()));
                }
            }

            if !chrome_options.is_empty() {
                capabilities.insert(
                    "goog:chromeOptions".to_string(),
                    Value::Object(chrome_options),
                );
            }

            let mut builder =
                ClientBuilder::rustls().context("Failed to initialize rustls connector")?;
            if !capabilities.is_empty() {
                builder.capabilities(capabilities);
            }

            let client = builder
                .connect(webdriver_url)
                .await
                .with_context(|| {
                    format!(
                        "Failed to connect to WebDriver at {webdriver_url}. Start chromedriver/geckodriver first"
                    )
                })?;

            self.client = Some(client);
            Ok(())
        }

        fn active_client(&self) -> Result<&Client> {
            self.client.as_ref().ok_or_else(|| {
                anyhow::anyhow!("No active native browser session. Run browser action='open' first")
            })
        }
    }

    fn webdriver_endpoint_reachable(webdriver_url: &str, timeout: Duration) -> bool {
        let parsed = match reqwest::Url::parse(webdriver_url) {
            Ok(url) => url,
            Err(_) => return false,
        };

        if parsed.scheme() != "http" && parsed.scheme() != "https" {
            return false;
        }

        let host = match parsed.host_str() {
            Some(h) if !h.is_empty() => h,
            _ => return false,
        };

        let port = parsed.port_or_known_default().unwrap_or(4444);
        let mut addrs = match (host, port).to_socket_addrs() {
            Ok(iter) => iter,
            Err(_) => return false,
        };

        let addr = match addrs.next() {
            Some(a) => a,
            None => return false,
        };

        TcpStream::connect_timeout(&addr, timeout).is_ok()
    }

    fn selector_for_find(by: &str, value: &str) -> String {
        let escaped = css_attr_escape(value);
        match by {
            "role" => format!(r#"[role=\"{escaped}\"]"#),
            "label" => format!("label={value}"),
            "placeholder" => format!(r#"[placeholder=\"{escaped}\"]"#),
            "testid" => format!(r#"[data-testid=\"{escaped}\"]"#),
            _ => format!("text={value}"),
        }
    }

    async fn wait_for_selector(client: &Client, selector: &str) -> Result<()> {
        match parse_selector(selector) {
            SelectorKind::Css(css) => {
                client
                    .wait()
                    .for_element(Locator::Css(&css))
                    .await
                    .with_context(|| format!("Timed out waiting for selector '{selector}'"))?;
            }
            SelectorKind::XPath(xpath) => {
                client
                    .wait()
                    .for_element(Locator::XPath(&xpath))
                    .await
                    .with_context(|| format!("Timed out waiting for selector '{selector}'"))?;
            }
        }
        Ok(())
    }

    async fn find_element(
        client: &Client,
        selector: &str,
    ) -> Result<fantoccini::elements::Element> {
        let element = match parse_selector(selector) {
            SelectorKind::Css(css) => client
                .find(Locator::Css(&css))
                .await
                .with_context(|| format!("Failed to find element by CSS '{css}'"))?,
            SelectorKind::XPath(xpath) => client
                .find(Locator::XPath(&xpath))
                .await
                .with_context(|| format!("Failed to find element by XPath '{xpath}'"))?,
        };
        Ok(element)
    }

    async fn hover_element(client: &Client, element: &fantoccini::elements::Element) -> Result<()> {
        let actions = MouseActions::new("mouse".to_string()).then(PointerAction::MoveToElement {
            element: element.clone(),
            duration: Some(Duration::from_millis(150)),
            x: 0.0,
            y: 0.0,
        });

        client
            .perform_actions(actions)
            .await
            .context("Failed to perform hover action")?;
        let _ = client.release_actions().await;
        Ok(())
    }

    async fn element_checked(element: &fantoccini::elements::Element) -> Result<bool> {
        let checked = element
            .prop("checked")
            .await
            .context("Failed to read checkbox checked property")?
            .unwrap_or_default()
            .to_ascii_lowercase();
        Ok(matches!(checked.as_str(), "true" | "checked" | "1"))
    }

    enum SelectorKind {
        Css(String),
        XPath(String),
    }

    fn parse_selector(selector: &str) -> SelectorKind {
        let trimmed = selector.trim();
        if let Some(text_query) = trimmed.strip_prefix("text=") {
            return SelectorKind::XPath(xpath_contains_text(text_query));
        }

        if let Some(label_query) = trimmed.strip_prefix("label=") {
            let literal = xpath_literal(label_query);
            return SelectorKind::XPath(format!(
                "(//label[contains(normalize-space(.), {literal})]/following::*[self::input or self::textarea or self::select][1] | //*[@aria-label and contains(normalize-space(@aria-label), {literal})] | //label[contains(normalize-space(.), {literal})])"
            ));
        }

        if trimmed.starts_with('@') {
            let escaped = css_attr_escape(trimmed);
            return SelectorKind::Css(format!(r#"[data-zc-ref=\"{escaped}\"]"#));
        }

        SelectorKind::Css(trimmed.to_string())
    }

    fn css_attr_escape(input: &str) -> String {
        input
            .replace('\\', "\\\\")
            .replace('"', "\\\"")
            .replace('\n', " ")
    }

    fn xpath_contains_text(text: &str) -> String {
        format!("//*[contains(normalize-space(.), {})]", xpath_literal(text))
    }

    fn xpath_literal(input: &str) -> String {
        if !input.contains('"') {
            return format!("\"{input}\"");
        }
        if !input.contains('\'') {
            return format!("'{input}'");
        }

        let segments: Vec<&str> = input.split('"').collect();
        let mut parts: Vec<String> = Vec::new();
        for (index, part) in segments.iter().enumerate() {
            if !part.is_empty() {
                parts.push(format!("\"{part}\""));
            }
            if index + 1 < segments.len() {
                parts.push("'\"'".to_string());
            }
        }

        if parts.is_empty() {
            "\"\"".to_string()
        } else {
            format!("concat({})", parts.join(","))
        }
    }

    fn webdriver_key(key: &str) -> String {
        match key.trim().to_ascii_lowercase().as_str() {
            "enter" => Key::Enter.to_string(),
            "return" => Key::Return.to_string(),
            "tab" => Key::Tab.to_string(),
            "escape" | "esc" => Key::Escape.to_string(),
            "backspace" => Key::Backspace.to_string(),
            "delete" => Key::Delete.to_string(),
            "space" => Key::Space.to_string(),
            "arrowup" | "up" => Key::Up.to_string(),
            "arrowdown" | "down" => Key::Down.to_string(),
            "arrowleft" | "left" => Key::Left.to_string(),
            "arrowright" | "right" => Key::Right.to_string(),
            "home" => Key::Home.to_string(),
            "end" => Key::End.to_string(),
            "pageup" => Key::PageUp.to_string(),
            "pagedown" => Key::PageDown.to_string(),
            other => other.to_string(),
        }
    }

    fn snapshot_script(interactive_only: bool, compact: bool, depth: Option<i64>) -> String {
        let depth_literal = depth
            .map(|level| level.to_string())
            .unwrap_or_else(|| "null".to_string());

        format!(
            r#"(() => {{
  const interactiveOnly = {interactive_only};
  const compact = {compact};
  const maxDepth = {depth_literal};
  const nodes = [];
  const root = document.body || document.documentElement;
  let counter = 0;

  const isVisible = (el) => {{
    const style = window.getComputedStyle(el);
    if (style.display === 'none' || style.visibility === 'hidden' || Number(style.opacity || 1) === 0) {{
      return false;
    }}
    const rect = el.getBoundingClientRect();
    return rect.width > 0 && rect.height > 0;
  }};

  const isInteractive = (el) => {{
    if (el.matches('a,button,input,select,textarea,summary,[role],*[tabindex]')) return true;
    return typeof el.onclick === 'function';
  }};

  const describe = (el, depth) => {{
    const interactive = isInteractive(el);
    const text = (el.innerText || el.textContent || '').trim().replace(/\s+/g, ' ').slice(0, 140);
    if (interactiveOnly && !interactive) return;
    if (compact && !interactive && !text) return;

    const ref = '@e' + (++counter);
    el.setAttribute('data-zc-ref', ref);
    nodes.push({{
      ref,
      depth,
      tag: el.tagName.toLowerCase(),
      id: el.id || null,
      role: el.getAttribute('role'),
      text,
      interactive,
    }});
  }};

  const walk = (el, depth) => {{
    if (!(el instanceof Element)) return;
    if (maxDepth !== null && depth > maxDepth) return;
    if (isVisible(el)) {{
      describe(el, depth);
    }}
    for (const child of el.children) {{
      walk(child, depth + 1);
      if (nodes.length >= 400) return;
    }}
  }};

  if (root) walk(root, 0);

  return {{
    title: document.title,
    url: window.location.href,
    count: nodes.length,
    nodes,
  }};
}})();"#
        )
    }
}

// ── Action parsing ──────────────────────────────────────────────

/// Parse a JSON `args` object into a typed `BrowserAction`.
fn parse_browser_action(action_str: &str, args: &Value) -> anyhow::Result<BrowserAction> {
    match action_str {
        "open" => {
            let url = args
                .get("url")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'url' for open action"))?;
            Ok(BrowserAction::Open { url: url.into() })
        }
        "snapshot" => Ok(BrowserAction::Snapshot {
            interactive_only: args
                .get("interactive_only")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            compact: args
                .get("compact")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(true),
            depth: args
                .get("depth")
                .and_then(serde_json::Value::as_u64)
                .map(|d| u32::try_from(d).unwrap_or(u32::MAX)),
        }),
        "click" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for click"))?;
            Ok(BrowserAction::Click {
                selector: selector.into(),
            })
        }
        "fill" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for fill"))?;
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'value' for fill"))?;
            Ok(BrowserAction::Fill {
                selector: selector.into(),
                value: value.into(),
            })
        }
        "type" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for type"))?;
            let text = args
                .get("text")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'text' for type"))?;
            Ok(BrowserAction::Type {
                selector: selector.into(),
                text: text.into(),
            })
        }
        "get_text" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for get_text"))?;
            Ok(BrowserAction::GetText {
                selector: selector.into(),
            })
        }
        "get_title" => Ok(BrowserAction::GetTitle),
        "get_url" => Ok(BrowserAction::GetUrl),
        "screenshot" => Ok(BrowserAction::Screenshot {
            path: args.get("path").and_then(|v| v.as_str()).map(String::from),
            full_page: args
                .get("full_page")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
        }),
        "wait" => Ok(BrowserAction::Wait {
            selector: args
                .get("selector")
                .and_then(|v| v.as_str())
                .map(String::from),
            ms: args.get("ms").and_then(serde_json::Value::as_u64),
            text: args.get("text").and_then(|v| v.as_str()).map(String::from),
        }),
        "press" => {
            let key = args
                .get("key")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'key' for press"))?;
            Ok(BrowserAction::Press { key: key.into() })
        }
        "hover" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for hover"))?;
            Ok(BrowserAction::Hover {
                selector: selector.into(),
            })
        }
        "scroll" => {
            let direction = args
                .get("direction")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'direction' for scroll"))?;
            Ok(BrowserAction::Scroll {
                direction: direction.into(),
                pixels: args
                    .get("pixels")
                    .and_then(serde_json::Value::as_u64)
                    .map(|p| u32::try_from(p).unwrap_or(u32::MAX)),
            })
        }
        "is_visible" => {
            let selector = args
                .get("selector")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'selector' for is_visible"))?;
            Ok(BrowserAction::IsVisible {
                selector: selector.into(),
            })
        }
        "close" => Ok(BrowserAction::Close),
        "find" => {
            let by = args
                .get("by")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'by' for find"))?;
            let value = args
                .get("value")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'value' for find"))?;
            let action = args
                .get("find_action")
                .and_then(|v| v.as_str())
                .ok_or_else(|| anyhow::anyhow!("Missing 'find_action' for find"))?;
            Ok(BrowserAction::Find {
                by: by.into(),
                value: value.into(),
                action: action.into(),
                fill_value: args
                    .get("fill_value")
                    .and_then(|v| v.as_str())
                    .map(String::from),
            })
        }
        other => anyhow::bail!("Unsupported browser action: {other}"),
    }
}

// ── Helper functions ─────────────────────────────────────────────

fn is_supported_browser_action(action: &str) -> bool {
    matches!(
        action,
        "open"
            | "snapshot"
            | "click"
            | "fill"
            | "type"
            | "get_text"
            | "get_title"
            | "get_url"
            | "screenshot"
            | "wait"
            | "press"
            | "hover"
            | "scroll"
            | "is_visible"
            | "close"
            | "find"
            | "mouse_move"
            | "mouse_click"
            | "mouse_drag"
            | "key_type"
            | "key_press"
            | "screen_capture"
    )
}

fn is_computer_use_only_action(action: &str) -> bool {
    matches!(
        action,
        "mouse_move" | "mouse_click" | "mouse_drag" | "key_type" | "key_press" | "screen_capture"
    )
}

fn backend_name(backend: ResolvedBackend) -> &'static str {
    match backend {
        ResolvedBackend::AgentBrowser => "agent_browser",
        ResolvedBackend::RustNative => "rust_native",
        ResolvedBackend::ComputerUse => "computer_use",
    }
}

fn unavailable_action_for_backend_error(action: &str, backend: ResolvedBackend) -> String {
    format!(
        "Action '{action}' is unavailable for backend '{}'",
        backend_name(backend)
    )
}

fn normalize_domains(domains: Vec<String>) -> Vec<String> {
    domains
        .into_iter()
        .map(|d| d.trim().to_lowercase())
        .filter(|d| !d.is_empty())
        .collect()
}

fn endpoint_reachable(endpoint: &reqwest::Url, timeout: Duration) -> bool {
    let host = match endpoint.host_str() {
        Some(host) if !host.is_empty() => host,
        _ => return false,
    };

    let port = match endpoint.port_or_known_default() {
        Some(port) => port,
        None => return false,
    };

    let mut addrs = match (host, port).to_socket_addrs() {
        Ok(addrs) => addrs,
        Err(_) => return false,
    };

    let addr = match addrs.next() {
        Some(addr) => addr,
        None => return false,
    };

    std::net::TcpStream::connect_timeout(&addr, timeout).is_ok()
}

fn extract_host(url_str: &str) -> anyhow::Result<String> {
    // Simple host extraction without url crate
    let url = url_str.trim();
    let without_scheme = url
        .strip_prefix("https://")
        .or_else(|| url.strip_prefix("http://"))
        .or_else(|| url.strip_prefix("file://"))
        .unwrap_or(url);

    // Extract host — handle bracketed IPv6 addresses like [::1]:8080
    let authority = without_scheme.split('/').next().unwrap_or(without_scheme);

    let host = if authority.starts_with('[') {
        // IPv6: take everything up to and including the closing ']'
        authority.find(']').map_or(authority, |i| &authority[..=i])
    } else {
        // IPv4 or hostname: take everything before the port separator
        authority.split(':').next().unwrap_or(authority)
    };

    if host.is_empty() {
        anyhow::bail!("Invalid URL: no host");
    }

    Ok(host.to_lowercase())
}

fn is_private_host(host: &str) -> bool {
    // Strip brackets from IPv6 addresses like [::1]
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    if bare == "localhost" || bare.ends_with(".localhost") {
        return true;
    }

    // .local TLD (mDNS)
    if bare
        .rsplit('.')
        .next()
        .is_some_and(|label| label == "local")
    {
        return true;
    }

    // Parse as IP address to catch all representations (decimal, hex, octal, mapped)
    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => is_non_global_v4(v4),
            std::net::IpAddr::V6(v6) => is_non_global_v6(v6),
        };
    }

    false
}

/// Returns `true` for any IPv4 address that is not globally routable.
fn is_non_global_v4(v4: std::net::Ipv4Addr) -> bool {
    let [a, b, _, _] = v4.octets();
    v4.is_loopback()
        || v4.is_private()
        || v4.is_link_local()
        || v4.is_unspecified()
        || v4.is_broadcast()
        || v4.is_multicast()
        // Shared address space (100.64/10)
        || (a == 100 && (64..=127).contains(&b))
        // Reserved (240.0.0.0/4)
        || a >= 240
        // Documentation (192.0.2.0/24, 198.51.100.0/24, 203.0.113.0/24)
        || (a == 192 && b == 0)
        || (a == 198 && b == 51)
        || (a == 203 && b == 0)
        // Benchmarking (198.18.0.0/15)
        || (a == 198 && (18..=19).contains(&b))
}

/// Returns `true` for any IPv6 address that is not globally routable.
fn is_non_global_v6(v6: std::net::Ipv6Addr) -> bool {
    let segs = v6.segments();
    v6.is_loopback()
        || v6.is_unspecified()
        || v6.is_multicast()
        // Unique-local (fc00::/7) — IPv6 equivalent of RFC 1918
        || (segs[0] & 0xfe00) == 0xfc00
        // Link-local (fe80::/10)
        || (segs[0] & 0xffc0) == 0xfe80
        // IPv4-mapped addresses
        || v6.to_ipv4_mapped().is_some_and(is_non_global_v4)
}

fn allow_all_browser_domains() -> bool {
    matches!(
        std::env::var("OPENHUMAN_BROWSER_ALLOW_ALL")
            .ok()
            .as_deref(),
        Some("1") | Some("true") | Some("TRUE") | Some("yes") | Some("YES")
    )
}

fn host_matches_allowlist(host: &str, allowed: &[String]) -> bool {
    allowed.iter().any(|pattern| {
        if pattern == "*" {
            return true;
        }
        if pattern.starts_with("*.") {
            // Wildcard subdomain match
            let suffix = &pattern[1..]; // ".example.com"
            host.ends_with(suffix) || host == &pattern[2..]
        } else {
            // Exact match or subdomain
            host == pattern || host.ends_with(&format!(".{pattern}"))
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_domains_works() {
        let domains = vec![
            "  Example.COM  ".into(),
            "docs.example.com".into(),
            String::new(),
        ];
        let normalized = normalize_domains(domains);
        assert_eq!(normalized, vec!["example.com", "docs.example.com"]);
    }

    #[test]
    fn extract_host_works() {
        assert_eq!(
            extract_host("https://example.com/path").unwrap(),
            "example.com"
        );
        assert_eq!(
            extract_host("https://Sub.Example.COM:8080/").unwrap(),
            "sub.example.com"
        );
    }

    #[test]
    fn extract_host_handles_ipv6() {
        // IPv6 with brackets (required for URLs with ports)
        assert_eq!(extract_host("https://[::1]/path").unwrap(), "[::1]");
        // IPv6 with brackets and port
        assert_eq!(
            extract_host("https://[2001:db8::1]:8080/path").unwrap(),
            "[2001:db8::1]"
        );
        // IPv6 with brackets, trailing slash
        assert_eq!(extract_host("https://[fe80::1]/").unwrap(), "[fe80::1]");
    }

    #[test]
    fn is_private_host_detects_local() {
        assert!(is_private_host("localhost"));
        assert!(is_private_host("app.localhost"));
        assert!(is_private_host("printer.local"));
        assert!(is_private_host("127.0.0.1"));
        assert!(is_private_host("192.168.1.1"));
        assert!(is_private_host("10.0.0.1"));
        assert!(!is_private_host("example.com"));
        assert!(!is_private_host("google.com"));
    }

    #[test]
    fn is_private_host_blocks_multicast_and_reserved() {
        assert!(is_private_host("224.0.0.1")); // multicast
        assert!(is_private_host("255.255.255.255")); // broadcast
        assert!(is_private_host("100.64.0.1")); // shared address space
        assert!(is_private_host("240.0.0.1")); // reserved
        assert!(is_private_host("192.0.2.1")); // documentation
        assert!(is_private_host("198.51.100.1")); // documentation
        assert!(is_private_host("203.0.113.1")); // documentation
        assert!(is_private_host("198.18.0.1")); // benchmarking
    }

    #[test]
    fn is_private_host_catches_ipv6() {
        assert!(is_private_host("::1"));
        assert!(is_private_host("[::1]"));
        assert!(is_private_host("0.0.0.0"));
    }

    #[test]
    fn is_private_host_catches_mapped_ipv4() {
        // IPv4-mapped IPv6 addresses
        assert!(is_private_host("::ffff:127.0.0.1"));
        assert!(is_private_host("::ffff:10.0.0.1"));
        assert!(is_private_host("::ffff:192.168.1.1"));
    }

    #[test]
    fn is_private_host_catches_ipv6_private_ranges() {
        // Unique-local (fc00::/7)
        assert!(is_private_host("fd00::1"));
        assert!(is_private_host("fc00::1"));
        // Link-local (fe80::/10)
        assert!(is_private_host("fe80::1"));
        // Public IPv6 should pass
        assert!(!is_private_host("2001:db8::1"));
    }

    #[test]
    fn validate_url_blocks_ipv6_ssrf() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new(security, vec!["*".into()], None);
        assert!(tool.validate_url("https://[::1]/").is_err());
        assert!(tool.validate_url("https://[::ffff:127.0.0.1]/").is_err());
        assert!(tool
            .validate_url("https://[::ffff:10.0.0.1]:8080/")
            .is_err());
    }

    #[test]
    fn host_matches_allowlist_exact() {
        let allowed = vec!["example.com".into()];
        assert!(host_matches_allowlist("example.com", &allowed));
        assert!(host_matches_allowlist("sub.example.com", &allowed));
        assert!(!host_matches_allowlist("notexample.com", &allowed));
    }

    #[test]
    fn host_matches_allowlist_wildcard() {
        let allowed = vec!["*.example.com".into()];
        assert!(host_matches_allowlist("sub.example.com", &allowed));
        assert!(host_matches_allowlist("example.com", &allowed));
        assert!(!host_matches_allowlist("other.com", &allowed));
    }

    #[test]
    fn host_matches_allowlist_star() {
        let allowed = vec!["*".into()];
        assert!(host_matches_allowlist("anything.com", &allowed));
        assert!(host_matches_allowlist("example.org", &allowed));
    }

    #[test]
    fn browser_backend_parser_accepts_supported_values() {
        assert_eq!(
            BrowserBackendKind::parse("agent_browser").unwrap(),
            BrowserBackendKind::AgentBrowser
        );
        assert_eq!(
            BrowserBackendKind::parse("rust-native").unwrap(),
            BrowserBackendKind::RustNative
        );
        assert_eq!(
            BrowserBackendKind::parse("computer_use").unwrap(),
            BrowserBackendKind::ComputerUse
        );
        assert_eq!(
            BrowserBackendKind::parse("auto").unwrap(),
            BrowserBackendKind::Auto
        );
    }

    #[test]
    fn browser_backend_parser_rejects_unknown_values() {
        assert!(BrowserBackendKind::parse("playwright").is_err());
    }

    #[test]
    fn browser_tool_default_backend_is_agent_browser() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new(security, vec!["example.com".into()], None);
        assert_eq!(
            tool.configured_backend().unwrap(),
            BrowserBackendKind::AgentBrowser
        );
    }

    #[test]
    fn browser_tool_accepts_auto_backend_config() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new_with_backend(
            security,
            vec!["example.com".into()],
            None,
            "auto".into(),
            true,
            "http://127.0.0.1:9515".into(),
            None,
            ComputerUseConfig::default(),
        );
        assert_eq!(tool.configured_backend().unwrap(), BrowserBackendKind::Auto);
    }

    #[test]
    fn browser_tool_accepts_computer_use_backend_config() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new_with_backend(
            security,
            vec!["example.com".into()],
            None,
            "computer_use".into(),
            true,
            "http://127.0.0.1:9515".into(),
            None,
            ComputerUseConfig::default(),
        );
        assert_eq!(
            tool.configured_backend().unwrap(),
            BrowserBackendKind::ComputerUse
        );
    }

    #[test]
    fn computer_use_endpoint_rejects_public_http_by_default() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new_with_backend(
            security,
            vec!["example.com".into()],
            None,
            "computer_use".into(),
            true,
            "http://127.0.0.1:9515".into(),
            None,
            ComputerUseConfig {
                endpoint: "http://computer-use.example.com/v1/actions".into(),
                ..ComputerUseConfig::default()
            },
        );

        assert!(tool.computer_use_endpoint_url().is_err());
    }

    #[test]
    fn computer_use_endpoint_requires_https_for_public_remote() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new_with_backend(
            security,
            vec!["example.com".into()],
            None,
            "computer_use".into(),
            true,
            "http://127.0.0.1:9515".into(),
            None,
            ComputerUseConfig {
                endpoint: "https://computer-use.example.com/v1/actions".into(),
                allow_remote_endpoint: true,
                ..ComputerUseConfig::default()
            },
        );

        assert!(tool.computer_use_endpoint_url().is_ok());
    }

    #[test]
    fn computer_use_coordinate_validation_applies_limits() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new_with_backend(
            security,
            vec!["example.com".into()],
            None,
            "computer_use".into(),
            true,
            "http://127.0.0.1:9515".into(),
            None,
            ComputerUseConfig {
                max_coordinate_x: Some(100),
                max_coordinate_y: Some(100),
                ..ComputerUseConfig::default()
            },
        );

        assert!(tool
            .validate_coordinate("x", 50, tool.computer_use.max_coordinate_x)
            .is_ok());
        assert!(tool
            .validate_coordinate("x", 101, tool.computer_use.max_coordinate_x)
            .is_err());
        assert!(tool
            .validate_coordinate("y", -1, tool.computer_use.max_coordinate_y)
            .is_err());
    }

    #[test]
    fn browser_tool_name() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new(security, vec!["example.com".into()], None);
        assert_eq!(tool.name(), "browser");
    }

    #[test]
    fn browser_tool_validates_url() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new(security, vec!["example.com".into()], None);

        // Valid
        assert!(tool.validate_url("https://example.com").is_ok());
        assert!(tool.validate_url("https://sub.example.com/path").is_ok());

        // Invalid - not in allowlist
        assert!(tool.validate_url("https://other.com").is_err());

        // Invalid - private host
        assert!(tool.validate_url("https://localhost").is_err());
        assert!(tool.validate_url("https://127.0.0.1").is_err());

        // Invalid - not https
        assert!(tool.validate_url("ftp://example.com").is_err());

        // file:// URLs blocked (local file exfiltration risk)
        assert!(tool.validate_url("file:///tmp/test.html").is_err());
    }

    #[test]
    fn browser_tool_empty_allowlist_blocks() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new(security, vec![], None);
        std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
        assert!(tool.validate_url("https://example.com").is_err());
    }

    #[test]
    fn browser_tool_empty_allowlist_allows_with_env_flag() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserTool::new(security, vec![], None);
        std::env::set_var("OPENHUMAN_BROWSER_ALLOW_ALL", "1");
        assert!(tool.validate_url("https://example.com").is_ok());
        std::env::remove_var("OPENHUMAN_BROWSER_ALLOW_ALL");
    }

    #[test]
    fn computer_use_only_action_detection_is_correct() {
        assert!(is_computer_use_only_action("mouse_move"));
        assert!(is_computer_use_only_action("mouse_click"));
        assert!(is_computer_use_only_action("mouse_drag"));
        assert!(is_computer_use_only_action("key_type"));
        assert!(is_computer_use_only_action("key_press"));
        assert!(is_computer_use_only_action("screen_capture"));
        assert!(!is_computer_use_only_action("open"));
        assert!(!is_computer_use_only_action("snapshot"));
    }

    #[test]
    fn unavailable_action_error_preserves_backend_context() {
        assert_eq!(
            unavailable_action_for_backend_error("mouse_move", ResolvedBackend::AgentBrowser),
            "Action 'mouse_move' is unavailable for backend 'agent_browser'"
        );
        assert_eq!(
            unavailable_action_for_backend_error("mouse_move", ResolvedBackend::RustNative),
            "Action 'mouse_move' is unavailable for backend 'rust_native'"
        );
    }
}
