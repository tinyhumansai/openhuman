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
    canonicalize_values, normalize_connection_identifiers, ConnectedIdentity, IdentityKind,
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
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    profile: &ProviderUserProfile,
) -> Result<usize, String> {
    let mut keys = vec![profile.toolkit.clone()];
    keys.extend(profile.connection_id.clone());
    let mut keys = normalize_connection_identifiers(config, &keys)
        .await?
        .into_iter();
    let toolkit = keys.next().unwrap_or_default();
    let identifier = keys
        .next()
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "default".to_string());

    let rows = expand_identity_rows(config, &toolkit, profile).await?;
    if rows.is_empty() {
        return Ok(0);
    }

    let written = rows.len();
    let (key_source, key_identifier) = (toolkit.clone(), identifier.clone());
    file_store::update(
        ctx,
        config,
        IDENTITIES_FILE,
        move |identities: &mut Vec<ConnectedIdentity>| {
            let index = match identities
                .iter()
                .position(|id| id.source == key_source && id.identifier == key_identifier)
            {
                Some(index) => index,
                None => {
                    identities.push(ConnectedIdentity {
                        source: key_source.clone(),
                        identifier: key_identifier.clone(),
                        ..ConnectedIdentity::default()
                    });
                    identities.len() - 1
                }
            };
            let entry = &mut identities[index];
            for (kind, value) in rows.iter().cloned() {
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
            ((), true)
        },
    )
    .await?;

    tracing::debug!(
        toolkit = %toolkit,
        rows_written = written,
        "[composio:profile] persisted identity rows"
    );
    Ok(written)
}

/// Expand a [`ProviderUserProfile`] (and provider-specific `extras`) into the
/// canonical `(kind, value)` rows. **All per-toolkit quirks live here**; the
/// matcher only sees normalized tuples. Canonicalisation itself runs in the
/// connector module, in one call for the whole profile.
async fn expand_identity_rows(
    config: &Config,
    toolkit: &str,
    profile: &ProviderUserProfile,
) -> Result<Vec<(IdentityKind, String)>, String> {
    let mut candidates: Vec<(IdentityKind, String)> = Vec::new();
    let mut push = |kind: IdentityKind, raw: Option<&str>| {
        if let Some(s) = raw {
            candidates.push((kind, s.to_string()));
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

    let canonical = canonicalize_values(config, &candidates).await?;
    Ok(candidates
        .into_iter()
        .zip(canonical)
        .filter_map(|((kind, _), value)| value.map(|v| (kind, v)))
        .collect())
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
pub async fn load_connected_identities(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
) -> Result<Vec<ConnectedIdentity>, String> {
    file_store::load(ctx, config, IDENTITIES_FILE).await
}

/// Delete the stored identity for a `(source, connection_id)` pair — used on
/// disconnect. Returns how many identity fields were removed.
///
/// # Errors
///
/// The identities file cannot be read or written.
pub async fn delete_connected_identity_facets(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    source: &str,
    identifier: &str,
) -> Result<usize, String> {
    let mut keys =
        normalize_connection_identifiers(config, &[source.to_string(), identifier.to_string()])
            .await?
            .into_iter();
    let source = keys.next().unwrap_or_default();
    let identifier = keys.next().unwrap_or_default();

    file_store::update(
        ctx,
        config,
        IDENTITIES_FILE,
        move |identities: &mut Vec<ConnectedIdentity>| {
            let mut deleted = 0usize;
            identities.retain(|id| {
                if id.source == source && id.identifier == identifier {
                    deleted += field_count(id);
                    false
                } else {
                    true
                }
            });
            (deleted, deleted > 0)
        },
    )
    .await
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
