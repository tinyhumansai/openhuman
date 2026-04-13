//! Profile persistence bridge — maps [`ProviderUserProfile`] fields
//! into the local `user_profile` facet table so provider-sourced
//! identity data (display name, email, username, avatar) accumulates
//! alongside conversation-extracted preferences.
//!
//! Each non-`None` field becomes a [`FacetType::Context`] facet keyed
//! as `composio:{toolkit}:{field}`. Confidence is set to 0.95 because
//! this data comes directly from the upstream provider API — it's
//! authoritative, not inferred from conversation.
//!
//! Callers are expected to invoke [`persist_provider_profile`] after
//! every successful `fetch_user_profile` call — from
//! `on_connection_created`, periodic syncs, and the
//! `composio_get_user_profile` RPC op.

use super::ProviderUserProfile;
use crate::openhuman::memory::store::profile::{self, FacetType};

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
    let toolkit = &profile.toolkit;

    let fields: &[(&str, &Option<String>)] = &[
        ("display_name", &profile.display_name),
        ("email", &profile.email),
        ("username", &profile.username),
        ("avatar_url", &profile.avatar_url),
    ];

    let mut written = 0usize;
    for (field_name, value) in fields {
        let Some(val) = value else { continue };
        if val.trim().is_empty() {
            continue;
        }

        let key = format!("composio:{toolkit}:{field_name}");
        let facet_id = format!("composio-{toolkit}-{field_name}");

        if let Err(e) = profile::profile_upsert(
            &conn,
            &facet_id,
            &FacetType::Context,
            &key,
            val,
            PROVIDER_CONFIDENCE,
            None, // no source segment — this comes from a provider, not a conversation
            now,
        ) {
            tracing::warn!(
                toolkit = %toolkit,
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
            facets_written = written,
            "[composio:profile] persisted provider profile facets"
        );
    }
    written
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
            "composio-gmail-email",
            &FacetType::Context,
            "composio:gmail:email",
            "user@example.com",
            PROVIDER_CONFIDENCE,
            None,
            now,
        )
        .unwrap();

        profile::profile_upsert(
            &conn,
            "composio-gmail-display_name",
            &FacetType::Context,
            "composio:gmail:display_name",
            "Jane Doe",
            PROVIDER_CONFIDENCE,
            None,
            now,
        )
        .unwrap();

        let facets = profile_load_all(&conn).unwrap();
        assert_eq!(facets.len(), 2);

        let email_facet = facets.iter().find(|f| f.key == "composio:gmail:email");
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
            "composio-notion-email",
            &FacetType::Context,
            "composio:notion:email",
            "user@workspace.com",
            PROVIDER_CONFIDENCE,
            None,
            1000.0,
        )
        .unwrap();

        // Second write — same key, same value (periodic re-sync).
        profile::profile_upsert(
            &conn,
            "composio-notion-email-2",
            &FacetType::Context,
            "composio:notion:email",
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
    fn empty_fields_are_skipped() {
        let profile = ProviderUserProfile {
            toolkit: "gmail".into(),
            connection_id: Some("conn-1".into()),
            display_name: Some("Jane".into()),
            email: None,
            username: Some("".into()), // empty string — should be skipped
            avatar_url: Some("  ".into()), // whitespace — should be skipped
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
        ];
        let non_empty_count = fields
            .iter()
            .filter(|(_, v)| v.as_deref().is_some_and(|s| !s.trim().is_empty()))
            .count();
        assert_eq!(non_empty_count, 1, "only display_name should pass");
    }
}
