//! Env overrides for Sentry, analytics, and agent-tracing consent.

use crate::config::schema::load::env::EnvLookup;
use crate::config::schema::Config;

impl Config {
    pub(super) fn apply_observability_env<E: EnvLookup + ?Sized>(&mut self, env: &E) {
        let dsn_value = env
            .get("OPENHUMAN_CORE_SENTRY_DSN")
            .or_else(|| env.get("OPENHUMAN_SENTRY_DSN"))
            .or_else(|| option_env!("OPENHUMAN_CORE_SENTRY_DSN").map(|s| s.to_string()))
            .or_else(|| option_env!("OPENHUMAN_SENTRY_DSN").map(|s| s.to_string()));
        if let Some(dsn) = dsn_value {
            let dsn = dsn.trim();
            if !dsn.is_empty() {
                self.observability.sentry_dsn = Some(dsn.to_string());
            }
        }

        if let Some(flag) = env.get("OPENHUMAN_ANALYTICS_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.observability.analytics_enabled = true,
                "0" | "false" | "no" | "off" => self.observability.analytics_enabled = false,
                _ => {}
            }
        }

        if let Some(flag) = env.get("OPENHUMAN_SHARE_USAGE_DATA") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => self.observability.share_usage_data = true,
                "0" | "false" | "no" | "off" => self.observability.share_usage_data = false,
                _ => {}
            }
        }

        // Operators opt in to content independently of trace metadata sharing.
        if let Some(flag) = env.get("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => {
                    self.observability.agent_tracing.capture_content = true
                }
                "0" | "false" | "no" | "off" => {
                    self.observability.agent_tracing.capture_content = false
                }
                _ => {}
            }
        }
    }
}
