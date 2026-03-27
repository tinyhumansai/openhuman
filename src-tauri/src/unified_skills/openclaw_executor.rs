//! Executor for openclaw-type skills (SKILL.md / SKILL.toml).
//!
//! openclaw skills are file-based:
//! - SKILL.toml → structured tool definitions (shell/http commands)
//! - SKILL.md   → markdown prompt content (returned as text)

use crate::runtime::types::{ToolContent, UnifiedSkillResult};
use chrono::Utc;
use openhuman_core::openhuman::skills::{Skill, SkillTool};
use std::collections::HashMap;
use std::net::IpAddr;

/// How substituted placeholder values should be escaped before insertion.
#[derive(Debug, Clone, Copy)]
enum EscapeContext {
    /// Values are shell-escaped (single-quote wrapping) for use in `sh -c` strings.
    Shell,
    /// Values are percent-encoded for use as URL components.
    Url,
    /// Values are inserted verbatim (no escaping).
    None,
}

/// Execute a named tool from an openclaw skill, or return prompt content if no tools.
pub async fn execute(
    skill: &Skill,
    skill_id: &str,
    tool_name: &str,
    args: serde_json::Value,
) -> Result<UnifiedSkillResult, String> {
    let executed_at = Utc::now().to_rfc3339();

    // SKILL.md skills have no tools — return the prompt content as text.
    if skill.tools.is_empty() {
        let content = skill.prompts.first().cloned().unwrap_or_default();
        return Ok(UnifiedSkillResult {
            skill_id: skill_id.to_string(),
            tool_name: None,
            content: vec![ToolContent::Text { text: content }],
            is_error: false,
            executed_at,
        });
    }

    // SKILL.toml: find the requested tool.
    let tool = skill
        .tools
        .iter()
        .find(|t| t.name == tool_name)
        .ok_or_else(|| format!("Tool '{tool_name}' not found in skill '{}'", skill.name))?;

    let result = run_tool(tool, args).await;

    match result {
        Ok(output) => Ok(UnifiedSkillResult {
            skill_id: skill_id.to_string(),
            tool_name: Some(tool_name.to_string()),
            content: vec![ToolContent::Text { text: output }],
            is_error: false,
            executed_at,
        }),
        Err(err) => Ok(UnifiedSkillResult {
            skill_id: skill_id.to_string(),
            tool_name: Some(tool_name.to_string()),
            content: vec![ToolContent::Text { text: err }],
            is_error: true,
            executed_at,
        }),
    }
}

/// Run a single SkillTool based on its kind.
async fn run_tool(tool: &SkillTool, args: serde_json::Value) -> Result<String, String> {
    match tool.kind.as_str() {
        "shell" => run_shell_tool(tool, args).await,
        "http" => run_http_tool(tool, args).await,
        other => Err(format!("Unsupported tool kind: '{other}'")),
    }
}

/// Execute a shell tool.
///
/// Placeholder values are shell-escaped before substitution to prevent injection.
/// Execution is bounded to 30 seconds.
async fn run_shell_tool(tool: &SkillTool, args: serde_json::Value) -> Result<String, String> {
    let command = interpolate_args(&tool.command, &tool.args, &args, EscapeContext::Shell);

    let child = tokio::process::Command::new("sh")
        .arg("-c")
        .arg(&command)
        .output();

    let output = tokio::time::timeout(std::time::Duration::from_secs(30), child)
        .await
        .map_err(|_| "Shell command timed out after 30 seconds".to_string())?
        .map_err(|e| format!("Failed to run shell command: {e}"))?;

    if output.status.success() {
        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        Err(if stderr.is_empty() {
            format!("Command exited with status {}", output.status)
        } else {
            stderr
        })
    }
}

