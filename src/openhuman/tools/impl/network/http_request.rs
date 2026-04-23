use crate::openhuman::security::SecurityPolicy;
use crate::openhuman::tools::traits::{Tool, ToolResult};
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;
use std::time::Duration;

/// HTTP request tool for API interactions.
/// Supports GET, POST, PUT, DELETE methods with configurable security.
pub struct HttpRequestTool {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
    max_response_size: usize,
    timeout_secs: u64,
}

impl HttpRequestTool {
    pub fn new(
        security: Arc<SecurityPolicy>,
        allowed_domains: Vec<String>,
        max_response_size: usize,
        timeout_secs: u64,
    ) -> Self {
        Self {
            security,
            allowed_domains: normalize_allowed_domains(allowed_domains),
            max_response_size,
            timeout_secs,
        }
    }

    fn validate_url(&self, raw_url: &str) -> anyhow::Result<String> {
        let url = raw_url.trim();

        if url.is_empty() {
            anyhow::bail!("URL cannot be empty");
        }

        if url.chars().any(char::is_whitespace) {
            anyhow::bail!("URL cannot contain whitespace");
        }

        if !url.starts_with("http://") && !url.starts_with("https://") {
            anyhow::bail!("Only http:// and https:// URLs are allowed");
        }

        if self.allowed_domains.is_empty() {
            anyhow::bail!(
                "HTTP request tool is enabled but no allowed_domains are configured. Add [http_request].allowed_domains in config.toml"
            );
        }

        let host = extract_host(url)?;

        if is_private_or_local_host(&host) {
            anyhow::bail!("Blocked local/private host: {host}");
        }

        if !host_matches_allowlist(&host, &self.allowed_domains) {
            anyhow::bail!("Host '{host}' is not in http_request.allowed_domains");
        }

        Ok(url.to_string())
    }

    fn validate_method(&self, method: &str) -> anyhow::Result<reqwest::Method> {
        match method.to_uppercase().as_str() {
            "GET" => Ok(reqwest::Method::GET),
            "POST" => Ok(reqwest::Method::POST),
            "PUT" => Ok(reqwest::Method::PUT),
            "DELETE" => Ok(reqwest::Method::DELETE),
            "PATCH" => Ok(reqwest::Method::PATCH),
            "HEAD" => Ok(reqwest::Method::HEAD),
            "OPTIONS" => Ok(reqwest::Method::OPTIONS),
            _ => anyhow::bail!("Unsupported HTTP method: {method}. Supported: GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS"),
        }
    }

    fn parse_headers(&self, headers: &serde_json::Value) -> Vec<(String, String)> {
        let mut result = Vec::new();
        if let Some(obj) = headers.as_object() {
            for (key, value) in obj {
                if let Some(str_val) = value.as_str() {
                    result.push((key.clone(), str_val.to_string()));
                }
            }
        }
        result
    }

    fn redact_headers_for_display(headers: &[(String, String)]) -> Vec<(String, String)> {
        headers
            .iter()
            .map(|(key, value)| {
                let lower = key.to_lowercase();
                let is_sensitive = lower.contains("authorization")
                    || lower.contains("api-key")
                    || lower.contains("apikey")
                    || lower.contains("token")
                    || lower.contains("secret");
                if is_sensitive {
                    (key.clone(), "***REDACTED***".into())
                } else {
                    (key.clone(), value.clone())
                }
            })
            .collect()
    }

    async fn execute_request(
        &self,
        url: &str,
        method: reqwest::Method,
        headers: Vec<(String, String)>,
        body: Option<&str>,
    ) -> anyhow::Result<reqwest::Response> {
        let builder = reqwest::Client::builder()
            .timeout(Duration::from_secs(self.timeout_secs))
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none());
        let builder =
            crate::openhuman::config::apply_runtime_proxy_to_builder(builder, "tool.http_request");
        let client = builder.build()?;

        let mut request = client.request(method, url);

        for (key, value) in headers {
            request = request.header(&key, &value);
        }

        if let Some(body_str) = body {
            request = request.body(body_str.to_string());
        }

