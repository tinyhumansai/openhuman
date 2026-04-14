//! Proxy configuration and runtime proxy client building.

use anyhow::{Context, Result};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

const SUPPORTED_PROXY_SERVICE_KEYS: &[&str] = &[
    "provider.anthropic",
    "provider.compatible",
    "provider.copilot",
    "provider.gemini",
    "provider.glm",
    "provider.ollama",
    "provider.openai",
    "provider.openrouter",
    "channel.dingtalk",
    "channel.discord",
    "channel.lark",
    "channel.matrix",
    "channel.mattermost",
    "channel.qq",
    "channel.signal",
    "channel.slack",
    "channel.telegram",
    "channel.whatsapp",
    "tool.browser",
    "tool.composio",
    "tool.http_request",
    "tool.pushover",
    "memory.embeddings",
    "tunnel.custom",
];

const SUPPORTED_PROXY_SERVICE_SELECTORS: &[&str] =
    &["provider.*", "channel.*", "tool.*", "memory.*", "tunnel.*"];

static RUNTIME_PROXY_CONFIG: OnceLock<RwLock<ProxyConfig>> = OnceLock::new();
static RUNTIME_PROXY_CLIENT_CACHE: OnceLock<RwLock<HashMap<String, reqwest::Client>>> =
    OnceLock::new();

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Default, PartialEq, Eq, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum ProxyScope {
    Environment,
    #[default]
    OpenHuman,
    Services,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct ProxyConfig {
    #[serde(default)]
    pub enabled: bool,
    #[serde(default)]
    pub http_proxy: Option<String>,
    #[serde(default)]
    pub https_proxy: Option<String>,
    #[serde(default)]
    pub all_proxy: Option<String>,
    #[serde(default)]
    pub no_proxy: Vec<String>,
    #[serde(default)]
    pub scope: ProxyScope,
    #[serde(default)]
    pub services: Vec<String>,
}

impl Default for ProxyConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            http_proxy: None,
            https_proxy: None,
            all_proxy: None,
            no_proxy: Vec::new(),
            scope: ProxyScope::OpenHuman,
            services: Vec::new(),
        }
    }
}