/// Execute an HTTP tool (GET to the URL, or POST if args provided).
///
/// The resolved URL is validated: only http/https schemes are accepted, and
/// requests to private/internal IP ranges are rejected to prevent SSRF.
async fn run_http_tool(tool: &SkillTool, args: serde_json::Value) -> Result<String, String> {
    let url = interpolate_args(&tool.command, &tool.args, &args, EscapeContext::Url);

    validate_url_no_ssrf(&url).await?;

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .build()
        .map_err(|e| e.to_string())?;

    let args_obj = args.as_object();
    let response = if args_obj.map(|o| !o.is_empty()).unwrap_or(false) {
        client.post(&url).json(&args).send().await
    } else {
        client.get(&url).send().await
    };

    let resp = response.map_err(|e| format!("HTTP request failed: {e}"))?;
    let status = resp.status();
    let body = resp.text().await.map_err(|e| e.to_string())?;

    if status.is_success() {
        Ok(body)
    } else {
        Err(format!("HTTP {status}: {body}"))
    }
}

/// Validate that `raw_url` is a safe public HTTP/HTTPS URL.
///
/// Rejects:
/// - Non-http/https schemes
/// - URLs with no host
/// - Hosts that resolve to private, loopback, or link-local IP addresses (SSRF guard)
async fn validate_url_no_ssrf(raw_url: &str) -> Result<(), String> {
    let parsed = url::Url::parse(raw_url).map_err(|e| format!("Invalid URL: {e}"))?;

    match parsed.scheme() {
        "http" | "https" => {}
        scheme => {
            return Err(format!(
                "URL scheme '{scheme}' is not permitted; only http and https are allowed"
            ))
        }
    }

    let host = parsed
        .host_str()
        .ok_or_else(|| "URL has no host".to_string())?;

    let port = parsed.port_or_known_default().unwrap_or(80);
    let addrs = tokio::net::lookup_host(format!("{host}:{port}"))
        .await
        .map_err(|e| format!("Failed to resolve host '{host}': {e}"))?;

    for addr in addrs {
        let ip = addr.ip();
        if is_private_ip(ip) {
            return Err(format!(
                "SSRF protection: request to '{host}' is not allowed \
                 (resolves to internal address {ip})"
            ));
        }
    }

    Ok(())
}

/// Returns `true` for any IP address that belongs to a private or internal range.
fn is_private_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            v4.is_loopback()      // 127.0.0.0/8
            || v4.is_private()    // 10/8, 172.16/12, 192.168/16
            || v4.is_link_local() // 169.254.0.0/16
            || v4.is_broadcast()  // 255.255.255.255
            || v4.is_unspecified() // 0.0.0.0
        }
        IpAddr::V6(v6) => {
            v6.is_loopback()      // ::1
            || v6.is_unspecified() // ::
            // fc00::/7 — unique local (includes fd00::/8)
            || (v6.segments()[0] & 0xfe00) == 0xfc00
            // fe80::/10 — link-local
            || (v6.segments()[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Replace `${key}` patterns in `template` using tool default args merged with caller args.
/// Each substituted value is escaped according to `ctx` before insertion.
fn interpolate_args(
    template: &str,
    tool_defaults: &HashMap<String, String>,
    caller_args: &serde_json::Value,
    ctx: EscapeContext,
) -> String {
    let escape = |val: &str| -> String {
        match ctx {
            EscapeContext::Shell => shell_escape(val),
            EscapeContext::Url => urlencoding::encode(val).into_owned(),
            EscapeContext::None => val.to_string(),
        }
    };

    let mut result = template.to_string();

    // Apply tool-level defaults first.
    for (key, value) in tool_defaults {
        result = result.replace(&format!("${{{key}}}"), &escape(value));
    }

    // Caller args override defaults.
    if let Some(obj) = caller_args.as_object() {
        for (key, value) in obj {
            let str_val = match value {
                serde_json::Value::String(s) => s.clone(),
                other => other.to_string(),
            };
            result = result.replace(&format!("${{{key}}}"), &escape(&str_val));
        }
    }

    result
}

/// POSIX single-quote shell escaping: wraps `s` in single quotes and escapes
/// any single quotes inside as `'\''`, making the value safe for `sh -c` strings.
fn shell_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('\'');
    for c in s.chars() {
        if c == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(c);
        }
    }
    out.push('\'');
    out
}
