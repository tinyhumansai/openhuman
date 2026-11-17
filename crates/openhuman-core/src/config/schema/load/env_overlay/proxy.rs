//! Env overrides for the outbound proxy configuration.

use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::proxy::{
    normalize_no_proxy_list, normalize_proxy_url_option, normalize_service_list,
    parse_proxy_enabled, parse_proxy_scope,
};
use crate::config::schema::Config;

impl Config {
    pub(super) fn apply_proxy_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        let explicit_proxy_enabled = env
            .get("OPENHUMAN_PROXY_ENABLED")
            .as_deref()
            .and_then(parse_proxy_enabled);
        if let Some(enabled) = explicit_proxy_enabled {
            self.proxy.enabled = enabled;
            self.record_env_override(env, "proxy.enabled", &["OPENHUMAN_PROXY_ENABLED"]);
        }

        let mut proxy_url_overridden = false;
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_HTTP_PROXY", "HTTP_PROXY"]) {
            self.proxy.http_proxy = normalize_proxy_url_option(Some(&proxy_url));
            self.record_env_override(
                env,
                "proxy.http_proxy",
                &["OPENHUMAN_HTTP_PROXY", "HTTP_PROXY"],
            );
            proxy_url_overridden = true;
        }
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_HTTPS_PROXY", "HTTPS_PROXY"]) {
            self.proxy.https_proxy = normalize_proxy_url_option(Some(&proxy_url));
            self.record_env_override(
                env,
                "proxy.https_proxy",
                &["OPENHUMAN_HTTPS_PROXY", "HTTPS_PROXY"],
            );
            proxy_url_overridden = true;
        }
        if let Some(proxy_url) = env.get_any(&["OPENHUMAN_ALL_PROXY", "ALL_PROXY"]) {
            self.proxy.all_proxy = normalize_proxy_url_option(Some(&proxy_url));
            self.record_env_override(
                env,
                "proxy.all_proxy",
                &["OPENHUMAN_ALL_PROXY", "ALL_PROXY"],
            );
            proxy_url_overridden = true;
        }
        if let Some(no_proxy) = env.get_any(&["OPENHUMAN_NO_PROXY", "NO_PROXY"]) {
            self.proxy.no_proxy = normalize_no_proxy_list(vec![no_proxy]);
            self.record_env_override(env, "proxy.no_proxy", &["OPENHUMAN_NO_PROXY", "NO_PROXY"]);
        }

        if explicit_proxy_enabled.is_none()
            && proxy_url_overridden
            && self.proxy.has_any_proxy_url()
        {
            self.proxy.enabled = true;
            self.record_env_override(
                env,
                "proxy.enabled",
                &[
                    "OPENHUMAN_HTTP_PROXY",
                    "HTTP_PROXY",
                    "OPENHUMAN_HTTPS_PROXY",
                    "HTTPS_PROXY",
                    "OPENHUMAN_ALL_PROXY",
                    "ALL_PROXY",
                ],
            );
        }

        if let Some(scope_raw) = env.get("OPENHUMAN_PROXY_SCOPE") {
            let trimmed = scope_raw.trim();
            if !trimmed.is_empty() {
                match parse_proxy_scope(trimmed) {
                    Some(scope) => {
                        self.proxy.scope = scope;
                        self.record_env_override(env, "proxy.scope", &["OPENHUMAN_PROXY_SCOPE"]);
                    }
                    None => {
                        tracing::warn!("Invalid OPENHUMAN_PROXY_SCOPE value {:?} ignored", trimmed);
                    }
                }
            }
        }

        if let Some(services_raw) = env.get("OPENHUMAN_PROXY_SERVICES") {
            self.proxy.services = normalize_service_list(vec![services_raw]);
            self.record_env_override(env, "proxy.services", &["OPENHUMAN_PROXY_SERVICES"]);
        }

        if let Err(error) = self.proxy.validate() {
            tracing::warn!("Invalid proxy configuration ignored: {error}");
            self.proxy.enabled = false;
        }
    }
}
