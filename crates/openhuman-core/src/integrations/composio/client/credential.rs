//! The key and base URL a direct-mode read is made with.
//!
//! Direct reads run in the `tinyconnectors` module over the bus
//! (`ListConnectionsDirect` / `ListToolsDirect`). The module owns the HTTP, the
//! paths and the response reshaping; this type is only what the host decides
//! and validates before a call: which key, and which base URL, refusing
//! anything that could send the key over plain HTTP to a remote host.

use tinyconnectors_bus::ComposioDirectCredential;

/// Composio's production v3 API root. The module's own default is the same
/// URL, so a credential on it sends no override.
const COMPOSIO_API_BASE_V3: &str = "https://backend.composio.dev/api/v3";

pub(super) fn is_loopback_http_url(url: &str) -> bool {
    // Parse rather than prefix-match: a raw `starts_with("http://127.0.0.1:")`
    // is fooled by userinfo smuggling like
    // `http://127.0.0.1:8080@evil.com/api/v3/tools`, which an HTTP client routes
    // to the *parsed* host (`evil.com`). Verify the actual scheme + host and
    // reject any embedded credentials so the insecure-loopback path can never
    // leak the `x-api-key` header to a non-loopback host.
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    if parsed.scheme() != "http" {
        return false;
    }
    if !parsed.username().is_empty() || parsed.password().is_some() {
        return false;
    }
    match parsed.host() {
        Some(url::Host::Domain(host)) => host.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
        Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
        None => false,
    }
}

pub(super) fn is_loopback_http_base(url: &str) -> bool {
    is_loopback_http_url(&format!("{}/", url.trim_end_matches('/')))
}

/// A Composio API key and the v3 base URL to use it against.
///
/// Held by [`super::ComposioRoute::Direct`]. The key is never printed: `Debug`
/// shows the base URL only.
pub struct DirectCredential {
    api_key: String,
    /// `None` is Composio's production API; `Some` is a validated override (a
    /// host-pinned tenant URL, or a loopback test server in debug builds).
    base_v3: Option<String>,
}

// Manual `Debug`: never prints the key.
impl std::fmt::Debug for DirectCredential {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DirectCredential")
            .field("base_v3", &self.base_v3)
            .finish_non_exhaustive()
    }
}

impl DirectCredential {
    /// A credential for Composio's production API.
    pub fn new(api_key: &str) -> Self {
        Self::new_internal(api_key, None)
    }

    pub(crate) fn auth_key_fingerprint(&self) -> u64 {
        crate::integrations::composio::direct_auth::fingerprint_api_key(&self.api_key)
    }

    /// Debug-test seam for raw integration coverage: a credential against
    /// explicit v2/v3 base URLs. Non-HTTPS URLs are accepted only for loopback
    /// hosts and only in debug builds.
    ///
    /// The v2 root is validated for the same safety rule but not kept: no
    /// remaining request path uses it.
    #[cfg(debug_assertions)]
    pub fn new_with_base_urls_for_loopback(
        api_key: &str,
        base_v2: String,
        base_v3: String,
    ) -> anyhow::Result<Self> {
        for base in [&base_v2, &base_v3] {
            if !base.starts_with("https://") && !is_loopback_http_base(base) {
                anyhow::bail!("debug Composio base URL must be HTTPS or loopback HTTP");
            }
        }
        Ok(Self::new_internal(api_key, Some(base_v3)))
    }

    /// A credential against explicit v2/v3 API roots. Both must be HTTPS;
    /// loopback HTTP is accepted only in debug builds.
    pub fn new_with_base_urls(
        api_key: &str,
        base_v2: String,
        base_v3: String,
    ) -> anyhow::Result<Self> {
        let allow_loopback = cfg!(debug_assertions);
        for base in [&base_v2, &base_v3] {
            let accepted =
                base.starts_with("https://") || (allow_loopback && is_loopback_http_base(base));
            if !accepted {
                anyhow::bail!("Composio base URL must be HTTPS");
            }
        }
        Ok(Self::new_internal(api_key, Some(base_v3)))
    }

    /// Test-only seam: a credential against an explicit v3 base so a test can
    /// point the module's reads at a local mock instead of `backend.composio.dev`.
    ///
    /// `#[cfg(test)]`-gated on purpose: an injectable base must never carry a
    /// non-HTTPS URL outside tests. Production reaches the module through
    /// [`Self::new`] or the validated constructors above.
    #[cfg(test)]
    pub(crate) fn new_with_v3_base(api_key: &str, base_v3: String) -> Self {
        Self::new_internal(api_key, Some(base_v3))
    }

    fn new_internal(api_key: &str, base_v3: Option<String>) -> Self {
        let trimmed = api_key.trim();
        if trimmed.len() != api_key.len() {
            // The key carried leading/trailing whitespace that would otherwise
            // reach Composio's `x-api-key` header verbatim and trip the
            // server-side "Invalid API key format" 401 (Sentry TAURI-RUST-D3).
            // We trim here so the request succeeds; logging the length delta
            // (never the key itself) helps trace which credential source
            // produced a dirty value without leaking the secret.
            tracing::debug!(
                original_len = api_key.len(),
                trimmed_len = trimmed.len(),
                "[composio] trimmed leading/trailing whitespace from api_key"
            );
        }
        Self {
            api_key: trimmed.to_string(),
            base_v3,
        }
    }

    /// The v3 base this credential reads from.
    fn effective_base(&self) -> &str {
        self.base_v3.as_deref().unwrap_or(COMPOSIO_API_BASE_V3)
    }

    /// What the module is handed for one read: this key, the base override if
    /// there is one, and the host's proxy and TLS policy for that destination.
    pub(super) fn module_credential(&self) -> ComposioDirectCredential {
        ComposioDirectCredential {
            api_key: self.api_key.clone(),
            entity_id: None,
            base_url: self.base_v3.clone(),
            transport: super::network::module_transport(self.effective_base()),
            webhooks: false,
        }
    }
}
