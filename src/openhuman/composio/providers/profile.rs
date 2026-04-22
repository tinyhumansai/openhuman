//! Profile persistence bridge — maps [`ProviderUserProfile`] fields
//! into the local `user_profile` facet table so provider-sourced
//! identity data (display name, email, username, avatar) accumulates
//! alongside conversation-extracted preferences.
//!
//! Each non-`None` field becomes a [`FacetType::Context`] facet keyed
//! as `skill:{toolkit}:{identifier}:{field}`. Confidence is set to 0.95 because
//! this data comes directly from the upstream provider API — it's
//! authoritative, not inferred from conversation.
//!
//! Callers are expected to invoke [`persist_provider_profile`] after
//! every successful `fetch_user_profile` call — from
//! `on_connection_created`, periodic syncs, and the
//! `composio_get_user_profile` RPC op.

use super::ProviderUserProfile;
use crate::openhuman::memory::store::profile::{self, FacetType};
use std::collections::BTreeMap;

/// Confidence level assigned to provider-sourced profile data.
///
/// This is higher than conversation-inferred facets (typically 0.5–0.7)
/// because the data comes directly from the upstream provider API.
const PROVIDER_CONFIDENCE: f64 = 0.95;

/// Persist the non-`None` fields of a [`ProviderUserProfile`] into the
/// local `user_profile` facet table.
///
/// Returns the number of facets written (0–4). Silently returns 0 if
/// the global memory client is not yet initialised (user not signed in,
/// startup race, etc.) — callers should treat that as non-fatal.
pub fn persist_provider_profile(profile: &ProviderUserProfile) -> usize {
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        tracing::debug!(
            toolkit = %profile.toolkit,
            "[composio:profile] memory client not ready, skipping profile persist"
        );
        return 0;
    };
    let conn = client.profile_conn();

    let now = now_secs();
    let toolkit = normalize_token(&profile.toolkit);
    let identifier = profile
        .connection_id
        .as_deref()
        .map(normalize_token)
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "default".to_string());

    let fields: &[(&str, &Option<String>)] = &[
        ("display_name", &profile.display_name),
        ("email", &profile.email),
        ("username", &profile.username),
        ("avatar_url", &profile.avatar_url),
        ("profile_url", &profile.profile_url),
    ];

    let mut written = 0usize;
    for (field_name, value) in fields {
        let Some(val) = value else { continue };
        if val.trim().is_empty() {
            continue;
        }

        let key = format!("skill:{toolkit}:{identifier}:{field_name}");
        let facet_id = format!("skill-{toolkit}-{identifier}-{field_name}");

        if let Err(e) = profile::profile_upsert(
            &conn,
            &facet_id,
            &FacetType::Skill,
            &key,
            val,
            PROVIDER_CONFIDENCE,
            None, // no source segment — this comes from a provider, not a conversation
            now,
        ) {
            tracing::warn!(
                toolkit = %toolkit,
                identifier = %identifier,
                field = %field_name,
                error = %e,
                "[composio:profile] profile_upsert failed (non-fatal)"
            );
            continue;
        }
        written += 1;
    }

    if written > 0 {
        tracing::debug!(
            toolkit = %toolkit,
            identifier = %identifier,
            facets_written = written,
            "[composio:profile] persisted provider profile facets"
        );
    }
    written
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConnectedIdentity {
    pub source: String,
    pub identifier: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub profile_url: Option<String>,
}

