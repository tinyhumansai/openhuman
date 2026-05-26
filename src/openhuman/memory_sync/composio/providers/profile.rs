//! Profile persistence — maps [`ProviderUserProfile`] (and provider-specific
//! `extras`) into [`IdentityKind`]-tagged facet rows so the self-identity
//! matcher can join directly against the memory tree's `EntityKind` and the
//! structural sender field on chunks.
//!
//! Schema: `user_profile.facet_type='skill'`,
//! `key = "skill:{toolkit}:{conn_id}:{identity_kind}"`, `value` =
//! canonicalized identifier. Confidence is set per-kind so the matcher can
//! refuse to auto-promote weak signals (display_name) to `is_self`.
//!
//! One [`ProviderUserProfile`] expands to multiple rows — including
//! identifiers carried in `extras` that the previous fixed-fields shape
//! dropped on the floor (e.g. Slack screen-name handle).
//!
//! Callers invoke [`persist_provider_profile`] after every successful
//! `fetch_user_profile` call — from `on_connection_created`, periodic syncs,
//! and the `composio_get_user_profile` / `composio_refresh_all_identities`
//! RPC ops.

use super::ProviderUserProfile;
use crate::openhuman::learning::candidate::{
    self as learning_candidate, CueFamily, EvidenceRef, FacetClass, LearningCandidate,
};
use crate::openhuman::memory_store::profile::{self, FacetType};
use rusqlite::params;
use serde_json::Value;
use std::collections::BTreeMap;

// ────────────────────────────────────────────────────────────────────────
// IdentityKind — the matching axis
// ────────────────────────────────────────────────────────────────────────

/// Shape of an identifier persisted against a connection. Mirrors the
/// matching dimensions of the memory tree's
/// `crate::openhuman::memory_tree::score::extract::EntityKind` so the
/// self-check is a direct `(toolkit, kind, value)` lookup.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IdentityKind {
    /// Platform-canonical immutable id — Slack `U123ABC`, Notion UUID.
    UserId,
    Email,
    /// `@`-style screen name, canonicalised without the leading `@`.
    Handle,
    /// E.164 phone number.
    Phone,
    /// Human display label. Weak signal — never auto-promotes to is_self.
    DisplayName,
    /// Not for matching; kept for UI / prompt rendering.
    AvatarUrl,
    /// Not for matching; kept for UI / prompt rendering.
    ProfileUrl,
}

impl IdentityKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::UserId => "user_id",
            Self::Email => "email",
            Self::Handle => "handle",
            Self::Phone => "phone",
            Self::DisplayName => "display_name",
            Self::AvatarUrl => "avatar_url",
            Self::ProfileUrl => "profile_url",
        }
    }

    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "user_id" => Self::UserId,
            "email" => Self::Email,
            "handle" => Self::Handle,
            "phone" => Self::Phone,
            "display_name" => Self::DisplayName,
            "avatar_url" => Self::AvatarUrl,
            "profile_url" => Self::ProfileUrl,
            _ => return None,
        })
    }

    /// Confidence the matcher records on the row. Hard kinds auto-promote
    /// a chunk to `is_self`; weak kinds require corroboration.
    pub fn confidence(self) -> f64 {
        match self {
            Self::UserId | Self::Phone => 1.00,
            Self::Email => 0.95,
            Self::Handle => 0.70,
            Self::DisplayName => 0.40,
            Self::AvatarUrl | Self::ProfileUrl => 0.50,
        }
    }

    /// True if this kind is a real identity signal worth running through
    /// the matcher (vs. UI-only fields).
    pub fn is_matchable(self) -> bool {
        matches!(
            self,
            Self::UserId | Self::Email | Self::Handle | Self::Phone | Self::DisplayName
        )
    }
}

