//! `curl` — download files from the web to a path under the workspace.
//!
//! Distinct from `http_request`: instead of returning the body inline
//! (size-capped), `curl` streams to disk with a hard byte ceiling. Same
//! SSRF/allowlist guards (shared via `url_guard`), shares
//! `http_request.allowed_domains` so there is one allowlist to reason
//! about.

use super::url_guard::{normalize_allowed_domains, validate_url};
use crate::openhuman::security::SecurityPolicy;
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

    fn validate_url(&self, raw_url: &str) -> anyhow::Result<String> {
        validate_url(raw_url, &self.allowed_domains)
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
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
        }
        if !self.security.record_action() {
            tracing::debug!(target: "[curl]", url = %url, "blocked: rate limit");
            return Ok(ToolResult::error("Action blocked: rate limit exceeded"));
        }

        let url = match self.validate_url(url) {
            Ok(v) => v,
            Err(e) => {
                tracing::debug!(target: "[curl]", url = %url, reason = %e, "url validation failed");
                return Ok(ToolResult::error(e.to_string()));
            }
        };

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
mod tests {
    use super::*;
    use crate::openhuman::security::SecurityPolicy;
    use tempfile::TempDir;

    fn tool(tmp: &TempDir, allow: Vec<&str>) -> CurlTool {
        CurlTool::new(
            Arc::new(SecurityPolicy::default()),
            allow.into_iter().map(String::from).collect(),
            tmp.path().to_path_buf(),
            "downloads".into(),
            1024 * 1024,
            30,
        )
    }

    #[test]
    fn sanitize_dest_subdir_strips_absolute_paths() {
        assert_eq!(sanitize_dest_subdir("/etc/passwd"), "etc/passwd");
        assert_eq!(sanitize_dest_subdir("//foo"), "foo");
    }

    #[test]
    fn sanitize_dest_subdir_strips_parent_segments() {
        assert_eq!(sanitize_dest_subdir("../../etc"), "etc");
        assert_eq!(sanitize_dest_subdir("a/../b"), "a/b");
    }

    #[test]
    fn sanitize_dest_subdir_falls_back_to_downloads() {
        assert_eq!(sanitize_dest_subdir(""), "downloads");
        assert_eq!(sanitize_dest_subdir("   "), "downloads");
        assert_eq!(sanitize_dest_subdir(".."), "downloads");
        assert_eq!(sanitize_dest_subdir("/"), "downloads");
    }

    #[test]
    fn sanitize_dest_subdir_keeps_normal_paths() {
        assert_eq!(sanitize_dest_subdir("downloads"), "downloads");
        assert_eq!(sanitize_dest_subdir("artifacts/build"), "artifacts/build");
    }

    #[test]
    fn new_sanitizes_malicious_dest_subdir() {
        let tmp = TempDir::new().unwrap();
        let t = CurlTool::new(
            Arc::new(SecurityPolicy::default()),
            vec!["example.com".into()],
            tmp.path().to_path_buf(),
            "../../etc".into(),
            1024,
            30,
        );
        let resolved = t.resolve_dest("file.txt").unwrap();
        // Sanitizer reduced "../../etc" to "etc"; resolution must stay under workspace.
        assert!(resolved.starts_with(tmp.path().join("etc")));
        assert!(resolved.starts_with(tmp.path()));
    }

    #[test]
    fn resolve_dest_normal() {
        let tmp = TempDir::new().unwrap();
        let t = tool(&tmp, vec!["example.com"]);
        let p = t.resolve_dest("foo/bar.txt").unwrap();
        assert!(p.starts_with(tmp.path().join("downloads")));
        assert!(p.ends_with("foo/bar.txt"));
    }

    #[test]
    fn resolve_dest_rejects_absolute() {
        let tmp = TempDir::new().unwrap();
        let t = tool(&tmp, vec!["example.com"]);
        let err = t.resolve_dest("/etc/passwd").unwrap_err().to_string();
        assert!(err.contains("relative"));
    }

    #[test]
    fn resolve_dest_rejects_parent_dir() {
        let tmp = TempDir::new().unwrap();
        let t = tool(&tmp, vec!["example.com"]);
        let err = t.resolve_dest("../etc/passwd").unwrap_err().to_string();
        assert!(err.contains(".."));
    }

    #[test]
    fn resolve_dest_rejects_nested_parent_dir() {
        let tmp = TempDir::new().unwrap();
        let t = tool(&tmp, vec!["example.com"]);
        let err = t.resolve_dest("a/../../b").unwrap_err().to_string();
        assert!(err.contains(".."));
    }

    #[test]
    fn resolve_dest_rejects_empty() {
        let tmp = TempDir::new().unwrap();
        let t = tool(&tmp, vec!["example.com"]);
        assert!(t.resolve_dest("").is_err());
        assert!(t.resolve_dest("   ").is_err());
    }

    #[test]
    fn default_filename_from_url_basic() {
        assert_eq!(
            CurlTool::default_filename_from_url("https://example.com/foo/bar.zip"),
            "bar.zip"
        );
    }

    #[test]
    fn default_filename_from_url_query_stripped() {
        assert_eq!(
            CurlTool::default_filename_from_url("https://example.com/file.tar.gz?token=x"),
            "file.tar.gz"
        );
    }

    #[test]
    fn default_filename_from_url_root_falls_back() {
        assert_eq!(
            CurlTool::default_filename_from_url("https://example.com/"),
            "download.bin"
        );
    }

    #[tokio::test]
    async fn execute_blocks_when_rate_limited() {
        let tmp = TempDir::new().unwrap();
        let security = Arc::new(SecurityPolicy {
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        });
        let t = CurlTool::new(
            security,
            vec!["example.com".into()],
            tmp.path().into(),
            "downloads".into(),
            1024,
            30,
        );
        let result = t
            .execute(serde_json::json!({"url": "https://example.com/x"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("rate limit"));
    }

    /// Live integration smoke: downloads example.com (a tiny, stable
    /// public page). Gated behind `OPENHUMAN_CURL_LIVE_TEST=1` so CI /
    /// offline runs don't depend on the network.
    #[tokio::test]
    async fn live_download_example_com() {
        if std::env::var("OPENHUMAN_CURL_LIVE_TEST").ok().as_deref() != Some("1") {
            return;
        }
        let tmp = TempDir::new().unwrap();
        let t = tool(&tmp, vec!["example.com"]);
        let result = t
            .execute(serde_json::json!({
                "url": "https://example.com/",
                "dest_path": "example.html"
            }))
            .await
            .unwrap();
        assert!(!result.is_error, "live curl errored: {}", result.output());
        let payload: serde_json::Value = serde_json::from_str(&result.output()).unwrap();
        let bytes = payload["bytes_written"].as_u64().unwrap();
        assert!(bytes > 100, "unexpectedly small download: {bytes} bytes");
        let path = payload["path"].as_str().unwrap();
        let content = std::fs::read_to_string(path).unwrap();
        assert!(content.to_lowercase().contains("example domain"));
    }

    #[tokio::test]
    async fn execute_rejects_allowlist_miss() {
        let tmp = TempDir::new().unwrap();
        let t = tool(&tmp, vec!["example.com"]);
        let result = t
            .execute(serde_json::json!({"url": "https://other.example.org/x"}))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("allowed_domains"));
    }
}
