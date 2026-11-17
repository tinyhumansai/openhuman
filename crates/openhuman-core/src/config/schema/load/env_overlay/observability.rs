//! Env overrides for Sentry, analytics, and agent-tracing content capture.

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
                let env_var = if env.contains("OPENHUMAN_CORE_SENTRY_DSN") {
                    Some("OPENHUMAN_CORE_SENTRY_DSN")
                } else if env.contains("OPENHUMAN_SENTRY_DSN") {
                    Some("OPENHUMAN_SENTRY_DSN")
                } else if option_env!("OPENHUMAN_CORE_SENTRY_DSN").is_some() {
                    Some("OPENHUMAN_CORE_SENTRY_DSN")
                } else if option_env!("OPENHUMAN_SENTRY_DSN").is_some() {
                    Some("OPENHUMAN_SENTRY_DSN")
                } else {
                    None
                };
                if let Some(env_var) = env_var {
                    self.record_env_override_var(env, "observability.sentry_dsn", env_var);
                }
            }
        }

        if let Some(flag) = env.get("OPENHUMAN_ANALYTICS_ENABLED") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => {
                    self.observability.analytics_enabled = true;
                    self.record_env_override(
                        env,
                        "observability.analytics_enabled",
                        &["OPENHUMAN_ANALYTICS_ENABLED"],
                    );
                }
                "0" | "false" | "no" | "off" => {
                    self.observability.analytics_enabled = false;
                    self.record_env_override(
                        env,
                        "observability.analytics_enabled",
                        &["OPENHUMAN_ANALYTICS_ENABLED"],
                    );
                }
                _ => {}
            }
        }

        // Prompt/reply content is captured by default; operators can disable
        // it independently of token and cost telemetry.
        if let Some(flag) = env.get("OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT") {
            let normalized = flag.trim().to_ascii_lowercase();
            match normalized.as_str() {
                "1" | "true" | "yes" | "on" => {
                    self.observability.agent_tracing.capture_content = true;
                    self.record_env_override(
                        env,
                        "observability.agent_tracing.capture_content",
                        &["OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT"],
                    );
                }
                "0" | "false" | "no" | "off" => {
                    self.observability.agent_tracing.capture_content = false;
                    self.record_env_override(
                        env,
                        "observability.agent_tracing.capture_content",
                        &["OPENHUMAN_AGENT_TRACING_CAPTURE_CONTENT"],
                    );
                }
                _ => {}
            }
        }
    }
}