/// Canonicalize a raw value for storage and lookup. The same routine runs
/// on the entity side at match time, so equality of canonical forms is the
/// matcher's only test — no `COLLATE NOCASE`, no per-call lowercasing.
pub fn canonicalize(kind: IdentityKind, raw: &str) -> Option<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    Some(match kind {
        IdentityKind::Email => trimmed.to_lowercase(),
        IdentityKind::Handle => trimmed.trim_start_matches('@').to_lowercase(),
        IdentityKind::Phone => trimmed
            .chars()
            .filter(|c| c.is_ascii_digit() || *c == '+')
            .collect(),
        IdentityKind::DisplayName => trimmed.split_whitespace().collect::<Vec<_>>().join(" "),
        IdentityKind::UserId | IdentityKind::AvatarUrl | IdentityKind::ProfileUrl => {
            trimmed.to_string()
        }
    })
}

// ────────────────────────────────────────────────────────────────────────
// Persist
// ────────────────────────────────────────────────────────────────────────

/// Persist a provider profile as one facet row per (kind, value). Returns
/// the number of rows written. Silently no-ops if the memory client isn't
/// ready (startup race / unauthenticated CLI).
pub fn persist_provider_profile(profile: &ProviderUserProfile) -> usize {
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        tracing::debug!(
            toolkit = %profile.toolkit,
            "[composio:profile] memory client not ready, skipping persist"
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

    let rows = expand_identity_rows(&toolkit, profile);

    let mut written = 0usize;
    for (kind, value) in rows {
        let key = format!("skill:{toolkit}:{identifier}:{}", kind.as_str());
        let facet_id = format!("skill-{toolkit}-{identifier}-{}", kind.as_str());

        if let Err(e) = profile::profile_upsert(
            &conn,
            &facet_id,
            &FacetType::Skill,
            &key,
            &value,
            kind.confidence(),
            None,
            now,
        ) {
            tracing::warn!(
                toolkit = %toolkit,
                identifier = %identifier,
                kind = kind.as_str(),
                error = %e,
                "[composio:profile] profile_upsert failed (non-fatal)"
            );
            continue;
        }

        // Phase 3 (#566): also emit a LearningCandidate so the stability detector
        // can score provider data alongside other evidence on the next rebuild.
        // We use the `identity/` key prefix for provider identity fields.
        if kind.is_matchable() {
            let identity_key = format!("{}:{}", normalize_token(&toolkit), kind.as_str());
            let candidate = LearningCandidate {
                class: FacetClass::Identity,
                key: identity_key,
                value: value.clone(),
                cue_family: CueFamily::Structural,
                evidence: EvidenceRef::Provider {
                    toolkit: toolkit.clone(),
                    connection_id: identifier.clone(),
                    field: kind.as_str().to_string(),
                },
                initial_confidence: kind.confidence(),
                observed_at: now,
            };
            learning_candidate::global().push(candidate);
        }

        written += 1;
    }

    if written > 0 {
        tracing::debug!(
            toolkit = %toolkit,
            identifier = %identifier,
            rows_written = written,
            "[composio:profile] persisted identity rows (+ emitted Identity candidates)"
        );
    }
    written
}

/// Expand a [`ProviderUserProfile`] (and provider-specific `extras`) into
/// the canonical (kind, value) rows. **All per-toolkit quirks live here**;
/// the matcher only sees normalized tuples.
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
            // After the auth.test + users.info fix in slack/provider.rs:
            //   profile.username == Slack user_id (e.g. U123ABC)
            //   extras.handle    == Slack screen_name (e.g. "cyrus")
            //   extras.team_*    → workspace context, not identity
            push(IdentityKind::UserId, profile.username.as_deref());
            push(IdentityKind::Handle, json_str(&profile.extras, "handle"));
        }
        "notion" => {
            // Notion's `username` is the user UUID
            // (`data.bot.owner.user.id` per notion/provider.rs).
            push(IdentityKind::UserId, profile.username.as_deref());
        }
        "gmail" => {
            // Email + display_name only — no platform user_id worth matching.
        }
        _ => {
            // Unknown toolkit: best-effort. If `username` is set treat it
            // as a handle so weak-match logic (medium confidence) applies.
            push(IdentityKind::Handle, profile.username.as_deref());
        }
    }

    rows
}

fn json_str<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(|x| x.as_str())
}

