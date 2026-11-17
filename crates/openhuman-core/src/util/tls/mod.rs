//! Platform-conditional TLS backend selection for reqwest clients.
//!
//! Centralises the `#[cfg(target_os = "windows")]` / `#[cfg(not(target_os = "windows"))]`
//! guard so every HTTP-client construction site stays at one line and future
//! policy changes (e.g. adding native-tls on macOS) only require editing this file.
//!
//! # Policy
//! - **Windows**: `native-tls` (schannel) — honors the Windows certificate store,
//!   including any corporate CA installed by AV / TLS-inspecting proxies that
//!   re-sign certificates with a private root. `rustls` + webpki-roots only knows
//!   Mozilla CAs and fails such environments with `UnknownIssuer`.
//! - **macOS / Linux**: `rustls` with Mozilla and native roots — trusts corporate
//!   CAs in the OS store or `SSL_CERT_FILE` while avoiding the OpenSSL runtime
//!   dependency on Linux.

/// Return a `reqwest::ClientBuilder` pre-configured with the platform-appropriate
/// TLS backend.
///
/// Use this as the starting point for every client that needs to reach external
/// HTTPS endpoints:
/// ```rust,ignore
/// let client = tls_client_builder()
///     .http1_only()
///     .timeout(Duration::from_secs(30))
///     .build()?;
/// ```
pub fn tls_client_builder() -> reqwest::ClientBuilder {
    let b = reqwest::Client::builder();
    #[cfg(target_os = "windows")]
    let b = b.use_native_tls();
    #[cfg(not(target_os = "windows"))]
    let b = b.use_rustls_tls().tls_built_in_native_certs(true);
    b
}

/// Parse a user-supplied PEM CA bundle without accepting private-key material.
pub fn parse_ca_bundle(pem: &str) -> Result<Vec<reqwest::Certificate>, String> {
    if pem.len() > 256 * 1024 || pem.contains("PRIVATE KEY") {
        return Err("CA bundle must contain certificates only and be at most 256 KiB".into());
    }
    let certificates = reqwest::Certificate::from_pem_bundle(pem.as_bytes())
        .map_err(|_| "CA bundle is not valid PEM certificate data".to_string())?;
    if certificates.is_empty() {
        return Err("CA bundle contains no certificates".into());
    }
    Ok(certificates)
}

/// Build a provider-scoped client builder with extra trusted CA certificates.
fn ca_client_builder(pem: &str, service_key: &str) -> Result<reqwest::ClientBuilder, String> {
    let mut builder =
        crate::config::apply_runtime_proxy_to_builder(tls_client_builder(), service_key);
    for certificate in parse_ca_bundle(pem)? {
        builder = builder.add_root_certificate(certificate);
    }
    Ok(builder)
}

pub fn client_with_ca_bundle(pem: &str, service_key: &str) -> Result<reqwest::Client, String> {
    let builder = ca_client_builder(pem, service_key)?;
    builder
        .build()
        .map_err(|error| format!("Could not build provider TLS client: {error}"))
}

pub fn client_with_ca_bundle_with_timeouts(
    pem: &str,
    service_key: &str,
    timeout_secs: u64,
    connect_timeout_secs: u64,
) -> Result<reqwest::Client, String> {
    ca_client_builder(pem, service_key)?
        .timeout(std::time::Duration::from_secs(timeout_secs))
        .connect_timeout(std::time::Duration::from_secs(connect_timeout_secs))
        .build()
        .map_err(|error| format!("Could not build provider TLS client: {error}"))
}

#[cfg(test)]
#[path = "mod_tests.rs"]
mod tests;
