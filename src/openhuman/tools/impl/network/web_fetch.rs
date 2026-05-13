//! `web_fetch` — fetch a URL and return its text body.
//!
//! Coding-harness baseline tool (issue #1205). Distinct from
//! `http_request` (full method/header surface) and `curl` (writes to
//! disk). `web_fetch` is the single-purpose "GET and read" primitive
//! the agent reaches for when researching: returns the response body
//! as text, capped, with a tiny preamble (status + final URL).

use super::url_guard::{normalize_allowed_domains, validate_url};
use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{PermissionLevel, Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

const DEFAULT_MAX_BYTES: usize = 1_000_000;
const DEFAULT_TIMEOUT_SECS: u64 = 20;

pub struct WebFetchTool {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    max_bytes: usize,
    timeout_secs: u64,
}

impl WebFetchTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        allowed_domains: Vec<String>,
        max_bytes: Option<usize>,
        timeout_secs: Option<u64>,
    ) -> Self {
        Self {
            security,
            allowed_domains: normalize_allowed_domains(allowed_domains),
            max_bytes: max_bytes.unwrap_or(DEFAULT_MAX_BYTES),
            timeout_secs: timeout_secs.unwrap_or(DEFAULT_TIMEOUT_SECS),
        }
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn name(&self) -> &str {
        "web_fetch"
    }

    fn description(&self) -> &str {
        "GET a URL and return its body as text (truncated). Use this for \
         reading docs / READMEs / spec pages. For richer HTTP semantics \
         (POST, custom headers, …) use `http_request`."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": { "type": "string", "description": "Absolute http(s) URL." },
                "max_bytes": {
                    "type": "integer",
                    "description": "Truncate body at this many bytes (default 1_000_000).",
                    "minimum": 1
                }
            },
            "required": ["url"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::ReadOnly
    }

    /// Idempotent GET — safe to fan out across parallel `web_fetch`
    /// calls. Targets that throttle aggressively are the user's
    /// concern; we don't try to second-guess at the tool layer.
    fn is_concurrency_safe(&self, _args: &serde_json::Value) -> bool {
        true
    }

    /// Cap web_fetch results at ~50k chars before they reach the
    /// model. The tool itself already truncates byte-wise via
    /// `max_bytes` (default 1MB), but a 1MB HTML page is still tens
    /// of thousands of tokens — the agent rarely needs that much, and
    /// when it does, `read_file` on a saved copy is the right tool.
    fn max_result_size_chars(&self) -> Option<usize> {
        Some(50_000)
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let raw_url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;
        let max_bytes = args
            .get("max_bytes")
            .and_then(|v| v.as_u64())
            .map(|n| (n as usize).max(1))
            .unwrap_or(self.max_bytes);

        if self.security.is_rate_limited() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: too many actions in the last hour",
            ));
        }
        if !self.security.record_action() {
            return Ok(ToolResult::error(
                "Rate limit exceeded: action budget exhausted",
            ));
        }

        let url = match validate_url(raw_url, &self.allowed_domains) {
            Ok(u) => u,
            Err(e) => return Ok(ToolResult::error(format!("URL rejected: {e}"))),
        };

        // Disable automatic redirect following: reqwest follows up to 10
        // redirects by default, and a redirect target may be on a host
        // outside the allowed-domains list. We surface 3xx responses to
        // the caller so they can decide whether to refetch the new URL.
        let client = match reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .redirect(reqwest::redirect::Policy::none())
            .build()
        {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to build client: {e}"))),
        };

        let resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Request failed: {e}"))),
        };
        let status = resp.status();
        let final_url = resp.url().to_string();
        let location = resp
            .headers()
            .get(reqwest::header::LOCATION)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let body = match resp.text().await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read body: {e}"))),
        };

        if let Some(loc) = &location {
            if status.is_redirection() {
                return Ok(ToolResult::success(format!(
                    "status={} url={} location={loc}\n[redirect not followed — re-call web_fetch with the location URL if it's an allowed domain]",
                    status.as_u16(),
                    final_url
                )));
            }
        }

        let (snippet, truncated) = if body.len() > max_bytes {
            let cut = crate::openhuman::util::floor_char_boundary(&body, max_bytes);
            (&body[..cut], true)
        } else {
            (body.as_str(), false)
        };

        let suffix = if truncated {
            format!("\n[truncated at {max_bytes} bytes]")
        } else {
            String::new()
        };
        let header = format!("status={} url={}\n", status.as_u16(), final_url);
        Ok(ToolResult::success(format!("{header}{snippet}{suffix}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::security::{AutonomyLevel, SecurityPolicy};

    fn test_security() -> Arc<SecurityPolicy> {
        Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            workspace_dir: std::env::temp_dir(),
            ..SecurityPolicy::default()
        })
    }

    #[test]
    fn web_fetch_name_and_schema() {
        let tool = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
        assert_eq!(tool.name(), "web_fetch");
        let schema = tool.parameters_schema();
        assert!(schema["properties"]["url"].is_object());
        assert!(schema["required"]
            .as_array()
            .unwrap()
            .contains(&json!("url")));
    }

    #[tokio::test]
    async fn web_fetch_rejects_disallowed_domain() {
        let tool = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
        let result = tool
            .execute(json!({ "url": "https://evil.test/path" }))
            .await
            .unwrap();
        assert!(result.is_error);
        assert!(result.output().contains("URL rejected"));
    }

    #[tokio::test]
    async fn web_fetch_rejects_invalid_url() {
        let tool = WebFetchTool::new(test_security(), vec!["example.com".into()], None, None);
        let result = tool.execute(json!({ "url": "not-a-url" })).await.unwrap();
        assert!(result.is_error);
    }

    #[test]
    fn test_web_fetch_truncation_utf8() {
        // Mock body with multi-byte char exactly at budget
        let body = "Hello 🦀 World"; // 🦀 is at index 6-9
        let max_bytes = 8;
        // Should truncate at index 6
        let cut = crate::openhuman::util::floor_char_boundary(body, max_bytes);
        assert_eq!(cut, 6);
        assert_eq!(&body[..cut], "Hello ");
    }
}