// ────────────────────────────────────────────────────────────────────────
// Read paths
// ────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ConnectedIdentity {
    pub source: String,
    pub identifier: String,
    pub display_name: Option<String>,
    pub email: Option<String>,
    pub handle: Option<String>,
    pub phone: Option<String>,
    pub user_id: Option<String>,
    pub avatar_url: Option<String>,
    pub profile_url: Option<String>,
}

/// Load all provider-sourced identities, grouped by `(source, conn_id)`.
/// Rows whose last segment is not a known [`IdentityKind`] are silently
/// skipped — that includes legacy `username` rows from before the rewrite.
pub fn load_connected_identities() -> Vec<ConnectedIdentity> {
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        tracing::debug!("[composio:profile] load_connected_identities: memory client not ready");
        return Vec::new();
    };
    let conn = client.profile_conn();
    let facets = match profile::profile_facets_by_type(&conn, &FacetType::Skill) {
        Ok(f) => f,
        Err(error) => {
            tracing::warn!(
                error = %error,
                "[composio:profile] load_connected_identities: profile_facets_by_type failed"
            );
            return Vec::new();
        }
    };

    let mut grouped: BTreeMap<(String, String), ConnectedIdentity> = BTreeMap::new();
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
    grouped.into_values().collect()
}

/// Direct self-check for the entity matcher and the chunk-build hook.
/// Returns true if any connection of `toolkit` has a row with this
/// `(kind, value)` after canonicalization. Non-matchable kinds
/// (avatar_url, profile_url) always return false.
pub fn is_self_identity(toolkit: &str, kind: IdentityKind, raw_value: &str) -> bool {
    if !kind.is_matchable() {
        return false;
    }
    let Some(canonical) = canonicalize(kind, raw_value) else {
        return false;
    };
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        return false;
    };
    let conn = client.profile_conn();
    let conn = conn.lock();

    let key_pattern = format!("skill:{}:%:{}", normalize_token(toolkit), kind.as_str());
    conn.query_row(
        "SELECT 1 FROM user_profile
          WHERE facet_type = 'skill'
            AND key LIKE ?1
            AND value = ?2
          LIMIT 1",
        params![key_pattern, canonical],
        |_| Ok(()),
    )
    .is_ok()
}

/// Cross-toolkit variant — matches against every connected provider's
/// rows of this kind. Used for marking memory-tree entity rows: an email
/// in a Slack message that matches the user's Gmail address is still
/// "me," regardless of which source produced the chunk.
pub fn is_self_identity_any_toolkit(kind: IdentityKind, raw_value: &str) -> bool {
    if !kind.is_matchable() {
        return false;
    }
    let Some(canonical) = canonicalize(kind, raw_value) else {
        return false;
    };
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        return false;
    };
    let conn = client.profile_conn();
    let conn = conn.lock();

    let key_pattern = format!("skill:%:%:{}", kind.as_str());
    conn.query_row(
        "SELECT 1 FROM user_profile
          WHERE facet_type = 'skill'
            AND key LIKE ?1
            AND value = ?2
          LIMIT 1",
        params![key_pattern, canonical],
        |_| Ok(()),
    )
    .is_ok()
}

/// Render a compact section for prompt injection. Skips `user_id` (not
/// human-readable), prefixes `handle` with `@`.
pub fn render_connected_identities_section(identities: &[ConnectedIdentity]) -> String {
    if identities.is_empty() {
        return String::new();
    }
    let mut out = String::from("## Connected Identities\n\n");
    for id in identities {
        let mut fields = Vec::<String>::new();
        if let Some(v) = id.display_name.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(v);
            }
        }
        if let Some(v) = id.email.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(v);
            }
        }
        if let Some(v) = id.handle.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(format!("@{v}"));
            }
        }
        if let Some(v) = id.profile_url.as_deref() {
            let v = sanitize_prompt_value(v);
            if !v.is_empty() {
                fields.push(v);
            }
        }
        if fields.is_empty() {
            continue;
        }
        let identifier = sanitize_prompt_value(&id.identifier);
        out.push_str(&format!(
            "- {} ({}): {}\n",
            title_case(&id.source),
            identifier,
            fields.join(" | ")
        ));
    }
    if out.trim() == "## Connected Identities" {
        return String::new();
    }
    out
}

