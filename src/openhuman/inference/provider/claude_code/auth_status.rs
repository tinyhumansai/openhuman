//! Detect Claude Code CLI auth state without spawning the binary.
//!
//! Surfaces three sources, in priority order:
//!   1. `ANTHROPIC_API_KEY` env var present → `ApiKeyEnv`.
//!   2. `~/.claude/.credentials.json` parseable → `Subscription` (Claude
//!      Pro / Max OAuth tokens land here after `claude login`).
//!   3. Neither → `None`.
//!
//! The credentials file is the CLI's source of truth; we never write to it
//! and never round-trip the access token through RPC. We extract only
//! non-secret metadata (account email, expiry) when the schema exposes it,
//! and fall back to `Subscription { account_email: None, expires_at: None }`
//! when Anthropic changes the shape on us.

use std::path::PathBuf;
use std::time::SystemTime;

use serde::{Deserialize, Serialize};

/// Discriminator for who actually authenticates the spawned CLI.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "source")]
pub enum AuthSource {
    /// Claude Pro / Max subscription — OAuth tokens in
    /// `~/.claude/.credentials.json`. Account email + expiry returned
    /// best-effort; absent when the schema drifts.
    Subscription {
        account_email: Option<String>,
        /// RFC3339-ish timestamp string copied verbatim from credentials
        /// when present. We do not parse + compare; UI surfaces it as
        /// "last seen" rather than a confident countdown.
        expires_at: Option<String>,
    },
    /// `ANTHROPIC_API_KEY` is set in the core process env. The spawned
    /// CLI inherits it.
    ApiKeyEnv,
    /// Nothing detected. The CLI will fail any chat with an auth error.
    None,
}

/// Returned by the `claude_code_auth_status` RPC. Snake-case Serde so the
/// TS side discriminates on `source`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuthStatus {
    #[serde(flatten)]
    pub source: AuthSource,
    /// Unix seconds when this probe ran — UI shows "last checked" so users
    /// can tell a stale subscription badge from a fresh one.
    pub last_checked: u64,
}

/// Resolve the on-disk path to `~/.claude/.credentials.json`. Overridable
/// via `OPENHUMAN_CLAUDE_CREDENTIALS` for tests.
pub fn credentials_path() -> Option<PathBuf> {
    if let Ok(explicit) = std::env::var("OPENHUMAN_CLAUDE_CREDENTIALS") {
        return Some(PathBuf::from(explicit));
    }
    dirs_next_home().map(|h| h.join(".claude").join(".credentials.json"))
}

