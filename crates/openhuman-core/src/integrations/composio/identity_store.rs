//! Persisting and reading the identities connected Composio accounts report.
//!
//! Each connected account's profile is expanded into canonical
//! `(IdentityKind, value)` rows (per-toolkit quirks live in
//! `expand_identity_rows`) and merged into one [`ConnectedIdentity`] per
//! `(toolkit, connection)`, stored in
//! `<workspace>/integrations/composio_identities.json` through
//! [`super::file_store`]. The connection picker labels, the prompt's
//! connected-identities section and the flows connection list read them back.

use crate::config::Config;
use crate::integrations::composio::contract::{
    canonicalize, normalize_connection_identifier, ConnectedIdentity, IdentityKind,
    ProviderUserProfile,
};

use super::file_store::{self, IDENTITIES_FILE};

/// Persist one [`ProviderUserProfile`], returning how many identity fields
/// were written.
///
/// Fields merge into the stored identity for the connection: a field the
/// profile does not report keeps its earlier value.
///
/// # Errors
///
/// The identities file cannot be read or written.
pub async fn persist_provider_profile(
    config: &Config,
    profile: &ProviderUserProfile,
) -> Result<usize, String> {
    let toolkit = normalize_connection_identifier(&profile.toolkit);
    let identifier = profile
        .connection_id
        .as_deref()
        .map(normalize_connection_identifier)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "default".to_string());

    let rows = expand_identity_rows(&toolkit, profile);
    if rows.is_empty() {
        return Ok(0);
    }

    let path = file_store::path(config, IDENTITIES_FILE);
    let _guard = file_store::lock().await;
    let mut identities: Vec<ConnectedIdentity> = file_store::load(&path).await?;
    let index = match identities
        .iter()
        .position(|id| id.source == toolkit && id.identifier == identifier)
    {
        Some(index) => index,
        None => {
            identities.push(ConnectedIdentity {
                source: toolkit.clone(),
                identifier: identifier.clone(),
                ..ConnectedIdentity::default()
            });
            identities.len() - 1
        }
    };
    let entry = &mut identities[index];
    let written = rows.len();
    for (kind, value) in rows {
        let slot = match kind {
            IdentityKind::DisplayName => &mut entry.display_name,
            IdentityKind::Email => &mut entry.email,
            IdentityKind::Handle => &mut entry.handle,
            IdentityKind::Phone => &mut entry.phone,
            IdentityKind::UserId => &mut entry.user_id,
            IdentityKind::AvatarUrl => &mut entry.avatar_url,
            IdentityKind::ProfileUrl => &mut entry.profile_url,
        };
        *slot = Some(value);
    }
    identities.sort_by(|a, b| (&a.source, &a.identifier).cmp(&(&b.source, &b.identifier)));
    file_store::save(&path, &identities).await?;

    tracing::debug!(
        toolkit = %toolkit,
        rows_written = written,
        "[composio:profile] persisted identity rows"
    );
    Ok(written)
}

/// Expand a [`ProviderUserProfile`] (and provider-specific `extras`) into the
/// canonical `(kind, value)` rows. **All per-toolkit quirks live here**; the
/// matcher only sees normalized tuples.
fn expand_identity_rows(
    toolkit: &str,
    profile: &ProviderUserProfile,
) -> Vec<(IdentityKind, String)> {
    let mut rows: Vec<(IdentityKind, String)> = Vec::new();
    let mut push = |kind: IdentityKind, raw: Option<&str>| {
        if let Some(v) = raw.and_then(|s| canonicalize(kind, s)) {
            rows.push((kind, v));
        }
    };

    push(IdentityKind::DisplayName, profile.display_name.as_deref());
    push(IdentityKind::Email, profile.email.as_deref());
    push(IdentityKind::AvatarUrl, profile.avatar_url.as_deref());
    push(IdentityKind::ProfileUrl, profile.profile_url.as_deref());

    match toolkit {
        "slack" => {
            // profile.username == Slack user_id (e.g. U123ABC); extras.handle
            // == Slack screen_name (e.g. "cyrus"); extras.team_* is workspace
            // context, not identity.
            push(IdentityKind::UserId, profile.username.as_deref());
            push(IdentityKind::Handle, json_str(&profile.extras, "handle"));
        }
        "notion" => {
            // Notion's `username` is the user UUID.
            push(IdentityKind::UserId, profile.username.as_deref());
        }
        "gmail" => {
            // Email + display_name only — no platform user_id worth matching.
        }
        _ => {
            // Unknown toolkit: best-effort. If `username` is set treat it as a
            // handle so weak-match logic (medium confidence) applies.
            push(IdentityKind::Handle, profile.username.as_deref());
        }
    }

    rows
}

fn json_str<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

/// Load every stored connected-account identity, ordered by
/// `(source, connection)`.
///
/// # Errors
///
/// The identities file exists but cannot be read or parsed.
pub async fn load_connected_identities(config: &Config) -> Result<Vec<ConnectedIdentity>, String> {
    file_store::load(&file_store::path(config, IDENTITIES_FILE)).await
}

/// Delete the stored identity for a `(source, connection_id)` pair — used on
/// disconnect. Returns how many identity fields were removed.
///
/// # Errors
///
/// The identities file cannot be read or written.
pub async fn delete_connected_identity_facets(
    config: &Config,
    source: &str,
    identifier: &str,
) -> Result<usize, String> {
    let source = normalize_connection_identifier(source);
    let identifier = normalize_connection_identifier(identifier);

    let path = file_store::path(config, IDENTITIES_FILE);
    let _guard = file_store::lock().await;
    let mut identities: Vec<ConnectedIdentity> = file_store::load(&path).await?;
    let mut deleted = 0usize;
    identities.retain(|id| {
        if id.source == source && id.identifier == identifier {
            deleted += field_count(id);
            false
        } else {
            true
        }
    });
    if deleted > 0 {
        file_store::save(&path, &identities).await?;
    }
    Ok(deleted)
}

/// How many identity fields an identity carries.
fn field_count(identity: &ConnectedIdentity) -> usize {
    [
        &identity.display_name,
        &identity.email,
        &identity.handle,
        &identity.phone,
        &identity.user_id,
        &identity.avatar_url,
        &identity.profile_url,
    ]
    .iter()
    .filter(|field| field.is_some())
    .count()
}

#[cfg(test)]
#[path = "identity_store_tests.rs"]
mod tests;