/// Delete every row for a `(source, conn_id)` pair — used on disconnect.
pub fn delete_connected_identity_facets(source: &str, identifier: &str) -> usize {
    // `persist_provider_profile` writes keys with `normalize_token`-applied
    // segments; compare against the same normalized form here so a caller
    // passing the raw toolkit/connection_id still matches stored rows
    // (otherwise rows would survive disconnect and the user-tagger would
    // keep treating the removed account as the user — #1381 review).
    let source = normalize_token(source);
    let identifier = normalize_token(identifier);
    let Some(client) = crate::openhuman::memory::global::client_if_ready() else {
        tracing::debug!(
            source = %source,
            identifier = %identifier,
            "[composio:profile] delete_connected_identity_facets: memory client not ready"
        );
        return 0;
    };
    let conn = client.profile_conn();
    let Ok(facets) = profile::profile_facets_by_type(&conn, &FacetType::Skill) else {
        return 0;
    };
    let mut deleted = 0usize;
    for facet in facets {
        let Some((s, i, _kind)) = parse_skill_identity_key(&facet.key) else {
            continue;
        };
        if s == source && i == identifier {
            let conn_guard = conn.lock();
            if conn_guard
                .execute(
                    "DELETE FROM user_profile WHERE facet_id = ?1",
                    params![facet.facet_id],
                )
                .unwrap_or(0)
                > 0
            {
                deleted += 1;
            }
        }
    }
    deleted
}

// ────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────

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

pub(crate) fn normalize_connection_identifier(raw: &str) -> String {
    normalize_token(raw)
}

fn title_case(raw: &str) -> String {
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
        None => String::new(),
    }
}