fn dirs_next_home() -> Option<PathBuf> {
    // Mirror the stdlib's home detection without pulling another dep.
    #[cfg(windows)]
    {
        if let Ok(p) = std::env::var("USERPROFILE") {
            return Some(PathBuf::from(p));
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(p) = std::env::var("HOME") {
            return Some(PathBuf::from(p));
        }
    }
    None
}

/// Tolerant credentials parser. Inspects a few known shape variants
/// without committing to any of them; on any failure we still return a
/// `Subscription { None, None }` because the file existing at all is
/// strong evidence the user has logged in.
fn parse_credentials(raw: &str) -> AuthSource {
    let val: serde_json::Value = match serde_json::from_str(raw) {
        Ok(v) => v,
        Err(_) => {
            return AuthSource::Subscription {
                account_email: None,
                expires_at: None,
            };
        }
    };

    // Schema observed in the wild:
    //   { "claudeAiOauth": { "accessToken": "...", "expiresAt": "...",
    //                        "subscriptionType": "max", "email": "..." } }
    // We probe a few plausible spellings to be drift-tolerant.
    let oauth_obj = val
        .get("claudeAiOauth")
        .or_else(|| val.get("oauth"))
        .or_else(|| val.get("claude_ai_oauth"));

    let lookup_str = |obj: &serde_json::Value, key: &str| -> Option<String> {
        obj.get(key).and_then(|v| v.as_str()).map(str::to_string)
    };

    if let Some(obj) = oauth_obj {
        let email = lookup_str(obj, "email")
            .or_else(|| lookup_str(obj, "account_email"))
            .or_else(|| lookup_str(obj, "accountEmail"));
        let expires = lookup_str(obj, "expiresAt").or_else(|| lookup_str(obj, "expires_at"));
        return AuthSource::Subscription {
            account_email: email,
            expires_at: expires,
        };
    }

    // Top-level email/expiresAt fallback.
    let email = lookup_str(&val, "email");
    let expires = lookup_str(&val, "expiresAt").or_else(|| lookup_str(&val, "expires_at"));
    AuthSource::Subscription {
        account_email: email,
        expires_at: expires,
    }
}

/// Probe auth state. Pure FS work — no CLI spawn, no network.
pub fn probe() -> AuthStatus {
    let last_checked = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        if !k.trim().is_empty() {
            return AuthStatus {
                source: AuthSource::ApiKeyEnv,
                last_checked,
            };
        }
    }

    let source = match credentials_path() {
        Some(p) if p.is_file() => match std::fs::read_to_string(&p) {
            Ok(raw) => parse_credentials(&raw),
            // File exists but unreadable — still signal "signed in" rather
            // than "none" so the user gets accurate UX. The CLI itself
            // will surface a permission error on next turn.
            Err(_) => AuthSource::Subscription {
                account_email: None,
                expires_at: None,
            },
        },
        _ => AuthSource::None,
    };

    AuthStatus {
        source,
        last_checked,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_known_oauth_shape() {
        let raw = r#"{
            "claudeAiOauth": {
                "accessToken": "redacted",
                "refreshToken": "redacted",
                "expiresAt": "2026-06-01T00:00:00Z",
                "subscriptionType": "max",
                "email": "user@example.com"
            }
        }"#;
        match parse_credentials(raw) {
            AuthSource::Subscription {
                account_email,
                expires_at,
            } => {
                assert_eq!(account_email.as_deref(), Some("user@example.com"));
                assert_eq!(expires_at.as_deref(), Some("2026-06-01T00:00:00Z"));
            }
            other => panic!("expected Subscription, got {other:?}"),
        }
    }

    #[test]
    fn drift_falls_back_to_subscription_without_details() {
        let raw = r#"{ "some_future_shape": { "token": "x" } }"#;
        match parse_credentials(raw) {
            AuthSource::Subscription {
                account_email,
                expires_at,
            } => {
                assert!(account_email.is_none());
                assert!(expires_at.is_none());
            }
            other => panic!("expected Subscription fallback, got {other:?}"),
        }
    }

    #[test]
    fn malformed_json_still_returns_subscription() {
        match parse_credentials("not json at all") {
            AuthSource::Subscription { .. } => {}
            other => panic!("expected Subscription, got {other:?}"),
        }
    }

    #[test]
    fn probe_returns_none_when_no_env_and_no_file() {
        // Force the lookup to a path we control that doesn't exist.
        let tmp = std::env::temp_dir().join("openhuman-test-nonexistent-creds.json");
        if tmp.exists() {
            std::fs::remove_file(&tmp).ok();
        }
        // Save & clear env so the test is hermetic.
        let prev_key = std::env::var("ANTHROPIC_API_KEY").ok();
        let prev_creds = std::env::var("OPENHUMAN_CLAUDE_CREDENTIALS").ok();
        std::env::remove_var("ANTHROPIC_API_KEY");
        std::env::set_var("OPENHUMAN_CLAUDE_CREDENTIALS", &tmp);

        let s = probe();
        assert!(matches!(s.source, AuthSource::None));

        // Restore env to avoid bleed.
        match prev_key {
            Some(v) => std::env::set_var("ANTHROPIC_API_KEY", v),
            None => std::env::remove_var("ANTHROPIC_API_KEY"),
        }
        match prev_creds {
            Some(v) => std::env::set_var("OPENHUMAN_CLAUDE_CREDENTIALS", v),
            None => std::env::remove_var("OPENHUMAN_CLAUDE_CREDENTIALS"),
        }
    }
}