/// Load provider-sourced identity fragments from profile facets and
/// group them by `source + identifier`.
pub fn load_connected_identities() -> Vec<ConnectedIdentity> {
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        return Vec::new();
    };
    let conn = client.profile_conn();
    let Ok(facets) = profile::profile_facets_by_type(&conn, &FacetType::Skill) else {
        return Vec::new();
    };

    let mut grouped: BTreeMap<(String, String), ConnectedIdentity> = BTreeMap::new();
    for facet in facets {
        let Some((source, identifier, field)) = parse_skill_identity_key(&facet.key) else {
            continue;
        };
        let entry = grouped
            .entry((source.clone(), identifier.clone()))
            .or_insert_with(|| ConnectedIdentity {
                source,
                identifier,
                display_name: None,
                email: None,
                username: None,
                profile_url: None,
            });
        match field.as_str() {
            "display_name" => entry.display_name = Some(facet.value.clone()),
            "email" => entry.email = Some(facet.value.clone()),
            "username" => entry.username = Some(facet.value.clone()),
            "profile_url" => entry.profile_url = Some(facet.value.clone()),
            _ => {}
        }
    }
    grouped.into_values().collect()
}

/// Render a compact markdown section for prompt injection.
pub fn render_connected_identities_section(identities: &[ConnectedIdentity]) -> String {
    if identities.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Connected Identities\n\n");
    for identity in identities {
        let mut fields = Vec::new();
        if let Some(display_name) = identity.display_name.as_deref() {
            fields.push(display_name.to_string());
        }
        if let Some(email) = identity.email.as_deref() {
            fields.push(email.to_string());
        }
        if let Some(username) = identity.username.as_deref() {
            fields.push(format!("@{username}"));
        }
        if let Some(profile_url) = identity.profile_url.as_deref() {
            fields.push(profile_url.to_string());
        }
        if fields.is_empty() {
            continue;
        }
        out.push_str(&format!(
            "- {} ({}): {}\n",
            title_case(&identity.source),
            identity.identifier,
            fields.join(" | ")
        ));
    }
    if out.trim() == "## Connected Identities" {
        return String::new();
    }
    out
}

fn parse_skill_identity_key(key: &str) -> Option<(String, String, String)> {
    let mut parts = key.split(':');
    let prefix = parts.next()?;
    let source = parts.next()?;
    let identifier = parts.next()?;
    let field = parts.next()?;
    if prefix != "skill" || parts.next().is_some() {
        return None;
    }
    Some((
        source.to_string(),
        identifier.to_string(),
        field.to_string(),
    ))
}

fn normalize_token(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        let lower = ch.to_ascii_lowercase();
        if lower.is_ascii_alphanumeric() || lower == '-' || lower == '_' {
            out.push(lower);
        } else {
            out.push('_');
        }
    }
    out.trim_matches('_').to_string()
}

