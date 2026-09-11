//! `curl` — download files from the web to a path under the workspace.
//!
//! Distinct from `http_request`: instead of returning the body inline
//! (size-capped), `curl` streams to disk with a hard byte ceiling. Same
//! SSRF/allowlist guards (shared via `url_guard`), shares
//! `http_request.allowed_domains` so there is one allowlist to reason
//! about.

use super::url_guard::{normalize_allowed_domains, validate_url_with_dns_check};
use crate::openhuman::security::{CommandClass, GateDecision, SecurityPolicy};
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::fs;
use tokio::io::AsyncWriteExt;

pub struct CurlTool {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    workspace_dir: PathBuf,
    dest_subdir: String,
    max_download_bytes: u64,
    timeout_secs: u64,
}

impl CurlTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        allowed_domains: Vec<String>,
        workspace_dir: PathBuf,
        dest_subdir: String,
        max_download_bytes: u64,
        timeout_secs: u64,
    ) -> Self {
        Self {
            security,
            allowed_domains: normalize_allowed_domains(allowed_domains),
            workspace_dir,
            dest_subdir: sanitize_dest_subdir(&dest_subdir),
            max_download_bytes,
            timeout_secs,
        }
    }

    /// Resolve a user-supplied dest path to an absolute path inside
    /// `<workspace>/<dest_subdir>`. Rejects absolute paths, `..`
    /// segments, and any other escape attempts.
    fn resolve_dest(&self, dest: &str) -> anyhow::Result<PathBuf> {
        let trimmed = dest.trim();
        if trimmed.is_empty() {
            anyhow::bail!("dest_path cannot be empty");
        }

        let p = Path::new(trimmed);
        if p.is_absolute() {
            anyhow::bail!("dest_path must be relative — got absolute path");
        }

        for component in p.components() {
            match component {
                Component::Normal(_) => {}
                Component::CurDir => {}
                Component::ParentDir => {
                    anyhow::bail!("dest_path may not contain '..'");
                }
                Component::Prefix(_) | Component::RootDir => {
                    anyhow::bail!("dest_path must be relative");
                }
            }
        }

        let root = self.workspace_dir.join(&self.dest_subdir);
        let resolved = root.join(p);

        // Belt-and-braces: ensure the resolved path still lives under root.
        // Lexical check is sufficient because we already rejected `..`.
        if !resolved.starts_with(&root) {
            anyhow::bail!("dest_path resolves outside the downloads root");
        }

        Ok(resolved)
    }

    async fn validate_url(&self, raw_url: &str) -> anyhow::Result<String> {
        validate_url_with_dns_check(raw_url, &self.allowed_domains).await
    }

    fn default_filename_from_url(url: &str) -> String {
        let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
        let path_part = after_scheme.split_once('/').map(|(_, p)| p).unwrap_or("");
        let last = path_part
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("");
        let cleaned: String = last
            .chars()
            .filter(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_'))
            .collect();
        if cleaned.is_empty() {
            "download.bin".into()
        } else {
            cleaned
        }
    }
}

#[async_trait]
impl Tool for CurlTool {
    fn name(&self) -> &str {
        "curl"
    }

