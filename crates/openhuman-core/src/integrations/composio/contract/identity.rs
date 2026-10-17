//! Identity canonicalisation and prompt rendering, served by the connector
//! module's `NormalizeIdentities` member.
//!
//! The vocabulary ([`ConnectedIdentity`](super::ConnectedIdentity),
//! [`IdentityKind`](super::IdentityKind),
//! [`ProviderUserProfile`](super::ProviderUserProfile)) is the connector
//! contract's. The pure functions over it run in the module; the host only
//! batches its inputs into one call.

use tinyconnectors_bus::names::methods;
use tinyconnectors_bus::vocabulary::{
    IdentityValueRequest, NormalizeIdentitiesRequest, NormalizeIdentitiesResponse,
};

use super::{ConnectedIdentity, IdentityKind};
use crate::config::Config;
use crate::integrations::composio::module_client as connectors;

async fn normalize(
    config: &Config,
    request: NormalizeIdentitiesRequest,
) -> Result<NormalizeIdentitiesResponse, String> {
    tracing::debug!(
        values = request.values.len(),
        connection_identifiers = request.connection_identifiers.len(),
        identities = request.identities.len(),
        "[composio][identity] NormalizeIdentities"
    );
    connectors::call_stateless(config, methods::NORMALIZE_IDENTITIES, request).await
}

/// Canonicalise each `(kind, raw)` pair, in order. `None` marks a blank input.
///
/// # Errors
///
/// The connector module is unreachable or rejects the request.
pub async fn canonicalize_values(
    config: &Config,
    values: &[(IdentityKind, String)],
) -> Result<Vec<Option<String>>, String> {
    if values.is_empty() {
        return Ok(Vec::new());
    }
    let request = NormalizeIdentitiesRequest {
        values: values
            .iter()
            .map(|(kind, value)| IdentityValueRequest {
                kind: *kind,
                value: value.clone(),
            })
            .collect(),
        ..Default::default()
    };
    let reply = normalize(config, request).await?;
    Ok(reply.values.into_iter().map(|v| v.value).collect())
}

/// Normalise connection identifiers (toolkit slugs, connection ids), in order.
///
/// # Errors
///
/// The connector module is unreachable or rejects the request.
pub async fn normalize_connection_identifiers(
    config: &Config,
    raw: &[String],
) -> Result<Vec<String>, String> {
    if raw.is_empty() {
        return Ok(Vec::new());
    }
    let request = NormalizeIdentitiesRequest {
        connection_identifiers: raw.to_vec(),
        ..Default::default()
    };
    Ok(normalize(config, request).await?.connection_identifiers)
}

/// The `## Connected Identities` prompt section for `identities`; empty when
/// none is renderable.
///
/// # Errors
///
/// The connector module is unreachable or rejects the request.
pub async fn render_connected_identities_section(
    config: &Config,
    identities: &[ConnectedIdentity],
) -> Result<String, String> {
    if identities.is_empty() {
        return Ok(String::new());
    }
    let request = NormalizeIdentitiesRequest {
        identities: identities.to_vec(),
        ..Default::default()
    };
    Ok(normalize(config, request).await?.prompt_section)
}

#[cfg(test)]
#[path = "identity_tests.rs"]
mod tests;