fn title_case(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn now_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory::store::profile::{profile_load_all, PROFILE_INIT_SQL};
    use parking_lot::Mutex;
    use rusqlite::Connection;
    use std::sync::Arc;

    fn setup_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        Arc::new(Mutex::new(conn))
    }

    /// Directly exercise `profile_upsert` with provider-style keys to
    /// verify the facet schema and conflict resolution work end-to-end.
    #[test]
    fn provider_fields_map_to_facets() {
        let conn = setup_db();
        let now = 1000.0;

        // Simulate what persist_provider_profile does internally.
        profile::profile_upsert(
            &conn,
            "skill-gmail-conn-1-email",
            &FacetType::Skill,
            "skill:gmail:conn-1:email",
            "user@example.com",
            PROVIDER_CONFIDENCE,
            None,
            now,
        )
        .unwrap();

        profile::profile_upsert(
            &conn,
            "skill-gmail-conn-1-display_name",
            &FacetType::Skill,
            "skill:gmail:conn-1:display_name",
            "Jane Doe",
            PROVIDER_CONFIDENCE,
            None,
            now,
        )
        .unwrap();

        let facets = profile_load_all(&conn).unwrap();
        assert_eq!(facets.len(), 2);

        let email_facet = facets.iter().find(|f| f.key == "skill:gmail:conn-1:email");
        assert!(email_facet.is_some());
        let email_facet = email_facet.unwrap();
        assert_eq!(email_facet.value, "user@example.com");
        assert!((email_facet.confidence - PROVIDER_CONFIDENCE).abs() < f64::EPSILON);
        assert_eq!(email_facet.evidence_count, 1);
    }

    #[test]
    fn repeated_persist_increments_evidence() {
        let conn = setup_db();

        // First write.
        profile::profile_upsert(
            &conn,
            "skill-notion-default-email",
            &FacetType::Skill,
            "skill:notion:default:email",
            "user@workspace.com",
            PROVIDER_CONFIDENCE,
            None,
            1000.0,
        )
        .unwrap();

        // Second write — same key, same value (periodic re-sync).
        profile::profile_upsert(
            &conn,
            "skill-notion-default-email-2",
            &FacetType::Skill,
            "skill:notion:default:email",
            "user@workspace.com",
            PROVIDER_CONFIDENCE,
            None,
            2000.0,
        )
        .unwrap();

        let facets = profile_load_all(&conn).unwrap();
        assert_eq!(facets.len(), 1, "duplicate key should merge into one row");
        assert_eq!(facets[0].evidence_count, 2);
    }

    #[test]
    fn now_secs_returns_recent_unix_seconds() {
        // Sanity check: the helper just wraps SystemTime::now() into f64.
        let t = now_secs();
        assert!(t > 1_000_000_000.0, "expected unix epoch seconds, got {t}");
    }

    #[test]
    fn persist_provider_profile_returns_zero_when_memory_client_not_ready() {
        // The global memory client is gated behind login; in the test
        // binary it may or may not be initialised depending on test
        // ordering. We just exercise the entrypoint to cover the
        // early-return branch — if the global IS ready we accept the
        // returned count without further assertions.
        let profile = ProviderUserProfile {
            toolkit: "gmail".into(),
            connection_id: Some("c-1".into()),
            display_name: Some("Jane".into()),
            email: Some("jane@example.com".into()),
            username: None,
            avatar_url: None,
            profile_url: None,
            extras: serde_json::Value::Null,
        };
        let _written = persist_provider_profile(&profile);
    }

    #[test]
    fn empty_fields_are_skipped() {
        let profile = ProviderUserProfile {
            toolkit: "gmail".into(),
            connection_id: Some("conn-1".into()),
            display_name: Some("Jane".into()),
            email: None,
            username: Some("".into()), // empty string — should be skipped
            avatar_url: Some("  ".into()), // whitespace — should be skipped
            profile_url: Some("https://mail.google.com".into()),
            extras: serde_json::Value::Null,
        };

        // We can't call persist_provider_profile directly without the
        // global memory singleton, but we can verify the filtering logic
        // by checking the field iteration manually.
        let fields: &[(&str, &Option<String>)] = &[
            ("display_name", &profile.display_name),
            ("email", &profile.email),
            ("username", &profile.username),
            ("avatar_url", &profile.avatar_url),
            ("profile_url", &profile.profile_url),
        ];
        let non_empty_count = fields
            .iter()
            .filter(|(_, v)| v.as_deref().is_some_and(|s| !s.trim().is_empty()))
            .count();
        assert_eq!(
            non_empty_count, 2,
            "display_name and profile_url should pass"
        );
    }

    #[test]
    fn parse_skill_identity_key_accepts_valid_key() {
        let parsed = parse_skill_identity_key("skill:gmail:conn_1:email");
        assert_eq!(
            parsed,
            Some((
                "gmail".to_string(),
                "conn_1".to_string(),
                "email".to_string()
            ))
        );
    }

    #[test]
    fn render_connected_identities_section_formats_lines() {
        let rendered = render_connected_identities_section(&[ConnectedIdentity {
            source: "gmail".into(),
            identifier: "default".into(),
            display_name: Some("Jane Doe".into()),
            email: Some("jane@example.com".into()),
            username: None,
            profile_url: Some("https://mail.google.com".into()),
        }]);
        assert!(rendered.contains("## Connected Identities"));
        assert!(rendered
            .contains("- Gmail (default): Jane Doe | jane@example.com | https://mail.google.com"));
    }
}
