//! Webhook event notifications for chat_with_data insights.
//!
//! Fires HTTP POST to registered webhook URLs when anomalies or insights are detected.

use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::Mutex;
use tracing::{debug, info, warn};

use super::types::Insight;

const LOG_PREFIX: &str = "[cwd-webhooks]";
const MAX_HOOKS: usize = 20;

static HOOKS: std::sync::LazyLock<Mutex<HashMap<String, WebhookConfig>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

#[derive(Debug, Clone)]
pub struct WebhookConfig {
    pub id: String,
    pub url: String,
    pub events: Vec<WebhookEvent>,
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub enum WebhookEvent {
    AnomalyDetected,
    InsightGenerated,
    ThresholdBreached,
}

/// Returns true if `ip` is in a range we must never POST to (SSRF guard):
/// loopback, private (RFC1918 / ULA), link-local, unspecified, CGNAT, or
/// IPv4-mapped IPv6 forms of any of those.
fn is_blocked_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            let o = v4.octets();
            v4.is_loopback()
                || v4.is_private()
                || v4.is_link_local()
                || v4.is_unspecified()
                || o[0] == 0 // 0.0.0.0/8
                || (o[0] == 100 && (o[1] & 0xc0) == 64) // 100.64.0.0/10 CGNAT
        }
        IpAddr::V6(v6) => {
            // Treat IPv4-mapped addresses (::ffff:a.b.c.d) as their IPv4 form.
            if let Some(v4) = v6.to_ipv4_mapped() {
                return is_blocked_ip(IpAddr::V4(v4));
            }
            let seg = v6.segments();
            v6.is_loopback()
                || v6.is_unspecified()
                || (seg[0] & 0xffc0) == 0xfe80 // link-local fe80::/10
                || (seg[0] & 0xfe00) == 0xfc00 // unique-local fc00::/7
        }
    }
}

/// Strip the surrounding brackets from a bracketed IPv6 host literal (`[::1]` -> `::1`).
///
/// `reqwest::Url::host_str` returns IPv6 hosts with brackets, but both
/// `IpAddr::parse` and `tokio::net::lookup_host` expect the bare address.
/// Non-bracketed hosts (domains, IPv4) are returned unchanged.
fn unbracket_host(host: &str) -> &str {
    host.strip_prefix('[')
        .and_then(|h| h.strip_suffix(']'))
        .unwrap_or(host)
}

/// Validate that a webhook URL does not target private/loopback addresses (SSRF protection).
///
/// This screens the literal host. Hostnames that resolve to private addresses are
/// re-checked against [`is_blocked_ip`] at send time in [`notify_insight`] to defend
/// against DNS rebinding / private DNS records.
fn validate_webhook_url(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    match parsed.scheme() {
        "http" | "https" => {}
        s => return Err(format!("webhook url must be http/https, got: {s}")),
    }
    let host = parsed.host_str().unwrap_or("");
    if host.is_empty() {
        return Err("URL has no host".into());
    }
    if host.eq_ignore_ascii_case("localhost") {
        return Err("loopback addresses are not allowed".into());
    }
    // `host_str()` returns IPv6 literals wrapped in brackets (e.g. `[::1]`),
    // which do not parse as `IpAddr` — strip them before checking.
    if let Ok(ip) = unbracket_host(host).parse::<IpAddr>() {
        if is_blocked_ip(ip) {
            return Err("private/loopback/link-local addresses are not allowed".into());
        }
    }
    Ok(())
}

/// Resolve a URL's host and reject if any resolved address is blocked.
///
/// Run immediately before sending so a hostname that resolves to an internal
/// address (DNS rebinding, changed records) cannot reach private services.
async fn resolve_and_check(url: &str) -> Result<(), String> {
    let parsed = reqwest::Url::parse(url).map_err(|e| format!("invalid URL: {e}"))?;
    let host = unbracket_host(
        parsed
            .host_str()
            .ok_or_else(|| "URL has no host".to_string())?,
    )
    .to_string();
    // Literal IPs are already covered by validate_webhook_url, but lookup_host
    // handles them uniformly and we re-check regardless.
    let port = parsed.port_or_known_default().unwrap_or(443);
    let mut resolved = false;
    for addr in tokio::net::lookup_host((host.as_str(), port))
        .await
        .map_err(|e| format!("dns resolution failed: {e}"))?
    {
        resolved = true;
        if is_blocked_ip(addr.ip()) {
            return Err("resolved address is private/loopback/link-local".into());
        }
    }
    if !resolved {
        return Err("host did not resolve".into());
    }
    Ok(())
}