        Ok(request.send().await?)
    }

    fn truncate_response(&self, text: &str) -> String {
        if text.len() > self.max_response_size {
            let mut truncated = text
                .chars()
                .take(self.max_response_size)
                .collect::<String>();
            truncated.push_str("\n\n... [Response truncated due to size limit] ...");
            truncated
        } else {
            text.to_string()
        }
    }
}

#[async_trait]
impl Tool for HttpRequestTool {
    fn name(&self) -> &str {
        "http_request"
    }

    fn description(&self) -> &str {
        "Make HTTP requests to external APIs. Supports GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS methods. \
        Security constraints: allowlist-only domains, no local/private hosts, configurable timeout and response size limits."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTP or HTTPS URL to request"
                },
                "method": {
                    "type": "string",
                    "description": "HTTP method (GET, POST, PUT, DELETE, PATCH, HEAD, OPTIONS)",
                    "default": "GET"
                },
                "headers": {
                    "type": "object",
                    "description": "Optional HTTP headers as key-value pairs (e.g., {\"Authorization\": \"Bearer token\", \"Content-Type\": \"application/json\"})",
                    "default": {}
                },
                "body": {
                    "type": "string",
                    "description": "Optional request body (for POST, PUT, PATCH requests)"
                }
            },
            "required": ["url"]
        })
    }

    async fn execute(&self, args: serde_json::Value) -> anyhow::Result<ToolResult> {
        let url = args
            .get("url")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("Missing 'url' parameter"))?;

        let method_str = args.get("method").and_then(|v| v.as_str()).unwrap_or("GET");
        let headers_val = args.get("headers").cloned().unwrap_or(json!({}));
        let body = args.get("body").and_then(|v| v.as_str());

        if !self.security.can_act() {
            return Ok(ToolResult::error("Action blocked: autonomy is read-only"));
        }

        if !self.security.record_action() {
            return Ok(ToolResult::error("Action blocked: rate limit exceeded"));
        }

        let url = match self.validate_url(url) {
            Ok(v) => v,
            Err(e) => return Ok(ToolResult::error(e.to_string())),
        };

        let method = match self.validate_method(method_str) {
            Ok(m) => m,
            Err(e) => return Ok(ToolResult::error(e.to_string())),
        };

        let request_headers = self.parse_headers(&headers_val);

        match self
            .execute_request(&url, method, request_headers, body)
            .await
        {
            Ok(response) => {
                let status = response.status();
                let status_code = status.as_u16();

                // Get response headers (redact sensitive ones)
                let response_headers = response.headers().iter();
                let headers_text = response_headers
                    .map(|(k, _)| {
                        let is_sensitive = k.as_str().to_lowercase().contains("set-cookie");
                        if is_sensitive {
                            format!("{}: ***REDACTED***", k.as_str())
                        } else {
                            format!("{}: {:?}", k.as_str(), k.as_str())
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(", ");

                // Get response body with size limit
                let response_text = match response.text().await {
                    Ok(text) => self.truncate_response(&text),
                    Err(e) => format!("[Failed to read response body: {e}]"),
                };

                let output = format!(
                    "Status: {} {}\nResponse Headers: {}\n\nResponse Body:\n{}",
                    status_code,
                    status.canonical_reason().unwrap_or("Unknown"),
                    headers_text,
                    response_text
                );

                if status.is_success() {
                    Ok(ToolResult::success(output))
                } else {
                    Ok(ToolResult::error(format!("HTTP {}", status_code)))
                }
            }
            Err(e) => Ok(ToolResult::error(format!("HTTP request failed: {e}"))),
        }
    }
}

// Helper functions similar to browser_open.rs

fn normalize_allowed_domains(domains: Vec<String>) -> Vec<String> {
    let mut normalized = domains
        .into_iter()
        .filter_map(|d| normalize_domain(&d))
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn normalize_domain(raw: &str) -> Option<String> {
    let mut d = raw.trim().to_lowercase();
    if d.is_empty() {
        return None;
    }

    if let Some(stripped) = d.strip_prefix("https://") {
        d = stripped.to_string();
    } else if let Some(stripped) = d.strip_prefix("http://") {
        d = stripped.to_string();
    }

    if let Some((host, _)) = d.split_once('/') {
        d = host.to_string();
    }

    d = d.trim_start_matches('.').trim_end_matches('.').to_string();

    if let Some((host, _)) = d.split_once(':') {
        d = host.to_string();
    }

    if d.is_empty() || d.chars().any(char::is_whitespace) {
        return None;
    }

    Some(d)
}

fn extract_host(url: &str) -> anyhow::Result<String> {
    let rest = url
        .strip_prefix("http://")
        .or_else(|| url.strip_prefix("https://"))
        .ok_or_else(|| anyhow::anyhow!("Only http:// and https:// URLs are allowed"))?;

    let authority = rest
        .split(['/', '?', '#'])
        .next()
        .ok_or_else(|| anyhow::anyhow!("Invalid URL"))?;

    if authority.is_empty() {
        anyhow::bail!("URL must include a host");
    }

    if authority.contains('@') {
        anyhow::bail!("URL userinfo is not allowed");
    }

    if authority.starts_with('[') {
        anyhow::bail!("IPv6 hosts are not supported in http_request");
    }

    let host = authority
        .split(':')
        .next()
        .unwrap_or_default()
        .trim()
        .trim_end_matches('.')
        .to_lowercase();

    if host.is_empty() {
        anyhow::bail!("URL must include a valid host");
    }

    Ok(host)
}

fn host_matches_allowlist(host: &str, allowed_domains: &[String]) -> bool {
    allowed_domains.iter().any(|domain| {
        host == domain
            || host
                .strip_suffix(domain)
                .is_some_and(|prefix| prefix.ends_with('.'))
    })
}

fn is_private_or_local_host(host: &str) -> bool {
    // Strip brackets from IPv6 addresses like [::1]
    let bare = host
        .strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host);

    let has_local_tld = bare
        .rsplit('.')
        .next()
        .is_some_and(|label| label == "local");

    if bare == "localhost" || bare.ends_with(".localhost") || has_local_tld {
        return true;
    }

    if let Ok(ip) = bare.parse::<std::net::IpAddr>() {
        return match ip {
            std::net::IpAddr::V4(v4) => is_non_global_v4(v4),
            std::net::IpAddr::V6(v6) => is_non_global_v6(v6),
        };
    }

    false
}

/// Returns true if the IPv4 address is not globally routable.
fn is_non_global_v4(v4: std::net::Ipv4Addr) -> bool {
    let [a, b, c, _] = v4.octets();
    v4.is_loopback()                       // 127.0.0.0/8
        || v4.is_private()                 // 10/8, 172.16/12, 192.168/16
        || v4.is_link_local()              // 169.254.0.0/16
        || v4.is_unspecified()             // 0.0.0.0
        || v4.is_broadcast()              // 255.255.255.255
        || v4.is_multicast()              // 224.0.0.0/4
        || (a == 100 && (64..=127).contains(&b)) // Shared address space (RFC 6598)
        || a >= 240                        // Reserved (240.0.0.0/4, except broadcast)
        || (a == 192 && b == 0 && (c == 0 || c == 2)) // IETF assignments + TEST-NET-1
        || (a == 198 && b == 51)           // Documentation (198.51.100.0/24)
        || (a == 203 && b == 0)            // Documentation (203.0.113.0/24)
        || (a == 198 && (18..=19).contains(&b)) // Benchmarking (198.18.0.0/15)
}

/// Returns true if the IPv6 address is not globally routable.
fn is_non_global_v6(v6: std::net::Ipv6Addr) -> bool {
    let segs = v6.segments();
    v6.is_loopback()                       // ::1
        || v6.is_unspecified()             // ::
        || v6.is_multicast()              // ff00::/8
        || (segs[0] & 0xfe00) == 0xfc00   // Unique-local (fc00::/7)
        || (segs[0] & 0xffc0) == 0xfe80   // Link-local (fe80::/10)
        || (segs[0] == 0x2001 && segs[1] == 0x0db8) // Documentation (2001:db8::/32)
        || v6.to_ipv4_mapped().is_some_and(is_non_global_v4)
}

#[cfg(test)]
#[path = "http_request_tests.rs"]
mod tests;
