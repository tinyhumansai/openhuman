//! Resolve an `ANTHROPIC_API_KEY` for the spawned `claude` CLI.
//!
//! v1 resolution order:
//!   1. Process env `ANTHROPIC_API_KEY` (highest precedence).
//!   2. `~/.claude/.credentials.json` — only used if the CLI is already
//!      logged in via `claude login`. We pass it through transparently by
//!      *not* setting `ANTHROPIC_API_KEY`; the CLI then reads its own
//!      credentials file.
//!
//! v1.1 will wire OpenHuman `AuthService` (auth-profiles.json) so an
//! Anthropic key stored in settings is picked up automatically.
//! Subscription / OAuth auth (Claude Pro/Max) deferred to v2.

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AuthSource {
    /// Explicit API key — pass via `ANTHROPIC_API_KEY` env var.
    EnvApiKey,
    /// No explicit key resolved. Defer to whatever the CLI finds in
    /// `~/.claude/.credentials.json`.
    CliCredentials,
}

/// Probe sources in priority order. Returns the resolved API key plus the
/// origin label (for logging) when found. The returned key is only the
/// key value — call-sites set env on spawn, never log it.
pub fn resolve() -> (AuthSource, Option<String>) {
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        let k = k.trim();
        if !k.is_empty() {
            return (AuthSource::EnvApiKey, Some(k.to_string()));
        }
    }
    (AuthSource::CliCredentials, None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_cli_credentials_without_env() {
        let _env = super::super::ENV_TEST_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let prev = std::env::var("ANTHROPIC_API_KEY").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");

        let (src, key) = resolve();
        assert_eq!(src, AuthSource::CliCredentials);
        assert!(key.is_none());

        if let Some(v) = prev {
            std::env::set_var("ANTHROPIC_API_KEY", v);
        }
    }
}
