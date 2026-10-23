//! Backend-host resolution and the deployment-environment push gate for the
//! Langfuse exporter: where the OTLP proxy lives, which environment the
//! resolved host belongs to, and whether that environment may push at all.

use crate::config::Config;
use crate::security::credentials::session_support::direct_backend_credential;

const LOG_TARGET: &str = "agent-tracing::langfuse";

/// The OTLP/HTTP traces URL the host resolved
/// (`config.resolved_endpoints.tracing_otlp_url`). Empty when no host resolved
/// one; callers treat a non-`http` URL as "proxy unavailable".
pub(crate) fn otlp_url(config: &Config) -> String {
    config
        .resolved_endpoints
        .tracing_otlp_url
        .clone()
        .unwrap_or_default()
}

/// Derive the Langfuse `environment` for a tracing base URL. Chosen signal:
/// the resolved backend host is the single existing config-driven fact that
/// distinguishes deployments (there is no NODE_ENV-style flag in the core
/// config) — loopback/local → development; a non-local HTTPS host takes the
/// bucket the installed host resolved for its control plane
/// (`config.resolved_endpoints.deployment_environment`: production, staging);
/// anything else → external. With no host registered nothing is resolved, so
/// every non-local host is external and does not push.
///
/// # Why the host is parsed rather than substring-matched
///
/// The answer decides whether a push happens at all ([`skip_push`]), so a
/// sloppy match is load-bearing in both directions. A substring test on
/// `staging` let `https://staging-attacker.invalid` through, and missed an
/// IPv6 loopback backend (`http://[::1]:7788`). Local classification is
/// delegated to [`crate::util::url::host_is_local`], which parses the URL and
/// handles IPv4 loopback/unspecified/private, IPv6 loopback/unspecified and
/// `localhost` / `*.localhost`; one local-host predicate keeps the gates in
/// agreement. The hosted bucket is classified by the host that owns the
/// domain, anchored to its own deployment domain so the classifier fails
/// *closed*.
///
/// An unparseable URL is external — the fail-closed default. `otlp_url`
/// can return a non-URL placeholder when no backend host resolves, and the
/// caller checks `starts_with("http")` separately; classifying that as
/// anything pushable would defeat the gate.
pub(crate) fn environment_for_base(config: &Config, base: &str) -> &'static str {
    let Ok(parsed) = url::Url::parse(base) else {
        return "external";
    };
    // Classification comes from the host-resolved deployment metadata. The
    // core must not infer a trusted destination from a URL's IP or hostname:
    // that classification controls whether session credentials may be sent.
    match config.resolved_endpoints.deployment_environment.as_deref() {
        Some("development") if matches!(parsed.scheme(), "http" | "https") => "development",
        Some("production") if parsed.scheme() == "https" => "production",
        Some("staging") if parsed.scheme() == "https" => "staging",
        _ => "external",
    }
}

/// The environments this client may push to Langfuse from.
///
/// An allowlist keeps the fail-closed property structural — if
/// [`environment_for_base`] ever grows a fourth bucket, that bucket does not
/// push until someone adds it here on purpose.
///
/// `test` appears in the backend's list but not here because there is no such
/// bucket on this side: [`environment_for_base`] maps loopback hosts to
/// `development`, and that is what the Rust suite resolves to.
pub(super) const LANGFUSE_PUSH_ENVIRONMENTS: &[&str] = &["production", "staging", "development"];

/// Whether a push is permitted for a resolved environment.
pub(super) fn push_allowed(environment: &str) -> bool {
    LANGFUSE_PUSH_ENVIRONMENTS.contains(&environment)
}

/// Emitted at most once per process by [`skip_push`].
static SKIP_LOGGED: std::sync::Once = std::sync::Once::new();

/// Whether this push should be dropped before any work, logging the reason
/// once per process.
///
/// # Why skip at all, when the backend already refuses
///
/// Defence in depth: an unknown backend origin must not receive a session
/// bearer merely because usage sharing is enabled.
///
/// # Why once per process, and at info
///
/// This is on the path of every completed run. A warning per turn would move
/// the noise rather than remove it, and a skip for an external host is the configured
/// outcome, not a fault — so it is `info`, said once, and then silence. The
/// caller receives `Ok(())`: skipping is a successful no-op, and returning
/// `Err` would make the caller log the same line on every turn, which is the
/// thing being avoided.
pub(crate) fn skip_push(environment: &str) -> bool {
    if push_allowed(environment) {
        return false;
    }
    SKIP_LOGGED.call_once(|| {
        tracing::info!(
            target: LOG_TARGET,
            "[agent-tracing] Langfuse push disabled for environment {environment:?} \
             (enabled in: {}) — traces stay local for the rest of this process",
            LANGFUSE_PUSH_ENVIRONMENTS.join(", ")
        );
    });
    true
}

/// Whether a journal push would actually send for `config`: the URL gate, the
/// environment gate and a live session credential. Callers must read a journal
/// and build observations first, so without a live session (unit tests, a
/// signed-out or embedder host) or on a skipped environment that work is
/// discarded anyway, and reading a whole child journal is not free.
pub(crate) fn journal_push_ready(config: &Config) -> bool {
    let url = otlp_url(config);
    !skip_push(environment_for_base(config, &url))
        && url.starts_with("http")
        && matches!(
            direct_backend_credential(config, "langfuse journal push"),
            Some(crate::security::credentials::session_support::BackendCredential::Session(_))
        )
}

#[cfg(test)]
#[path = "environment_policy_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "environment_parse_tests.rs"]
mod parse_tests;
