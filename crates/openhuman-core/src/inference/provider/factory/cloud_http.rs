//! HTTP-client policy shared by native BYOK construction.
//! Credentialed routes never follow redirects: a custom authorization header
//! must not travel to a different host or a plaintext HTTP endpoint.
use crate::config::schema::cloud_providers::AuthStyle;

pub(super) fn build_cloud_http_client(
    style: AuthStyle,
    anthropic_native: bool,
    ca_pem: Option<&str>,
) -> Result<Option<reqwest::Client>, String> {
    let service_key = if anthropic_native {
        "provider.anthropic"
    } else {
        "provider.compatible"
    };
    let pem = ca_pem.filter(|value| !value.is_empty());
    if matches!(style, AuthStyle::Bearer | AuthStyle::Anthropic) {
        let client = crate::util::tls::credentialed_client_builder(service_key, pem)?
            .build()
            .map_err(|_| "Could not build secure provider inference HTTP client".to_string())?;
        return Ok(Some(client));
    }
    match pem {
        Some(pem) => crate::util::tls::client_with_ca_bundle(pem, service_key).map(Some),
        None => Ok(None),
    }
}

#[cfg(test)]
#[path = "cloud_http_tests.rs"]
mod tests;