impl ProxyConfig {
    pub fn supported_service_keys() -> &'static [&'static str] {
        SUPPORTED_PROXY_SERVICE_KEYS
    }

    pub fn supported_service_selectors() -> &'static [&'static str] {
        SUPPORTED_PROXY_SERVICE_SELECTORS
    }

    pub fn has_any_proxy_url(&self) -> bool {
        normalize_proxy_url_option(self.http_proxy.as_deref()).is_some()
            || normalize_proxy_url_option(self.https_proxy.as_deref()).is_some()
            || normalize_proxy_url_option(self.all_proxy.as_deref()).is_some()
    }

    pub fn normalized_services(&self) -> Vec<String> {
        normalize_service_list(self.services.clone())
    }

    pub fn normalized_no_proxy(&self) -> Vec<String> {
        normalize_no_proxy_list(self.no_proxy.clone())
    }

    pub fn validate(&self) -> Result<()> {
        for (field, value) in [
            ("http_proxy", self.http_proxy.as_deref()),
            ("https_proxy", self.https_proxy.as_deref()),
            ("all_proxy", self.all_proxy.as_deref()),
        ] {
            if let Some(url) = normalize_proxy_url_option(value) {
                validate_proxy_url(field, &url)?;
            }
        }

        for selector in self.normalized_services() {
            if !is_supported_proxy_service_selector(&selector) {
                anyhow::bail!(
                    "Unsupported proxy service selector '{selector}'. Use tool `proxy_config` action `list_services` for valid values"
                );
            }
        }

        if self.enabled && !self.has_any_proxy_url() {
            anyhow::bail!(
                "Proxy is enabled but no proxy URL is configured. Set at least one of http_proxy, https_proxy, or all_proxy"
            );
        }

        if self.enabled
            && self.scope == ProxyScope::Services
            && self.normalized_services().is_empty()
        {
            anyhow::bail!(
                "proxy.scope='services' requires a non-empty proxy.services list when proxy is enabled"
            );
        }

        Ok(())
    }

    pub fn should_apply_to_service(&self, service_key: &str) -> bool {
        if !self.enabled {
            return false;
        }

        match self.scope {
            ProxyScope::Environment => false,
            ProxyScope::OpenHuman => true,
            ProxyScope::Services => {
                let service_key = service_key.trim().to_ascii_lowercase();
                if service_key.is_empty() {
                    return false;
                }

                self.normalized_services()
                    .iter()
                    .any(|selector| service_selector_matches(selector, &service_key))
            }
        }
    }

    pub fn apply_to_reqwest_builder(
        &self,
        mut builder: reqwest::ClientBuilder,
        service_key: &str,
    ) -> reqwest::ClientBuilder {
        if !self.should_apply_to_service(service_key) {
            return builder;
        }

        let no_proxy = self.no_proxy_value();

        if let Some(url) = normalize_proxy_url_option(self.all_proxy.as_deref()) {
            match reqwest::Proxy::all(&url) {
                Ok(proxy) => {
                    builder = builder.proxy(apply_no_proxy(proxy, no_proxy.clone()));
                }
                Err(error) => {
                    tracing::warn!(
                        proxy_url = %url,
                        service_key,
                        "Ignoring invalid all_proxy URL: {error}"
                    );
                }
            }
        }

        if let Some(url) = normalize_proxy_url_option(self.http_proxy.as_deref()) {
            match reqwest::Proxy::http(&url) {
                Ok(proxy) => {
                    builder = builder.proxy(apply_no_proxy(proxy, no_proxy.clone()));
                }
                Err(error) => {
                    tracing::warn!(
                        proxy_url = %url,
                        service_key,
                        "Ignoring invalid http_proxy URL: {error}"
                    );
                }
            }
        }

        if let Some(url) = normalize_proxy_url_option(self.https_proxy.as_deref()) {
            match reqwest::Proxy::https(&url) {
                Ok(proxy) => {
                    builder = builder.proxy(apply_no_proxy(proxy, no_proxy));
                }
                Err(error) => {
                    tracing::warn!(
                        proxy_url = %url,
                        service_key,
                        "Ignoring invalid https_proxy URL: {error}"
                    );
                }
            }
        }

        builder
    }

    pub fn apply_to_process_env(&self) {
        set_proxy_env_pair("HTTP_PROXY", self.http_proxy.as_deref());
        set_proxy_env_pair("HTTPS_PROXY", self.https_proxy.as_deref());
        set_proxy_env_pair("ALL_PROXY", self.all_proxy.as_deref());

        let no_proxy_joined = {
            let list = self.normalized_no_proxy();
            (!list.is_empty()).then(|| list.join(","))
        };
        set_proxy_env_pair("NO_PROXY", no_proxy_joined.as_deref());
    }

    pub fn clear_process_env() {
        clear_proxy_env_pair("HTTP_PROXY");
        clear_proxy_env_pair("HTTPS_PROXY");
        clear_proxy_env_pair("ALL_PROXY");
        clear_proxy_env_pair("NO_PROXY");
    }

    fn no_proxy_value(&self) -> Option<reqwest::NoProxy> {
        let joined = {
            let list = self.normalized_no_proxy();
            (!list.is_empty()).then(|| list.join(","))
        };
        joined.as_deref().and_then(reqwest::NoProxy::from_string)
    }
}

fn apply_no_proxy(proxy: reqwest::Proxy, no_proxy: Option<reqwest::NoProxy>) -> reqwest::Proxy {
    proxy.no_proxy(no_proxy)
}

pub(crate) fn normalize_proxy_url_option(raw: Option<&str>) -> Option<String> {
    let value = raw?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

pub(crate) fn normalize_no_proxy_list(values: Vec<String>) -> Vec<String> {
    normalize_comma_values(values)
}

pub(crate) fn normalize_service_list(values: Vec<String>) -> Vec<String> {
    let mut normalized = normalize_comma_values(values)
        .into_iter()
        .map(|value| value.to_ascii_lowercase())
        .collect::<Vec<_>>();
    normalized.sort_unstable();
    normalized.dedup();
    normalized
}

fn normalize_comma_values(values: Vec<String>) -> Vec<String> {
    let mut output = Vec::new();
    for value in values {
        for part in value.split(',') {
            let normalized = part.trim();
            if normalized.is_empty() {
                continue;
            }
            output.push(normalized.to_string());
        }
    }
    output.sort_unstable();
    output.dedup();
    output
}

fn is_supported_proxy_service_selector(selector: &str) -> bool {
    if SUPPORTED_PROXY_SERVICE_KEYS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(selector))
    {
        return true;
    }

    SUPPORTED_PROXY_SERVICE_SELECTORS
        .iter()
        .any(|known| known.eq_ignore_ascii_case(selector))
}

