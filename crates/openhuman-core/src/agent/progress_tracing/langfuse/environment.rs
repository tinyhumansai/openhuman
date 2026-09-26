//! Backend-host resolution and the deployment-environment push gate for the
//! Langfuse exporter: where the ingestion proxy lives, which environment the
//! resolved host belongs to, and whether that environment may push at all.

use crate::api::config::effective_backend_api_url;
use crate::config::Config;

use super::LOG_TARGET;

/// Backend proxy route for Langfuse ingestion (relative to the backend origin).
/// The backend authenticates the caller's session JWT, injects the Langfuse
/// project keys, and forwards to Langfuse's real `/api/public/ingestion` — so
/// clients POST here, NOT to `/api/public/ingestion` (which is unexposed and
/// carries no keys).
const INGESTION_PATH: &str = "/telemetry/langfuse/ingestion";

/// Resolve the Langfuse ingestion URL from the current backend host. Joins the
/// proxy path onto [`effective_backend_api_url`] — the exact base-server
/// resolution every other backend call uses — via the canonical
/// [`crate::api::config::api_url`] helper, which replaces any path the base
/// carried with the given absolute path. So the host always matches wherever the
/// app's domain calls go (staging, prod, or a custom `api_url` override).
pub(crate) fn ingestion_url(config: &Config) -> String {
    let base = effective_backend_api_url(&config.api_url);
    crate::api::config::api_url(&base, INGESTION_PATH)
}

/// The domain the deployed backends live under. A host outside it cannot be
/// one of ours, so it cannot be staging however it is spelled.
const DEPLOYMENT_DOMAIN: &str = "tinyhumans.ai";

/// Derive the Langfuse `environment` for a backend base URL. Chosen signal:
/// the resolved backend host is the single existing config-driven fact that
/// distinguishes deployments (there is no NODE_ENV-style flag in the core
/// config) — loopback/local → development, a staging host under
/// [`DEPLOYMENT_DOMAIN`] → staging, the canonical API host → production,
/// anything else → external.
///
/// # Why the host is parsed rather than substring-matched
///
/// This used to test `base.contains("staging")` and
/// `base.contains("localhost" | "127.0.0.1" | "0.0.0.0")` against the whole
/// URL, which was tolerable while the answer only labelled a payload. It is
/// not tolerable now that [`skip_push`] decides whether to push at all, so
/// both directions of the sloppiness became load-bearing:
///
/// - **Too broad on staging.** `https://staging-attacker.invalid` contains
///   `staging`, so it classified as staging and passed the push gate — a host
///   nothing in this tree owns, reached with a live session token. Anchoring
///   to [`DEPLOYMENT_DOMAIN`] is what makes the classifier fail *closed*: a
///   host that is not ours is external, and external does not push.
/// - **Too narrow on local.** An IPv6 loopback backend (`http://[::1]:7788`)
///   or a private LAN address matched none of the three literals and
///   classified as external, so a working development setup would have
///   silently stopped exporting the moment the gate landed.
///
/// Host classification is delegated to [`crate::api::config::host_is_local`],
/// which already parses the URL and handles IPv4 loopback/unspecified/private,
/// IPv6 loopback/unspecified, and `localhost` / `*.localhost`. Keeping one
/// definition matters more than the few lines it saves: two local-host
/// predicates that disagree is how a gate lets through exactly the case the
/// other one blocks.
///
/// An unparseable URL is external — the fail-closed default. `ingestion_url`
/// can return a non-URL placeholder when no backend host resolves, and the
/// caller checks `starts_with("http")` separately; classifying that as
/// anything pushable would defeat the gate.
pub(crate) fn environment_for_base(base: &str) -> &'static str {
    let Ok(parsed) = url::Url::parse(base) else {
        return "external";
    };
    if crate::api::config::host_is_local(&parsed) {
        return "development";
    }
    if parsed.scheme() != "https" {
        return "external";
    }
    let Some(url::Host::Domain(host)) = parsed.host() else {
        // A public IP literal is not a deployment of ours.
        return "external";
    };
    let host = host.to_ascii_lowercase();
    let under_deployment_domain =
        host == DEPLOYMENT_DOMAIN || host.ends_with(&format!(".{DEPLOYMENT_DOMAIN}"));
    // The label test is on the leftmost label only, so `staging-api…` and
    // `staging…` match while `api-staging-mirror…` does not sneak in on a
    // substring.
    let leftmost_is_staging = host
        .split('.')
        .next()
        .is_some_and(|label| label == "staging" || label.starts_with("staging-"));
    if host == "api.tinyhumans.ai" {
        "production"
    } else if under_deployment_domain && leftmost_is_staging {
        "staging"
    } else {
        "external"
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