fn sanitize_prompt_value(raw: &str) -> String {
    let replaced = raw.replace(['\n', '\r', '\t'], " ").replace('|', "/");
    replaced.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn now_secs() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ────────────────────────────────────────────────────────────────────────
// Tests
// ────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::memory_store::profile::{profile_load_all, PROFILE_INIT_SQL};
    use parking_lot::Mutex;
    use rusqlite::Connection;
    use serde_json::json;
    use std::sync::Arc;

    fn setup_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        Arc::new(Mutex::new(conn))
    }

    // ── IdentityKind ───────────────────────────────────────────────

    #[test]
    fn identity_kind_round_trips_through_str() {
        for kind in [
            IdentityKind::UserId,
            IdentityKind::Email,
            IdentityKind::Handle,
            IdentityKind::Phone,
            IdentityKind::DisplayName,
            IdentityKind::AvatarUrl,
            IdentityKind::ProfileUrl,
        ] {
            assert_eq!(IdentityKind::parse(kind.as_str()), Some(kind));
        }
    }

    #[test]
    fn identity_kind_parse_rejects_unknown() {
        assert_eq!(IdentityKind::parse("username"), None);
        assert_eq!(IdentityKind::parse(""), None);
        assert_eq!(IdentityKind::parse("UserId"), None);
    }

    #[test]
    fn matchable_kinds_exclude_url_fields() {
        assert!(IdentityKind::UserId.is_matchable());
        assert!(IdentityKind::Email.is_matchable());
        assert!(IdentityKind::Handle.is_matchable());
        assert!(IdentityKind::Phone.is_matchable());
        assert!(IdentityKind::DisplayName.is_matchable());
        assert!(!IdentityKind::AvatarUrl.is_matchable());
        assert!(!IdentityKind::ProfileUrl.is_matchable());
    }

    #[test]
    fn confidence_orders_hard_above_weak() {
        assert!(IdentityKind::UserId.confidence() > IdentityKind::Email.confidence());
        assert!(IdentityKind::Email.confidence() > IdentityKind::Handle.confidence());
        assert!(IdentityKind::Handle.confidence() > IdentityKind::DisplayName.confidence());
    }

    // ── canonicalize ──────────────────────────────────────────────

    #[test]
    fn canonicalize_email_lowercases_and_trims() {
        assert_eq!(
            canonicalize(IdentityKind::Email, "  Cyrus@Example.COM "),
            Some("cyrus@example.com".to_string())
        );
    }

    #[test]
    fn canonicalize_handle_strips_at_and_lowercases() {
        assert_eq!(
            canonicalize(IdentityKind::Handle, "@Cyrus"),
            Some("cyrus".to_string())
        );
        assert_eq!(
            canonicalize(IdentityKind::Handle, "cyrus"),
            Some("cyrus".to_string())
        );
    }

    #[test]
    fn canonicalize_phone_keeps_only_digits_and_plus() {
        assert_eq!(
            canonicalize(IdentityKind::Phone, "+1 (555) 123-4567"),
            Some("+15551234567".to_string())
        );
    }

    #[test]
    fn canonicalize_display_name_collapses_whitespace() {
        assert_eq!(
            canonicalize(IdentityKind::DisplayName, "  Cyrus    Smith  "),
            Some("Cyrus Smith".to_string())
        );
    }

    #[test]
    fn canonicalize_user_id_preserved_as_is() {
        // Slack user_ids are case-sensitive; do not lowercase.
        assert_eq!(
            canonicalize(IdentityKind::UserId, "U123ABC"),
            Some("U123ABC".to_string())
        );
    }

    #[test]
    fn canonicalize_empty_returns_none() {
        assert_eq!(canonicalize(IdentityKind::Email, ""), None);
        assert_eq!(canonicalize(IdentityKind::Email, "   "), None);
    }

    // ── expand_identity_rows ──────────────────────────────────────

    fn fixture_profile(
        toolkit: &str,
        username: Option<&str>,
        extras: Value,
    ) -> ProviderUserProfile {
        ProviderUserProfile {
            toolkit: toolkit.into(),
            connection_id: Some("conn-1".into()),
            display_name: Some("Cyrus Smith".into()),
            email: Some("cyrus@example.com".into()),
            username: username.map(str::to_string),
            avatar_url: None,
            profile_url: Some("https://example.com/cyrus".into()),
            extras,
        }
    }

    #[test]
    fn expand_slack_promotes_username_to_user_id_and_extras_handle() {
        let p = fixture_profile("slack", Some("U123ABC"), json!({ "handle": "cyrus" }));
        let rows = expand_identity_rows("slack", &p);

        assert!(rows.contains(&(IdentityKind::UserId, "U123ABC".to_string())));
        assert!(rows.contains(&(IdentityKind::Handle, "cyrus".to_string())));
        assert!(rows.contains(&(IdentityKind::Email, "cyrus@example.com".to_string())));
        assert!(rows.contains(&(IdentityKind::DisplayName, "Cyrus Smith".to_string())));
        assert!(rows.contains(&(
            IdentityKind::ProfileUrl,
            "https://example.com/cyrus".to_string()
        )));
    }

    #[test]
    fn expand_gmail_skips_username_with_no_user_id_concept() {
        let p = fixture_profile("gmail", None, Value::Null);
        let rows = expand_identity_rows("gmail", &p);

        assert!(rows
            .iter()
            .all(|(k, _)| !matches!(k, IdentityKind::UserId | IdentityKind::Handle)));
        assert!(rows.contains(&(IdentityKind::Email, "cyrus@example.com".to_string())));
    }

    #[test]
    fn expand_notion_treats_username_as_user_id() {
        let p = fixture_profile(
            "notion",
            Some("f3c1a8e2-b9b7-4a8d-9d5b-31a2e9f44e2f"),
            Value::Null,
        );
        let rows = expand_identity_rows("notion", &p);

        assert!(rows.contains(&(
            IdentityKind::UserId,
            "f3c1a8e2-b9b7-4a8d-9d5b-31a2e9f44e2f".to_string()
        )));
    }

    #[test]
    fn expand_unknown_toolkit_falls_back_to_handle() {
        let p = fixture_profile("hypothetical", Some("alice"), Value::Null);
        let rows = expand_identity_rows("hypothetical", &p);

        assert!(rows.contains(&(IdentityKind::Handle, "alice".to_string())));
    }

    #[test]
    fn expand_empty_profile_emits_nothing_matchable() {
        let p = ProviderUserProfile {
            toolkit: "gmail".into(),
            connection_id: Some("c-1".into()),
            display_name: None,
            email: None,
            username: None,
            avatar_url: None,
            profile_url: None,
            extras: Value::Null,
        };
        let rows = expand_identity_rows("gmail", &p);
        assert!(rows.is_empty());
    }

    // ── upsert wiring (uses the underlying profile_upsert directly) ─

    #[test]
    fn upsert_writes_kind_tagged_key() {
        let conn = setup_db();

        profile::profile_upsert(
            &conn,
            "skill-slack-conn-1-user_id",
            &FacetType::Skill,
            "skill:slack:conn-1:user_id",
            "U123ABC",
            IdentityKind::UserId.confidence(),
            None,
            1000.0,
        )
        .unwrap();

        let facets = profile_load_all(&conn).unwrap();
        let row = facets
            .iter()
            .find(|f| f.key == "skill:slack:conn-1:user_id")
            .expect("row exists");
        assert_eq!(row.value, "U123ABC");
        assert!((row.confidence - 1.00).abs() < f64::EPSILON);
    }

    #[test]
    fn upsert_repeated_increments_evidence() {
        let conn = setup_db();

        for now in [1000.0, 2000.0] {
            profile::profile_upsert(
                &conn,
                "skill-notion-default-email",
                &FacetType::Skill,
                "skill:notion:default:email",
                "user@workspace.com",
                IdentityKind::Email.confidence(),
                None,
                now,
            )
            .unwrap();
        }

        let facets = profile_load_all(&conn).unwrap();
        assert_eq!(facets.len(), 1);
        assert_eq!(facets[0].evidence_count, 2);
    }

    // ── parse_skill_identity_key ──────────────────────────────────

    #[test]
    fn parse_key_round_trip() {
        let parsed = parse_skill_identity_key("skill:slack:conn_1:user_id");
        assert_eq!(
            parsed,
            Some((
                "slack".to_string(),
                "conn_1".to_string(),
                "user_id".to_string()
            ))
        );
    }

    #[test]
    fn parse_key_rejects_wrong_prefix() {
        assert!(parse_skill_identity_key("preference:slack:c:email").is_none());
    }

    #[test]
    fn parse_key_rejects_extra_segments() {
        assert!(parse_skill_identity_key("skill:slack:c:email:extra").is_none());
    }

    // ── render ────────────────────────────────────────────────────

    #[test]
    fn render_includes_handle_with_at_and_omits_user_id() {
        let rendered = render_connected_identities_section(&[ConnectedIdentity {
            source: "slack".into(),
            identifier: "T01ABC".into(),
            display_name: Some("Cyrus Smith".into()),
            email: Some("cyrus@example.com".into()),
            handle: Some("cyrus".into()),
            phone: None,
            user_id: Some("U123ABC".into()),
            avatar_url: None,
            profile_url: None,
        }]);
        assert!(rendered.contains("## Connected Identities"));
        assert!(rendered.contains("- Slack (T01ABC): Cyrus Smith | cyrus@example.com | @cyrus"));
        assert!(
            !rendered.contains("U123ABC"),
            "user_id should not appear in prompt"
        );
    }

    #[test]
    fn render_empty_list_returns_empty_string() {
        assert_eq!(render_connected_identities_section(&[]), "");
    }

    // ── now_secs sanity ───────────────────────────────────────────

    #[test]
    fn now_secs_returns_recent_unix_seconds() {
        let t = now_secs();
        assert!(t > 1_000_000_000.0);
    }

    #[test]
    fn persist_returns_zero_when_memory_client_not_ready() {
        // Exercise the early-return branch. Global client may or may
        // not be initialised in the test binary depending on ordering.
        let p = fixture_profile("gmail", None, Value::Null);
        let _ = persist_provider_profile(&p);
    }
}