fn service_selector_matches(selector: &str, service_key: &str) -> bool {
    if selector == service_key {
        return true;
    }

    if let Some(prefix) = selector.strip_suffix(".*") {
        return service_key.starts_with(prefix)
            && service_key
                .strip_prefix(prefix)
                .is_some_and(|suffix| suffix.starts_with('.'));
    }

    false
}

fn validate_proxy_url(field: &str, url: &str) -> Result<()> {
    let parsed = reqwest::Url::parse(url)
        .with_context(|| format!("Invalid {field} URL: '{url}' is not a valid URL"))?;

    match parsed.scheme() {
        "http" | "https" | "socks5" | "socks5h" => {}
        scheme => {
            anyhow::bail!(
                "Invalid {field} URL scheme '{scheme}'. Allowed: http, https, socks5, socks5h"
            );
        }
    }

    if parsed.host_str().is_none() {
        anyhow::bail!("Invalid {field} URL: host is required");
    }

    Ok(())
}

fn set_proxy_env_pair(key: &str, value: Option<&str>) {
    let lowercase_key = key.to_ascii_lowercase();
    if let Some(value) = value.and_then(|candidate| normalize_proxy_url_option(Some(candidate))) {
        std::env::set_var(key, &value);
        std::env::set_var(lowercase_key, value);
    } else {
        std::env::remove_var(key);
        std::env::remove_var(lowercase_key);
    }
}

fn clear_proxy_env_pair(key: &str) {
    std::env::remove_var(key);
    std::env::remove_var(key.to_ascii_lowercase());
}

fn runtime_proxy_state() -> &'static RwLock<ProxyConfig> {
    RUNTIME_PROXY_CONFIG.get_or_init(|| RwLock::new(ProxyConfig::default()))
}

fn runtime_proxy_client_cache() -> &'static RwLock<HashMap<String, reqwest::Client>> {
    RUNTIME_PROXY_CLIENT_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn clear_runtime_proxy_client_cache() {
    match runtime_proxy_client_cache().write() {
        Ok(mut guard) => {
            guard.clear();
        }
        Err(poisoned) => {
            poisoned.into_inner().clear();
        }
    }
}

fn runtime_proxy_cache_key(
    service_key: &str,
    timeout_secs: Option<u64>,
    connect_timeout_secs: Option<u64>,
) -> String {
    format!(
        "{}|timeout={}|connect_timeout={}",
        service_key.trim().to_ascii_lowercase(),
        timeout_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string()),
        connect_timeout_secs
            .map(|value| value.to_string())
            .unwrap_or_else(|| "none".to_string())
    )
}

fn runtime_proxy_cached_client(cache_key: &str) -> Option<reqwest::Client> {
    match runtime_proxy_client_cache().read() {
        Ok(guard) => guard.get(cache_key).cloned(),
        Err(poisoned) => poisoned.into_inner().get(cache_key).cloned(),
    }
}

fn set_runtime_proxy_cached_client(cache_key: String, client: reqwest::Client) {
    match runtime_proxy_client_cache().write() {
        Ok(mut guard) => {
            guard.insert(cache_key, client);
        }
        Err(poisoned) => {
            poisoned.into_inner().insert(cache_key, client);
        }
    }
}

pub fn set_runtime_proxy_config(config: ProxyConfig) {
    match runtime_proxy_state().write() {
        Ok(mut guard) => {
            *guard = config;
        }
        Err(poisoned) => {
            *poisoned.into_inner() = config;
        }
    }

    clear_runtime_proxy_client_cache();
}

