use super::traits::{Tool, ToolResult};
use crate::alphahuman::security::SecurityPolicy;
use async_trait::async_trait;
use serde_json::json;
use std::sync::Arc;

/// Open approved HTTPS URLs in Brave Browser (no scraping, no DOM automation).
pub struct BrowserOpenTool {
    security: Arc<SecurityPolicy>,
    allowed_domains: Vec<String>,
}

impl BrowserOpenTool {
    pub fn new(security: Arc<SecurityPolicy>, allowed_domains: Vec<String>) -> Self {
        Self {
            security,
            allowed_domains: normalize_allowed_domains(allowed_domains),
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

        if !url.starts_with("https://") {
            anyhow::bail!("Only https:// URLs are allowed");
        }

        if self.allowed_domains.is_empty() {
            anyhow::bail!(
                "Browser tool is enabled but no allowed_domains are configured. Add [browser].allowed_domains in config.toml"
            );
        }

        let host = extract_host(url)?;

        if is_private_or_local_host(&host) {
            anyhow::bail!("Blocked local/private host: {host}");
        }

        if !host_matches_allowlist(&host, &self.allowed_domains) {
            anyhow::bail!("Host '{host}' is not in browser.allowed_domains");
        }

        Ok(url.to_string())
    }
}

#[async_trait]
impl Tool for BrowserOpenTool {
    fn name(&self) -> &str {
        "browser_open"
    }

    fn description(&self) -> &str {
        "Open an approved HTTPS URL in Brave Browser. Security constraints: allowlist-only domains, no local/private hosts, no scraping."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "HTTPS URL to open in Brave Browser"
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

        let url = match self.validate_url(url) {
            Ok(v) => v,
            Err(e) => {
                return Ok(ToolResult {
                    success: false,
                    output: String::new(),
                    error: Some(e.to_string()),
                })
            }
        };

        match open_in_brave(&url).await {
            Ok(()) => Ok(ToolResult {
                success: true,
                output: format!("Opened in Brave: {url}"),
                error: None,
            }),
            Err(e) => Ok(ToolResult {
                success: false,
                output: String::new(),
                error: Some(format!("Failed to open Brave Browser: {e}")),
            }),
        }
    }
}

async fn open_in_brave(url: &str) -> anyhow::Result<()> {
    #[cfg(target_os = "macos")]
    {
        for app in ["Brave Browser", "Brave"] {
            let status = tokio::process::Command::new("open")
                .arg("-a")
                .arg(app)
                .arg(url)
                .status()
                .await;

            if let Ok(s) = status {
                if s.success() {
                    return Ok(());
                }
            }
        }
        anyhow::bail!(
            "Brave Browser was not found (tried macOS app names 'Brave Browser' and 'Brave')"
        );
    }

    #[cfg(target_os = "linux")]
    {
        let mut last_error = String::new();
        for cmd in ["brave-browser", "brave"] {
            match tokio::process::Command::new(cmd).arg(url).status().await {
                Ok(status) if status.success() => return Ok(()),
                Ok(status) => {
                    last_error = format!("{cmd} exited with status {status}");
                }
                Err(e) => {
                    last_error = format!("{cmd} not runnable: {e}");
                }
            }
        }
        anyhow::bail!("{last_error}");
    }

    #[cfg(target_os = "windows")]
    {
        let status = tokio::process::Command::new("cmd")
            .args(["/C", "start", "", "brave", url])
            .status()
            .await?;

        if status.success() {
            return Ok(());
        }

        anyhow::bail!("cmd start brave exited with status {status}");
    }

    #[cfg(not(any(target_os = "macos", target_os = "linux", target_os = "windows")))]
    {
        let _ = url;
        anyhow::bail!("browser_open is not supported on this OS");
    }
}

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
        .strip_prefix("https://")
        .ok_or_else(|| anyhow::anyhow!("Only https:// URLs are allowed"))?;

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
        anyhow::bail!("IPv6 hosts are not supported in browser_open");
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
    let has_local_tld = host
        .rsplit('.')
        .next()
        .is_some_and(|label| label == "local");

    if host == "localhost" || host.ends_with(".localhost") || has_local_tld || host == "::1" {
        return true;
    }

    if let Some([a, b, _, _]) = parse_ipv4(host) {
        return a == 0
            || a == 10
            || a == 127
            || (a == 169 && b == 254)
            || (a == 172 && (16..=31).contains(&b))
            || (a == 192 && b == 168)
            || (a == 100 && (64..=127).contains(&b));
    }

