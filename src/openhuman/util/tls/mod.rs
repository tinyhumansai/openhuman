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
//! - **macOS / Linux**: `rustls` + webpki-roots — avoids the OpenSSL runtime
//!   dependency on Linux and has historically been more reliable on macOS staging
//!   TLS handshakes than `native-tls`.

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
    let b = b.use_rustls_tls();
    b
}
