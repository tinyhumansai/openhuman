//! Persisting and reading Composio-sourced identity facets through the bound
//! memory driver.
//!
//! Ported from the deleted `tinymemory_core::sync::composio::providers::profile`
//! (tinymemory v1.13.4 removed it along with the rest of the in-process
//! Composio pipeline — see the module's own docs on why the pipeline went and
//! what replaced it). The behaviour is unchanged: each identity field a
//! connected account reports becomes one `FacetType::Workflow` row, keyed
//! `skill:<toolkit>:<identifier>:<kind>`, written through
//! `MemoryProfile::upsert_provider_facet`.
//!
//! # What did not carry over
//!
//! The deleted engine also emitted a `LearningCandidate` for every matchable
//! field so the stability detector could score provider data alongside other
//! evidence on the next rebuild (`learning_candidate::global().push(...)`).
//! That queue is engine-internal state with no bus member — `upsert_provider_facet`
//! is a store write only — so this host cannot reproduce it. The facet itself
//! is written identically; only the downstream stability-scoring signal is
//! missing. Worth knowing if identity facets ever seem slower to stabilise
//! than they did before v1.13.4.

use crate::openhuman::config::Config;
use crate::openhuman::memory::api::provider::FacetType;
use tinymemory_api::composio::{
    canonicalize, normalize_connection_identifier, ConnectedIdentity, IdentityKind,
    ProviderUserProfile,
};

/// Persist one [`ProviderUserProfile`] as identity facets, returning how many
/// rows were written.
///
/// # Errors
///
/// Backend failures from the bound driver, or a driver that does not serve
/// `MemoryProfile`.
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

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let profile_family = binding.provider().as_profile().ok_or_else(|| {
        format!(
            "the bound memory driver '{}' does not serve Profile",
            binding.driver_id()
        )
    })?;

    let now = now_secs();
    let mut written = 0usize;
    for (kind, value) in rows {
        let key = format!("skill:{toolkit}:{identifier}:{}", kind.as_str());
        let facet_id = format!("skill-{toolkit}-{identifier}-{}", kind.as_str());
        match profile_family
            .upsert_provider_facet(
                &facet_id,
                FacetType::Workflow,
                &key,
                &value,
                kind.confidence(),
                None,
                now,
            )
            .await
        {
            Ok(()) => written += 1,
            Err(error) => {
                tracing::warn!(
                    toolkit = %toolkit,
                    identifier = %identifier,
                    kind = kind.as_str(),
                    %error,
                    "[composio:profile] profile_upsert failed (non-fatal)"
                );
            }
        }
    }

    if written > 0 {
        tracing::debug!(
            toolkit = %toolkit,
            identifier = %identifier,
            rows_written = written,
            "[composio:profile] persisted identity rows"
        );
    }
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

/// Load all provider-sourced identities, grouped by `(source, connection_id)`.
///
/// # Errors
///
/// Backend failures from the bound driver, or a driver that does not serve
/// `MemoryProfile`.
pub async fn load_connected_identities(config: &Config) -> Result<Vec<ConnectedIdentity>, String> {
    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let profile_family = binding.provider().as_profile().ok_or_else(|| {
        format!(
            "the bound memory driver '{}' does not serve Profile",
            binding.driver_id()
        )
    })?;
    let facets = profile_family
        .facets_by_type(FacetType::Workflow)
        .await
        .map_err(|error| error.to_string())?;

    let mut grouped: std::collections::BTreeMap<(String, String), ConnectedIdentity> =
        std::collections::BTreeMap::new();
    for facet in facets {
        let Some((source, identifier, kind_str)) = parse_skill_identity_key(&facet.key) else {
            continue;
        };
        let Some(kind) = IdentityKind::parse(&kind_str) else {
            continue;
        };
        let entry = grouped
            .entry((source.clone(), identifier.clone()))
            .or_insert_with(|| ConnectedIdentity {
                source,
                identifier,
                ..Default::default()
            });
        match kind {
            IdentityKind::DisplayName => entry.display_name = Some(facet.value),
            IdentityKind::Email => entry.email = Some(facet.value),
            IdentityKind::Handle => entry.handle = Some(facet.value),
            IdentityKind::Phone => entry.phone = Some(facet.value),
            IdentityKind::UserId => entry.user_id = Some(facet.value),
            IdentityKind::AvatarUrl => entry.avatar_url = Some(facet.value),
            IdentityKind::ProfileUrl => entry.profile_url = Some(facet.value),
        }
    }
    Ok(grouped.into_values().collect())
}

/// Delete every identity row for a `(source, connection_id)` pair — used on
/// disconnect. Returns how many rows were removed.
///
/// # Errors
///
/// Backend failures from the bound driver, or a driver that does not serve
/// `MemoryProfile`.
pub async fn delete_connected_identity_facets(
    config: &Config,
    source: &str,
    identifier: &str,
) -> Result<usize, String> {
    let source = normalize_connection_identifier(source);
    let identifier = normalize_connection_identifier(identifier);

    let binding = crate::openhuman::memory::binding::for_config(config)?;
    let profile_family = binding.provider().as_profile().ok_or_else(|| {
        format!(
            "the bound memory driver '{}' does not serve Profile",
            binding.driver_id()
        )
    })?;
    let facets = profile_family
        .facets_by_type(FacetType::Workflow)
        .await
        .map_err(|error| error.to_string())?;

    let mut deleted = 0usize;
    for facet in facets {
        let Some((s, i, _kind)) = parse_skill_identity_key(&facet.key) else {
            continue;
        };
        if s == source && i == identifier {
            // Same swallow as the deleted engine's own version: a disconnect
            // must not fail because one row was already gone.
            match profile_family.delete_facet_by_id(&facet.facet_id).await {
                Ok(true) => deleted += 1,
                Ok(false) => {}
                Err(error) => tracing::debug!(
                    facet_id = %facet.facet_id,
                    %error,
                    "[composio:profile] delete_connected_identity_facets: delete_facet_by_id failed (non-fatal)"
                ),
            }
        }
    }
    Ok(deleted)
}

fn parse_skill_identity_key(key: &str) -> Option<(String, String, String)> {
    let mut parts = key.split(':');
    let prefix = parts.next()?;
    let source = parts.next()?;
    let identifier = parts.next()?;
    let kind = parts.next()?;
    if prefix != "skill" || parts.next().is_some() {
        return None;
    }
    Some((source.to_string(), identifier.to_string(), kind.to_string()))
}

fn now_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}