/// Register a webhook endpoint.
pub fn register_webhook(url: &str, events: Vec<WebhookEvent>) -> Result<String, String> {
    validate_webhook_url(url)?;
    let mut store = HOOKS.lock().map_err(|e| format!("lock: {e}"))?;
    if store.len() >= MAX_HOOKS {
        return Err("max webhooks reached".into());
    }
    let id = format!("wh-{}", crate::openhuman::util::uuid_v4());
    let host = reqwest::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_string))
        .unwrap_or_else(|| "redacted".into());
    store.insert(
        id.clone(),
        WebhookConfig {
            id: id.clone(),
            url: url.into(),
            events,
            active: true,
        },
    );
    info!("{LOG_PREFIX} registered webhook {id} -> host={host}");
    Ok(id)
}

/// Remove a webhook.
pub fn unregister_webhook(id: &str) -> Result<(), String> {
    HOOKS
        .lock()
        .map_err(|e| format!("lock: {e}"))?
        .remove(id)
        .map(|_| ())
        .ok_or("webhook not found".into())
}

/// List registered webhooks.
pub fn list_webhooks() -> Vec<WebhookConfig> {
    HOOKS
        .lock()
        .map(|s| s.values().cloned().collect())
        .unwrap_or_default()
}

/// Fire webhook notifications for an insight event.
/// Spawns async HTTP POST tasks — does not block.
pub fn notify_insight(insight: &Insight, event: WebhookEvent) {
    let hooks: Vec<WebhookConfig> = HOOKS
        .lock()
        .map(|s| {
            s.values()
                .filter(|h| h.active && h.events.contains(&event))
                .cloned()
                .collect()
        })
        .unwrap_or_default();

    if hooks.is_empty() {
        return;
    }

    let payload = serde_json::json!({
        "event": format!("{:?}", event),
        "insight_id": insight.id,
        "title": insight.title,
        "dataset": insight.dataset,
        "severity": format!("{:?}", insight.severity),
        "description": insight.description,
        "timestamp": crate::openhuman::util::now_epoch(),
    });

    for hook in hooks {
        let payload = payload.clone();
        let url = hook.url.clone();
        let host = reqwest::Url::parse(&url)
            .ok()
            .and_then(|u| u.host_str().map(str::to_string))
            .unwrap_or_else(|| "redacted".into());
        tokio::spawn(async move {
            // Re-resolve and re-validate at send time (DNS rebinding guard).
            if let Err(e) = resolve_and_check(&url).await {
                warn!("{LOG_PREFIX} host={host} blocked: {e}");
                return;
            }
            debug!("{LOG_PREFIX} firing to host={host}");
            match reqwest::Client::new()
                .post(&url)
                .json(&payload)
                .timeout(std::time::Duration::from_secs(10))
                .send()
                .await
            {
                Ok(resp) => debug!("{LOG_PREFIX} host={host} responded {}", resp.status()),
                Err(e) => warn!("{LOG_PREFIX} host={host} failed: {e}"),
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_and_list() {
        let id = register_webhook(
            "http://example.com:9999/hook",
            vec![WebhookEvent::AnomalyDetected],
        )
        .unwrap();
        assert!(id.starts_with("wh-"));
        let hooks = list_webhooks();
        assert!(hooks.iter().any(|h| h.id == id));
        unregister_webhook(&id).unwrap();
    }

    #[test]
    fn rejects_localhost() {
        let r = register_webhook(
            "http://localhost:9999/hook",
            vec![WebhookEvent::AnomalyDetected],
        );
        assert!(r.is_err());
    }

    #[test]
    fn rejects_private_and_special_literal_ips() {
        for url in [
            "http://127.0.0.1/hook",       // loopback
            "http://10.0.0.1/hook",        // RFC1918
            "http://172.16.0.1/hook",      // RFC1918
            "http://192.168.1.1/hook",     // RFC1918
            "http://169.254.169.254/hook", // link-local / cloud metadata
            "http://0.0.0.0/hook",         // unspecified
            "http://100.64.0.1/hook",      // CGNAT
        ] {
            assert!(
                register_webhook(url, vec![WebhookEvent::AnomalyDetected]).is_err(),
                "expected {url} to be rejected"
            );
        }
    }

    #[test]
    fn rejects_private_ipv6_literals() {
        for url in [
            "http://[::1]/hook",              // loopback
            "http://[fe80::1]/hook",          // link-local
            "http://[fc00::1]/hook",          // unique-local
            "http://[fd00::1]/hook",          // unique-local
            "http://[::ffff:127.0.0.1]/hook", // IPv4-mapped loopback
        ] {
            assert!(
                register_webhook(url, vec![WebhookEvent::AnomalyDetected]).is_err(),
                "expected {url} to be rejected"
            );
        }
    }

    #[test]
    fn allows_public_literal_ip() {
        let id = register_webhook("http://8.8.8.8/hook", vec![WebhookEvent::InsightGenerated])
            .expect("public ip should be allowed");
        unregister_webhook(&id).unwrap();
    }
}