pub fn runtime_proxy_config() -> ProxyConfig {
    match runtime_proxy_state().read() {
        Ok(guard) => guard.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

pub fn apply_runtime_proxy_to_builder(
    builder: reqwest::ClientBuilder,
    service_key: &str,
) -> reqwest::ClientBuilder {
    runtime_proxy_config().apply_to_reqwest_builder(builder, service_key)
}

pub fn build_runtime_proxy_client(service_key: &str) -> reqwest::Client {
    let cache_key = runtime_proxy_cache_key(service_key, None, None);
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let builder = apply_runtime_proxy_to_builder(reqwest::Client::builder(), service_key);
    let client = builder.build().unwrap_or_else(|error| {
        tracing::warn!(service_key, "Failed to build proxied client: {error}");
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

pub fn build_runtime_proxy_client_with_timeouts(
    service_key: &str,
    timeout_secs: u64,
    connect_timeout_secs: u64,
) -> reqwest::Client {
    let cache_key =
        runtime_proxy_cache_key(service_key, Some(timeout_secs), Some(connect_timeout_secs));
    if let Some(client) = runtime_proxy_cached_client(&cache_key) {
        return client;
    }

    let builder = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .connect_timeout(std::time::Duration::from_secs(connect_timeout_secs));
    let builder = apply_runtime_proxy_to_builder(builder, service_key);
    let client = builder.build().unwrap_or_else(|error| {
        tracing::warn!(
            service_key,
            "Failed to build proxied timeout client: {error}"
        );
        reqwest::Client::new()
    });
    set_runtime_proxy_cached_client(cache_key, client.clone());
    client
}

pub(crate) fn parse_proxy_scope(raw: &str) -> Option<ProxyScope> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "environment" | "env" => Some(ProxyScope::Environment),
        "openhuman" | "internal" | "core" => Some(ProxyScope::OpenHuman),
        "services" | "service" => Some(ProxyScope::Services),
        _ => None,
    }
}

pub(crate) fn parse_proxy_enabled(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── normalize_proxy_url_option ─────────────────────────────────

    #[test]
    fn normalize_proxy_url_option_handles_none_empty_and_valid() {
        assert_eq!(normalize_proxy_url_option(None), None);
        assert_eq!(normalize_proxy_url_option(Some("")), None);
        assert_eq!(normalize_proxy_url_option(Some("   ")), None);
        assert_eq!(
            normalize_proxy_url_option(Some("  http://proxy:8080  ")),
            Some("http://proxy:8080".to_string())
        );
    }

    // ── normalize_comma_values / normalize_service_list / normalize_no_proxy_list ─

    #[test]
    fn normalize_comma_values_splits_trims_and_dedups() {
        let out = normalize_comma_values(vec!["a,b".into(), " c,a  ".into(), "".into()]);
        assert_eq!(out, vec!["a", "b", "c"]);
    }

    #[test]
    fn normalize_comma_values_empty_input_returns_empty() {
        assert!(normalize_comma_values(vec![]).is_empty());
        assert!(normalize_comma_values(vec!["".into(), " ".into()]).is_empty());
    }

    #[test]
    fn normalize_service_list_lowercases_and_dedups() {
        let out =
            normalize_service_list(vec!["OPENAI".into(), "openai".into(), "Anthropic".into()]);
        assert_eq!(out, vec!["anthropic", "openai"]);
    }

    #[test]
    fn normalize_no_proxy_list_preserves_case() {
        let out = normalize_no_proxy_list(vec!["localhost,127.0.0.1".into()]);
        assert_eq!(out, vec!["127.0.0.1", "localhost"]);
    }

    // ── parse_proxy_scope ──────────────────────────────────────────

    #[test]
    fn parse_proxy_scope_accepts_known_aliases() {
        assert_eq!(
            parse_proxy_scope("environment"),
            Some(ProxyScope::Environment)
        );
        assert_eq!(parse_proxy_scope("env"), Some(ProxyScope::Environment));
        assert_eq!(parse_proxy_scope("ENV"), Some(ProxyScope::Environment));
        assert_eq!(parse_proxy_scope("openhuman"), Some(ProxyScope::OpenHuman));
        assert_eq!(parse_proxy_scope("internal"), Some(ProxyScope::OpenHuman));
        assert_eq!(parse_proxy_scope("core"), Some(ProxyScope::OpenHuman));
        assert_eq!(parse_proxy_scope("services"), Some(ProxyScope::Services));
        assert_eq!(parse_proxy_scope("service"), Some(ProxyScope::Services));
        assert_eq!(
            parse_proxy_scope("  SERVICES  "),
            Some(ProxyScope::Services)
        );
    }

    #[test]
    fn parse_proxy_scope_rejects_unknown() {
        assert!(parse_proxy_scope("").is_none());
        assert!(parse_proxy_scope("other").is_none());
    }

    // ── parse_proxy_enabled ────────────────────────────────────────

    #[test]
    fn parse_proxy_enabled_accepts_truthy_and_falsy() {
        for t in ["1", "true", "yes", "on", "TRUE", " YES "] {
            assert_eq!(
                parse_proxy_enabled(t),
                Some(true),
                "`{t}` should parse truthy"
            );
        }
        for f in ["0", "false", "no", "off", "FALSE"] {
            assert_eq!(
                parse_proxy_enabled(f),
                Some(false),
                "`{f}` should parse falsy"
            );
        }
        assert_eq!(parse_proxy_enabled(""), None);
        assert_eq!(parse_proxy_enabled("nope"), None);
    }

    // ── ProxyConfig::default / has_any_proxy_url ──────────────────

    #[test]
    fn proxy_config_default_has_no_urls() {
        let c = ProxyConfig::default();
        assert!(!c.has_any_proxy_url());
    }

    #[test]
    fn proxy_config_has_any_proxy_url_detects_each_url_field() {
        let mut c = ProxyConfig::default();
        c.http_proxy = Some("http://h:8080".into());
        assert!(c.has_any_proxy_url());
        let mut c = ProxyConfig::default();
        c.https_proxy = Some("https://h:8443".into());
        assert!(c.has_any_proxy_url());
        let mut c = ProxyConfig::default();
        c.all_proxy = Some("socks5://h:1080".into());
        assert!(c.has_any_proxy_url());
    }

    #[test]
    fn proxy_config_has_any_proxy_url_ignores_whitespace_urls() {
        let mut c = ProxyConfig::default();
        c.http_proxy = Some("   ".into());
        c.https_proxy = Some("".into());
        assert!(!c.has_any_proxy_url());
    }

    // ── is_supported_proxy_service_selector ────────────────────────

    #[test]
    fn is_supported_proxy_service_selector_accepts_known_keys_case_insensitive() {
        for key in SUPPORTED_PROXY_SERVICE_KEYS {
            assert!(is_supported_proxy_service_selector(key));
            assert!(is_supported_proxy_service_selector(
                &key.to_ascii_uppercase()
            ));
        }
        for sel in SUPPORTED_PROXY_SERVICE_SELECTORS {
            assert!(is_supported_proxy_service_selector(sel));
        }
        assert!(!is_supported_proxy_service_selector("not-a-selector-xyz"));
    }

    // ── service_selector_matches ───────────────────────────────────

    #[test]
    fn service_selector_matches_exact_and_wildcard() {
        assert!(service_selector_matches("openai", "openai"));
        assert!(!service_selector_matches("openai", "anthropic"));
        // Wildcard prefix: `foo.*` matches `foo.bar` but not `foo` or `foobar`.
        assert!(service_selector_matches("foo.*", "foo.bar"));
        assert!(service_selector_matches("foo.*", "foo.bar.baz"));
        assert!(!service_selector_matches("foo.*", "foo"));
        assert!(!service_selector_matches("foo.*", "foobar"));
    }

    // ── validate_proxy_url ─────────────────────────────────────────

    #[test]
    fn validate_proxy_url_accepts_supported_schemes_with_host() {
        assert!(validate_proxy_url("http_proxy", "http://proxy:8080").is_ok());
        assert!(validate_proxy_url("https_proxy", "https://proxy:8443").is_ok());
        assert!(validate_proxy_url("all_proxy", "socks5://proxy:1080").is_ok());
        assert!(validate_proxy_url("all_proxy", "socks5h://proxy:1080").is_ok());
    }

    #[test]
    fn validate_proxy_url_rejects_unsupported_schemes() {
        let err = validate_proxy_url("x", "ftp://proxy:21").unwrap_err();
        assert!(err.to_string().contains("Invalid"));
    }

    #[test]
    fn validate_proxy_url_rejects_missing_host() {
        // e.g. scheme-only URL parses but has no host
        let err = validate_proxy_url("x", "http://").unwrap_err();
        assert!(err.to_string().to_lowercase().contains("invalid"));
    }

    #[test]
    fn validate_proxy_url_rejects_malformed_url() {
        let err = validate_proxy_url("x", "not a url").unwrap_err();
        assert!(err.to_string().to_lowercase().contains("invalid"));
    }
}