    fn description(&self) -> &str {
        "Download a file from an http(s) URL into the workspace. The body is streamed to disk \
        with a hard byte ceiling. Same allowlist as `http_request`. Returns the saved path, \
        bytes written, content-type, and SHA-256 of the file."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP or HTTPS URL of the file to download"
                },
                "dest_path": {
                    "type": "string",
                    "description": "Destination path relative to the downloads root inside the workspace. No '..' or absolute paths. If omitted, the filename is inferred from the URL."
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers (e.g. {\"Authorization\": \"Bearer …\"})",
                    "default": {}
                }
            },
            "required": ["url"]
        })
    }

    /// Downloading from the network is the always-ask `Network` bucket — it
    /// prompts the human in both ask-before-edit and Full; read-only is blocked
    /// in `execute`.
    fn external_effect_with_args(&self, _args: &serde_json::Value) -> bool {
        self.security.gate_decision(CommandClass::Network) == GateDecision::Prompt
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::Write
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

        let dest_arg = args.get("dest_path").and_then(|v| v.as_str());
        let headers_val = args
            .get("headers")
            .cloned()
            .unwrap_or_else(|| serde_json::json!({}));

        if !self.security.can_act() {
            tracing::debug!(target: "[curl]", url = %url, "blocked: autonomy read-only");
            return Ok(ToolResult::error(
                "[policy-blocked] Action blocked: autonomy is read-only",
            ));
        }
        if !self.security.record_action() {
            tracing::debug!(target: "[curl]", url = %url, "blocked: rate limit");
            return Ok(ToolResult::error("Action blocked: rate limit exceeded"));
        }

        // Local-only enforcement (privacy epic S7, #4441): mirror the read-only
        // `can_act()` deny above — under LocalOnly, refuse the download before
        // URL validation / DNS so nothing leaves the device.
        {
            let host = reqwest::Url::parse(url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            if let Some(msg) = crate::openhuman::security::egress::local_only_tool_block(
                &crate::openhuman::security::egress::EgressDescriptor::network_fetch(host.clone()),
            ) {
                // Log only the host, never the full URL: a raw URL can carry
                // secrets in its query string (pre-signed links, tokens). The
                // gate helper already logs `desc.service` (host only) too.
                tracing::debug!(target: "[curl]", host = %host, "blocked: local-only privacy mode");
                return Ok(ToolResult::error(msg));
            }
        }

        let url = match self.validate_url(url).await {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(target: "[curl]", url = %url, reason = %e, "url validation failed");
                return Ok(ToolResult::error(e.to_string()));
            }
        };

        // Egress spine (privacy epic S2/S7, #4436/#4441): a curl download
        // contacts an external host — disclose the destination (and that custom
        // headers ride along) before the request. Enforcement already ran at the
        // top of `execute`; this is the observe-only disclosure for permitted
        // downloads.
        {
            use crate::openhuman::security::egress::{DataKind, EgressDescriptor};
            let host = reqwest::Url::parse(&url)
                .ok()
                .and_then(|u| u.host_str().map(str::to_string))
                .unwrap_or_else(|| "unknown".to_string());
            let has_headers = headers_val
                .as_object()
                .map(|h| !h.is_empty())
                .unwrap_or(false);
            let mut desc = EgressDescriptor::network_fetch(host);
            if has_headers {
                desc = desc.with_data_kind(DataKind::Metadata);
            }
            crate::openhuman::security::egress::emit_external_transfer(desc);
        }

        let dest = match dest_arg {
            Some(d) => d.to_string(),
            None => Self::default_filename_from_url(&url),
        };
        let dest_path = match self.resolve_dest(&dest) {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(target: "[curl]", url = %url, dest = %dest, reason = %e, "dest_path rejected");
                return Ok(ToolResult::error(e.to_string()));
            }
        };

        if let Some(parent) = dest_path.parent() {
            if let Err(e) = fs::create_dir_all(parent).await {
                tracing::error!(target: "[curl]", url = %url, dest = %dest_path.display(), reason = %e, "create_dir_all failed");
                return Ok(ToolResult::error(format!(
                    "Failed to create destination directory: {e}"
                )));
            }
        }

        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none());
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.curl");
        let client = match builder.build() {
            Ok(c) => c,
            Err(e) => {
                tracing::error!(target: "[curl]", reason = %e, "HTTP client build failed");
                return Ok(ToolResult::error(format!("HTTP client build failed: {e}")));
            }
        };

        let mut request = client.get(&url);
        if let Some(obj) = headers_val.as_object() {
            for (k, v) in obj {
                if let Some(s) = v.as_str() {
                    request = request.header(k, s);
                }
            }
        }

        tracing::debug!(target: "[curl]", url = %url, dest = %dest_path.display(), "starting download");

        let response = match request.send().await {
            Ok(r) => r,
            Err(e) => {
                tracing::error!(target: "[curl]", url = %url, reason = %e, "request send failed");
                return Ok(ToolResult::error(format!("Request failed: {e}")));
            }
        };

        let status = response.status();
        if !status.is_success() {
            tracing::debug!(target: "[curl]", url = %url, status = %status.as_u16(), "non-success HTTP status");
            return Ok(ToolResult::error(format!(
                "HTTP {} {}",
                status.as_u16(),
                status.canonical_reason().unwrap_or("Unknown")
            )));
        }

        let content_type = response
            .headers()
            .get(reqwest::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("application/octet-stream")
            .to_string();

        let mut file = match fs::File::create(&dest_path).await {
            Ok(f) => f,
            Err(e) => {
                tracing::error!(target: "[curl]", dest = %dest_path.display(), reason = %e, "fs::File::create failed");
                return Ok(ToolResult::error(format!(
                    "Failed to create destination file: {e}"
                )));
            }
        };

        let mut hasher = Sha256::new();
        let mut bytes_written: u64 = 0;
        let mut stream = response.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = match chunk {
                Ok(c) => c,
                Err(e) => {
                    drop(file);
                    if let Err(rm) = fs::remove_file(&dest_path).await {
                        tracing::debug!(target: "[curl]", dest = %dest_path.display(), reason = %rm, "cleanup remove_file failed");
                    }
                    tracing::error!(target: "[curl]", url = %url, bytes_written, reason = %e, "stream error");
                    return Ok(ToolResult::error(format!("Stream error: {e}")));
                }
            };

            if bytes_written.saturating_add(chunk.len() as u64) > self.max_download_bytes {
                let _ = file.flush().await;
                drop(file);
                if let Err(rm) = fs::remove_file(&dest_path).await {
                    tracing::debug!(target: "[curl]", dest = %dest_path.display(), reason = %rm, "cleanup remove_file failed");
                }
                tracing::error!(target: "[curl]", url = %url, bytes_written, max = self.max_download_bytes, "size cap exceeded — download aborted");
                return Ok(ToolResult::error(format!(
                    "Download exceeded max_download_bytes ({} bytes)",
                    self.max_download_bytes
                )));
            }

            if let Err(e) = file.write_all(&chunk).await {
                drop(file);
                if let Err(rm) = fs::remove_file(&dest_path).await {
                    tracing::debug!(target: "[curl]", dest = %dest_path.display(), reason = %rm, "cleanup remove_file failed");
                }
                tracing::error!(target: "[curl]", dest = %dest_path.display(), bytes_written, reason = %e, "write_all failed");
                return Ok(ToolResult::error(format!("Write failed: {e}")));
            }
            hasher.update(&chunk);
            bytes_written += chunk.len() as u64;
        }

        if let Err(e) = file.flush().await {
            drop(file);
            if let Err(rm) = fs::remove_file(&dest_path).await {
                tracing::debug!(target: "[curl]", dest = %dest_path.display(), reason = %rm, "cleanup remove_file failed");
            }
            tracing::error!(target: "[curl]", dest = %dest_path.display(), bytes_written, reason = %e, "flush failed");
            return Ok(ToolResult::error(format!("Flush failed: {e}")));
        }

        let sha256 = format!("{:x}", hasher.finalize());

        tracing::debug!(
            target: "[curl]",
            url = %url,
            dest = %dest_path.display(),
            bytes = bytes_written,
            content_type = %content_type,
            sha256 = %sha256,
            "download complete"
        );

        let payload = serde_json::json!({
            "path": dest_path.display().to_string(),
            "bytes_written": bytes_written,
            "content_type": content_type,
            "sha256": sha256,
        });
        Ok(ToolResult::success(payload.to_string()))
    }
}

/// Sanitize the configured `dest_subdir` so a malicious or misconfigured
/// `[curl].dest_subdir` cannot escape the workspace via absolute paths
/// or `..` segments. Drops disallowed components rather than panicking;
/// falls back to `"downloads"` if everything is filtered out.
fn sanitize_dest_subdir(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return "downloads".into();
    }
    let p = Path::new(trimmed);
    let mut buf = PathBuf::new();
    for component in p.components() {
        match component {
            Component::Normal(c) => buf.push(c),
            // Drop everything else: absolute roots, prefixes, parent dirs, cur dirs.
            _ => continue,
        }
    }
    if buf.as_os_str().is_empty() {
        return "downloads".into();
    }
    buf.to_string_lossy().into_owned()
}

#[cfg(test)]
#[path = "curl_tests.rs"]
mod tests;
