// Managed-provider helpers for the `/models` listing.
//
// Split out of `models.rs` to keep it under the repo's per-file line limit.
// `include!`d textually rather than declared as a module, so these stay private
// to `models` and the existing test module sees them unchanged — which also
// means only `//` comments are legal here, not `//!`.
//
// All three exist because the MANAGED backend is not just another entry in
// `cloud_providers`: it has its own URL, its own credential, and its own
// meaning for a 401.

/// Synthesize a transient [`CloudProviderCreds`] entry for the well-known
/// local-runtime slugs (`ollama`, `lmstudio`) so [`list_configured_models`]
/// can probe their OpenAI-compatible `/v1/models` endpoint even when the
/// user has not registered a matching `cloud_providers` row.
///
/// Background: the AI settings panel registers an `ollama` `cloud_providers`
/// entry when the user configures Ollama (see comment on
/// [`crate::openhuman::config::schema::cloud_providers::is_slug_reserved`]),
/// but in practice some users hit
/// `inference_list_models("ollama")` without that entry — config drift,
/// flush-vs-probe race, or upgrade from a build that only persisted
/// `config.local_ai.base_url`. Sentry TAURI-RUST-28Z captures this:
/// 24 events / 7d, all `domain=rpc, method=openhuman.inference_list_models,
/// operation=invoke_method`. Without this fallback, the dropdown surfaces
/// the bare `"no cloud provider with id or slug 'ollama' found"` error
/// (also visible in the Sentry breadcrumb) instead of returning models.
///
/// Returns `None` for any slug that is not a recognized local-runtime
/// alias — callers continue down the normal "no cloud provider" error
/// path for `openai` / `anthropic` / opaque ids / typos.
/// Synthesize the managed (`openhuman`) provider entry when `cloud_providers`
/// has no row for it.
///
/// The row is normally seeded by the `unify_ai_provider_settings` migration,
/// but it is absent in a freshly created profile — which is exactly what the
/// app falls back to when a stored session is rejected server-side. Managed is
/// the product's own backend, not user-supplied configuration, so requiring a
/// config row to list its models turned "signed out" into
/// "no cloud provider with id or slug 'openhuman' found".
///
/// Endpoint and auth style only have to identify the entry as managed: the
/// caller replaces the URL with `effective_backend_api_url` and the credential
/// with the live session token before the request goes out.
fn synthesize_managed_entry(
    slug: &str,
) -> Option<crate::openhuman::config::schema::cloud_providers::CloudProviderCreds> {
    use crate::openhuman::config::schema::cloud_providers::{AuthStyle, CloudProviderCreds};
    if slug != "openhuman" {
        return None;
    }
    Some(CloudProviderCreds {
        id: "openhuman".to_string(),
        slug: "openhuman".to_string(),
        label: "OpenHuman".to_string(),
        endpoint: crate::openhuman::config::schema::cloud_providers::CloudProviderType::Openhuman
            .default_endpoint()
            .to_string(),
        auth_style: AuthStyle::OpenhumanJwt,
        ..Default::default()
    })
}

/// Whether a bearer credential may be attached to this URL.
///
/// `https` always, plus loopback over plain `http` so a locally-hosted backend
/// (`BACKEND_URL=http://127.0.0.1:5005`) still authenticates in development. Any
/// other plaintext destination gets the request without the credential rather
/// than leaking it.
fn url_is_credential_safe(url: &str) -> bool {
    match reqwest::Url::parse(url) {
        Ok(parsed) => {
            if parsed.scheme() == "https" {
                return true;
            }
            matches!(
                parsed.host_str(),
                Some("localhost" | "127.0.0.1" | "::1" | "[::1]")
            )
        }
        // Unparseable: treat as unsafe rather than guessing.
        Err(_) => false,
    }
}

/// Whether a non-2xx from a `/models` probe should be read as "signed out"
/// rather than a provider failure.
///
/// Only for the MANAGED provider, and only on 401. A managed 401 means the
/// stored session was rejected server-side — often while its local `exp` is
/// still valid, so nothing upstream flagged it — which is a signed-out state,
/// not a broken provider. For a BYOK provider a 401 IS the actionable error (a
/// wrong or revoked API key); swallowing it there would strand the user with a
/// silently empty dropdown and no clue why.
/// Whether the app session token will actually be attached to the managed
/// request — the input to [`managed_401_means_signed_out`].
///
/// This is deliberately not "is there a session": `url_is_credential_safe`
/// refuses to put a bearer token on a non-https, non-loopback URL, and in that
/// case the request goes out unauthenticated even though a live session exists.
/// A 401 on that request describes the misconfigured URL, not the session.
///
/// Note the token-emptiness check in the caller is subsumed here: whenever
/// `managed_token` is non-empty it *is* the token being sent, so a non-empty
/// managed token plus a safe URL is exactly the attach condition.
fn managed_session_attaches(managed_token: &str, url: &str) -> bool {
    !managed_token.is_empty() && url_is_credential_safe(url)
}

fn managed_401_means_signed_out(
    status: u16,
    auth_style: crate::openhuman::config::schema::cloud_providers::AuthStyle,
    managed_session_attached: bool,
) -> bool {
    use crate::openhuman::config::schema::cloud_providers::AuthStyle;
    // Only a 401 on a request that actually carried the app session token says
    // anything about the session. The two other ways to reach a 401 here are
    // real, actionable faults that must not be hidden behind an empty catalog
    // just because the entry's auth_style is OpenhumanJwt (review, #6206):
    //
    //   * no session, so the request went out on the provider-scoped fallback
    //     key — a 401 means THAT key is wrong or revoked.
    //   * a live session that the credential-safety guard refused to attach
    //     (non-https, non-loopback backend URL) — the request was
    //     unauthenticated, and the misconfigured URL is the thing to report.
    status == 401 && auth_style == AuthStyle::OpenhumanJwt && managed_session_attached
}