    false
}

fn parse_ipv4(host: &str) -> Option<[u8; 4]> {
    let parts: Vec<&str> = host.split('.').collect();
    if parts.len() != 4 {
        return None;
    }

    let mut octets = [0_u8; 4];
    for (i, part) in parts.iter().enumerate() {
        octets[i] = part.parse::<u8>().ok()?;
    }
    Some(octets)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::alphahuman::security::{AutonomyLevel, SecurityPolicy};

    fn test_tool(allowed_domains: Vec<&str>) -> BrowserOpenTool {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::Supervised,
            ..SecurityPolicy::default()
        });
        BrowserOpenTool::new(
            security,
            allowed_domains.into_iter().map(String::from).collect(),
        )
    }

    #[test]
    fn normalize_domain_strips_scheme_path_and_case() {
        let got = normalize_domain("  HTTPS://Docs.Example.com/path ").unwrap();
        assert_eq!(got, "docs.example.com");
    }

    #[test]
    fn normalize_allowed_domains_deduplicates() {
        let got = normalize_allowed_domains(vec![
            "example.com".into(),
            "EXAMPLE.COM".into(),
            "https://example.com/".into(),
        ]);
        assert_eq!(got, vec!["example.com".to_string()]);
    }

    #[test]
    fn validate_accepts_exact_domain() {
        let tool = test_tool(vec!["example.com"]);
        let got = tool.validate_url("https://example.com/docs").unwrap();
        assert_eq!(got, "https://example.com/docs");
    }

    #[test]
    fn validate_accepts_subdomain() {
        let tool = test_tool(vec!["example.com"]);
        assert!(tool.validate_url("https://api.example.com/v1").is_ok());
    }

    #[test]
    fn validate_rejects_http() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("http://example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("https://"));
    }

    #[test]
    fn validate_rejects_localhost() {
        let tool = test_tool(vec!["localhost"]);
        let err = tool
            .validate_url("https://localhost:8080")
            .unwrap_err()
            .to_string();
        assert!(err.contains("local/private"));
    }

    #[test]
    fn validate_rejects_private_ipv4() {
        let tool = test_tool(vec!["192.168.1.5"]);
        let err = tool
            .validate_url("https://192.168.1.5")
            .unwrap_err()
            .to_string();
        assert!(err.contains("local/private"));
    }

    #[test]
    fn validate_rejects_allowlist_miss() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("https://google.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("allowed_domains"));
    }

    #[test]
    fn validate_rejects_whitespace() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("https://example.com/hello world")
            .unwrap_err()
            .to_string();
        assert!(err.contains("whitespace"));
    }

    #[test]
    fn validate_rejects_userinfo() {
        let tool = test_tool(vec!["example.com"]);
        let err = tool
            .validate_url("https://user@example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("userinfo"));
    }

    #[test]
    fn validate_requires_allowlist() {
        let security = Arc::new(SecurityPolicy::default());
        let tool = BrowserOpenTool::new(security, vec![]);
        let err = tool
            .validate_url("https://example.com")
            .unwrap_err()
            .to_string();
        assert!(err.contains("allowed_domains"));
    }

    #[test]
    fn parse_ipv4_valid() {
        assert_eq!(parse_ipv4("1.2.3.4"), Some([1, 2, 3, 4]));
    }

    #[test]
    fn parse_ipv4_invalid() {
        assert_eq!(parse_ipv4("1.2.3"), None);
        assert_eq!(parse_ipv4("1.2.3.999"), None);
        assert_eq!(parse_ipv4("not-an-ip"), None);
    }

    #[tokio::test]
    async fn execute_blocks_readonly_mode() {
        let security = Arc::new(SecurityPolicy {
            autonomy: AutonomyLevel::ReadOnly,
            ..SecurityPolicy::default()
        });
        let tool = BrowserOpenTool::new(security, vec!["example.com".into()]);
        let result = tool
            .execute(json!({"url": "https://example.com"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("read-only"));
    }

    #[tokio::test]
    async fn execute_blocks_when_rate_limited() {
        let security = Arc::new(SecurityPolicy {
            max_actions_per_hour: 0,
            ..SecurityPolicy::default()
        });
        let tool = BrowserOpenTool::new(security, vec!["example.com".into()]);
        let result = tool
            .execute(json!({"url": "https://example.com"}))
            .await
            .unwrap();
        assert!(!result.success);
        assert!(result.error.unwrap().contains("rate limit"));
    }
}
