//! User profile accumulation — structured, evidence-backed profile facets
//! that accumulate across sessions.
//!
//! Profile facets are extracted from conversation events (preferences,
//! facts about the user, skills, roles) and stored with confidence scores
//! and evidence counts. On conflict (same facet_type + key), evidence_count
//! is incremented; the value is only overwritten if the new confidence is
//! higher.
//!
//! ## Phase 3 schema additions (#566)
//!
//! Added `state`, `stability`, `user_state`, and `evidence_refs_json` columns.
//! Existing databases are migrated idempotently via `ALTER TABLE … ADD COLUMN`
//! wrapped in `migrate_profile_schema()`.

use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

use crate::openhuman::learning::candidate::EvidenceRef;

/// SQL to create the user_profile table. Called during UnifiedMemory init.
pub const PROFILE_INIT_SQL: &str = r#"
CREATE TABLE IF NOT EXISTS user_profile (
    facet_id TEXT PRIMARY KEY,
    facet_type TEXT NOT NULL,
    key TEXT NOT NULL,
    value TEXT NOT NULL,
    confidence REAL NOT NULL DEFAULT 0.5,
    evidence_count INTEGER NOT NULL DEFAULT 1,
    source_segment_ids TEXT,
    first_seen_at REAL NOT NULL,
    last_seen_at REAL NOT NULL,
    state TEXT NOT NULL DEFAULT 'active',
    stability REAL NOT NULL DEFAULT 0.0,
    user_state TEXT NOT NULL DEFAULT 'auto',
    evidence_refs_json TEXT,
    class TEXT,
    cue_families_json TEXT,
    UNIQUE(facet_type, key)
);

CREATE INDEX IF NOT EXISTS idx_profile_type
    ON user_profile(facet_type);
"#;

/// Phase 3 ALTER TABLE statements for adding new columns to existing databases.
///
/// Used by both `migrate_profile_schema` (post-Arc-wrap path) and
/// `init.rs` (pre-Arc-wrap path) to avoid duplicating the SQL.
pub const PHASE3_COLUMNS_SQL: &[&str] = &[
    "ALTER TABLE user_profile ADD COLUMN state TEXT NOT NULL DEFAULT 'active'",
    "ALTER TABLE user_profile ADD COLUMN stability REAL NOT NULL DEFAULT 0.0",
    "ALTER TABLE user_profile ADD COLUMN user_state TEXT NOT NULL DEFAULT 'auto'",
    "ALTER TABLE user_profile ADD COLUMN evidence_refs_json TEXT",
    "ALTER TABLE user_profile ADD COLUMN class TEXT",
    "ALTER TABLE user_profile ADD COLUMN cue_families_json TEXT",
];

/// Idempotent schema migration for existing databases.
///
/// New installs get the full schema from `PROFILE_INIT_SQL`. Existing databases
/// may be missing the Phase 3 columns. This function adds each new column if it
/// doesn't exist, ignoring the "duplicate column name" error that SQLite returns
/// when the column is already present.
pub fn migrate_profile_schema(conn: &Arc<Mutex<Connection>>) {
    let conn = conn.lock();
    for sql in PHASE3_COLUMNS_SQL {
        match conn.execute(sql, []) {
            Ok(_) => {
                tracing::debug!("[profile] schema migration applied: {sql}");
            }
            Err(rusqlite::Error::SqliteFailure(err, _))
                if err.extended_code == rusqlite::ffi::SQLITE_ERROR =>
            {
                // "duplicate column name" is not a named SQLite error code; it comes
                // back as a generic SQLITE_ERROR with the text "duplicate column name".
                // We tolerate any SQLITE_ERROR here because that's the only class of
                // error this ALTER TABLE can produce when the column already exists.
                tracing::trace!("[profile] column already present (ok): {sql}");
            }
            Err(e) => {
                tracing::warn!("[profile] schema migration failed (non-fatal): {sql}: {e}");
            }
        }
    }
}

// ── FacetState ───────────────────────────────────────────────────────────────

/// Lifecycle state of a profile facet in the stability detector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum FacetState {
    /// Facet has cleared τ_promote and is included in the ambient cache.
    #[default]
    Active,
    /// Facet is between τ_provisional and τ_promote — included at lower weight.
    Provisional,
    /// Facet is between τ_evict and τ_provisional — held as a candidate.
    Candidate,
    /// Facet fell below τ_evict — will be removed on next rebuild.
    Dropped,
}

impl FacetState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Provisional => "provisional",
            Self::Candidate => "candidate",
            Self::Dropped => "dropped",
        }
    }

    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "provisional" => Self::Provisional,
            "candidate" => Self::Candidate,
            "dropped" => Self::Dropped,
            _ => Self::Active,
        }
    }
}

// ── UserState ────────────────────────────────────────────────────────────────

/// User-controlled override for a profile facet.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum UserState {
    /// No user override — stability detector manages the lifecycle.
    #[default]
    Auto,
    /// User has explicitly pinned this facet; it stays Active regardless of score.
    Pinned,
    /// User has explicitly forgotten this facet; it stays Dropped and cannot be
    /// re-promoted by new evidence.
    Forgotten,
}

impl UserState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::Pinned => "pinned",
            Self::Forgotten => "forgotten",
        }
    }

    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "pinned" => Self::Pinned,
            "forgotten" => Self::Forgotten,
            _ => Self::Auto,
        }
    }
}

// ── FacetType ────────────────────────────────────────────────────────────────

/// Profile facet types.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FacetType {
    Preference,
    Skill,
    Role,
    Personality,
    Context,
}

impl FacetType {
    /// Stable lowercase identifier persisted in the `user_profile` table.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Skill => "skill",
            Self::Role => "role",
            Self::Personality => "personality",
            Self::Context => "context",
        }
    }

    /// Parse a stored string back to a `FacetType`; unknown values fall back
    /// to `Preference`.
    pub fn parse_or_default(s: &str) -> Self {
        match s {
            "skill" => Self::Skill,
            "role" => Self::Role,
            "personality" => Self::Personality,
            "context" => Self::Context,
            _ => Self::Preference,
        }
    }
}

// ── ProfileFacet ─────────────────────────────────────────────────────────────

/// A single profile facet — extended with Phase 3 state + stability fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProfileFacet {
    pub facet_id: String,
    pub facet_type: FacetType,
    pub key: String,
    pub value: String,
    pub confidence: f64,
    pub evidence_count: i32,
    pub source_segment_ids: Option<String>,
    pub first_seen_at: f64,
    pub last_seen_at: f64,
    // ── Phase 3 additions ──
    /// Lifecycle state assigned by the stability detector.
    pub state: FacetState,
    /// Computed stability score from the last rebuild cycle.
    pub stability: f64,
    /// User-controlled override.
    pub user_state: UserState,
    /// Provenance references deserialized from `evidence_refs_json`.
    pub evidence_refs: Vec<EvidenceRef>,
    /// Facet class (style / identity / tooling / veto / goal / channel).
    ///
    /// Derived from the key prefix (e.g. `"style/verbosity"` → `"style"`) for
    /// learning-path rows. `None` for legacy provider rows whose key prefix
    /// doesn't match a known class.
    pub class: Option<String>,
    /// Per-cue-family evidence counts serialized as JSON.
    ///
    /// Shape: `{"explicit": N, "structural": N, "behavioral": N, "recurrence": N}`.
    /// `None` until the stability detector writes the first rebuild.
    pub cue_families: Option<std::collections::HashMap<String, u32>>,
}

// ── Write helpers ─────────────────────────────────────────────────────────────

/// Upsert a profile facet (legacy / provider path). On conflict (same facet_type + key):
/// - Increments evidence_count
/// - Updates last_seen_at
/// - Appends segment_id to source_segment_ids
/// - Only overwrites value if new confidence > existing confidence
///
/// The new Phase 3 columns (`state`, `stability`, `user_state`,
/// `evidence_refs_json`) default to `active`, `0.0`, `auto`, and `NULL`
/// respectively, so existing callers need no changes.
#[allow(clippy::too_many_arguments)]
pub fn profile_upsert(
    conn: &Arc<Mutex<Connection>>,
    facet_id: &str,
    facet_type: &FacetType,
    key: &str,
    value: &str,
    confidence: f64,
    segment_id: Option<&str>,
    now: f64,
) -> anyhow::Result<()> {
    let conn = conn.lock();

    // Check if this facet already exists.
    let existing: Option<(String, f64, i32, Option<String>)> = conn
        .query_row(
            "SELECT facet_id, confidence, evidence_count, source_segment_ids
             FROM user_profile WHERE facet_type = ?1 AND key = ?2",
            params![facet_type.as_str(), key],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
        )
        .ok();

    if let Some((existing_id, existing_confidence, existing_count, existing_segments)) = existing {
        let new_segments = merge_segments(existing_segments, segment_id);

        if confidence >= existing_confidence {
            // Higher or equal confidence: overwrite value + update metadata.
            conn.execute(
                "UPDATE user_profile
                 SET value = ?2, confidence = ?3, evidence_count = ?4,
                     source_segment_ids = ?5, last_seen_at = ?6
                 WHERE facet_id = ?1",
                params![
                    existing_id,
                    value,
                    confidence,
                    existing_count + 1,
                    new_segments,
                    now,
                ],
            )?;
        } else {
            // Lower confidence: keep existing value, only bump evidence.
            conn.execute(
                "UPDATE user_profile
                 SET evidence_count = ?2, source_segment_ids = ?3, last_seen_at = ?4
                 WHERE facet_id = ?1",
                params![existing_id, existing_count + 1, new_segments, now],
            )?;
        }
        tracing::debug!(
            "[profile] updated facet {}:{} (evidence_count={})",
            facet_type.as_str(),
            key,
            existing_count + 1
        );
    } else {
        // Insert new facet. Derive class from the key prefix for learning rows.
        let segments = segment_id.unwrap_or("").to_string();
        let class = infer_class_from_key(key, facet_type);
        conn.execute(
            "INSERT INTO user_profile
             (facet_id, facet_type, key, value, confidence, evidence_count,
              source_segment_ids, first_seen_at, last_seen_at,
              state, stability, user_state, evidence_refs_json,
              class, cue_families_json)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?7, 'active', 0.0, 'auto', NULL,
                     ?8, NULL)",
            params![
                facet_id,
                facet_type.as_str(),
                key,
                value,
                confidence,
                segments,
                now,
                class,
            ],
        )?;
        tracing::debug!(
            "[profile] inserted new facet {}:{} = {}",
            facet_type.as_str(),
            key,
            value
        );
    }

    Ok(())
}

/// Full upsert used by the stability detector rebuild path.
///
/// Writes all Phase 3 columns explicitly. On conflict (same facet_type + key)
/// the row is replaced in full — the rebuild owns these rows.
pub fn profile_upsert_full(
    conn: &Arc<Mutex<Connection>>,
    facet: &ProfileFacet,
) -> anyhow::Result<()> {
    let evidence_refs_json = if facet.evidence_refs.is_empty() {
        None
    } else {
        Some(serde_json::to_string(&facet.evidence_refs)?)
    };

    let cue_families_json = facet
        .cue_families
        .as_ref()
        .filter(|m| !m.is_empty())
        .map(|m| serde_json::to_string(m))
        .transpose()?;

    // Derive class from the facet's own class field or fall back to key prefix.
    let class = facet
        .class
        .clone()
        .or_else(|| infer_class_from_key(&facet.key, &facet.facet_type));

    let conn = conn.lock();

    // Use INSERT OR REPLACE to atomically update all columns including
    // state/stability without reading the row first. Note: on a UNIQUE(facet_type,
    // key) conflict, SQLite performs DELETE + INSERT rather than an in-place
    // update, which means facet_id will change for conflicting rows. This is
    // intentional: the stability detector owns these rows during rebuild and
    // provides consistent facet_id values; external references by facet_id are
    // not expected.
    conn.execute(
        "INSERT OR REPLACE INTO user_profile
         (facet_id, facet_type, key, value, confidence, evidence_count,
          source_segment_ids, first_seen_at, last_seen_at,
          state, stability, user_state, evidence_refs_json,
          class, cue_families_json)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                 ?14, ?15)",
        params![
            facet.facet_id,
            facet.facet_type.as_str(),
            facet.key,
            facet.value,
            facet.confidence,
            facet.evidence_count,
            facet.source_segment_ids,
            facet.first_seen_at,
            facet.last_seen_at,
            facet.state.as_str(),
            facet.stability,
            facet.user_state.as_str(),
            evidence_refs_json,
            class,
            cue_families_json,
        ],
    )?;

    tracing::debug!(
        "[profile] full-upsert facet {}:{} = {} (state={}, stability={:.3}, class={:?})",
        facet.facet_type.as_str(),
        facet.key,
        facet.value,
        facet.state.as_str(),
        facet.stability,
        class,
    );
    Ok(())
}

/// Update the `user_state` column for a facet by key.
///
/// Returns `Ok(true)` if a row was updated, `Ok(false)` if not found.
pub fn profile_set_user_state(
    conn: &Arc<Mutex<Connection>>,
    key: &str,
    user_state: UserState,
) -> anyhow::Result<bool> {
    let conn = conn.lock();
    let rows = conn.execute(
        "UPDATE user_profile SET user_state = ?1 WHERE key = ?2",
        params![user_state.as_str(), key],
    )?;
    Ok(rows > 0)
}

/// Delete a facet by key. Returns `true` if a row was deleted.
pub fn profile_delete_by_key(conn: &Arc<Mutex<Connection>>, key: &str) -> anyhow::Result<bool> {
    let conn = conn.lock();
    let rows = conn.execute("DELETE FROM user_profile WHERE key = ?1", params![key])?;
    Ok(rows > 0)
}

/// Delete all facets whose stability is below the given threshold.
///
/// Facets with `user_state = 'pinned'` are never deleted regardless of score.
/// Returns the number of rows deleted.
pub fn profile_delete_below_threshold(
    conn: &Arc<Mutex<Connection>>,
    threshold: f64,
) -> anyhow::Result<usize> {
    let conn = conn.lock();
    let rows = conn.execute(
        "DELETE FROM user_profile
         WHERE stability < ?1
           AND user_state != 'pinned'
           AND state = 'dropped'",
        params![threshold],
    )?;
    Ok(rows)
}

// ── Read helpers ──────────────────────────────────────────────────────────────

/// Load all profile facets.
pub fn profile_load_all(conn: &Arc<Mutex<Connection>>) -> anyhow::Result<Vec<ProfileFacet>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT facet_id, facet_type, key, value, confidence, evidence_count,
                source_segment_ids, first_seen_at, last_seen_at,
                state, stability, user_state, evidence_refs_json,
                class, cue_families_json
         FROM user_profile
         ORDER BY facet_type, evidence_count DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_facet)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Load all facets with `state = 'active'` ordered by stability descending.
pub fn profile_select_active(conn: &Arc<Mutex<Connection>>) -> anyhow::Result<Vec<ProfileFacet>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT facet_id, facet_type, key, value, confidence, evidence_count,
                source_segment_ids, first_seen_at, last_seen_at,
                state, stability, user_state, evidence_refs_json,
                class, cue_families_json
         FROM user_profile
         WHERE state = 'active'
         ORDER BY stability DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_facet)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Load all facets regardless of state (used by the rebuild cycle for a full view).
pub fn profile_select_all(conn: &Arc<Mutex<Connection>>) -> anyhow::Result<Vec<ProfileFacet>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT facet_id, facet_type, key, value, confidence, evidence_count,
                source_segment_ids, first_seen_at, last_seen_at,
                state, stability, user_state, evidence_refs_json,
                class, cue_families_json
         FROM user_profile
         ORDER BY stability DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_facet)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Load profile facets by type (legacy path).
pub fn profile_facets_by_type(
    conn: &Arc<Mutex<Connection>>,
    facet_type: &FacetType,
) -> anyhow::Result<Vec<ProfileFacet>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT facet_id, facet_type, key, value, confidence, evidence_count,
                source_segment_ids, first_seen_at, last_seen_at,
                state, stability, user_state, evidence_refs_json,
                class, cue_families_json
         FROM user_profile
         WHERE facet_type = ?1
         ORDER BY evidence_count DESC",
    )?;
    let rows = stmt
        .query_map(params![facet_type.as_str()], row_to_facet)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Load a single facet by key. Returns `None` if not found.
pub fn profile_get_by_key(
    conn: &Arc<Mutex<Connection>>,
    key: &str,
) -> anyhow::Result<Option<ProfileFacet>> {
    let conn = conn.lock();
    conn.query_row(
        "SELECT facet_id, facet_type, key, value, confidence, evidence_count,
                source_segment_ids, first_seen_at, last_seen_at,
                state, stability, user_state, evidence_refs_json,
                class, cue_families_json
         FROM user_profile WHERE key = ?1",
        params![key],
        row_to_facet,
    )
    .optional()
    .map_err(Into::into)
}

/// Count facets grouped by class prefix (the portion of `key` before the first `/`).
///
/// For example, `style/verbosity` → class `"style"`.
/// Facets whose key contains no `/` are grouped under `"_other"`.
pub fn profile_count_by_class(
    conn: &Arc<Mutex<Connection>>,
) -> anyhow::Result<HashMap<String, usize>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare("SELECT key FROM user_profile WHERE state = 'active'")?;
    let keys: Vec<String> = stmt
        .query_map([], |row| row.get(0))?
        .collect::<Result<Vec<_>, _>>()?;

    let mut counts: HashMap<String, usize> = HashMap::new();
    for key in keys {
        let class = key
            .split_once('/')
            .map(|(prefix, _)| prefix.to_string())
            .unwrap_or_else(|| "_other".to_string());
        *counts.entry(class).or_insert(0) += 1;
    }
    Ok(counts)
}

// ── Rendering ─────────────────────────────────────────────────────────────────

/// Render profile facets as a markdown section for context assembly.
pub fn render_profile_context(facets: &[ProfileFacet]) -> String {
    if facets.is_empty() {
        return String::new();
    }

    let mut sections: std::collections::BTreeMap<String, Vec<String>> =
        std::collections::BTreeMap::new();

    for facet in facets {
        let section = facet.facet_type.as_str().to_string();
        let evidence = if facet.evidence_count > 1 {
            format!(" (confirmed {}x)", facet.evidence_count)
        } else {
            String::new()
        };
        sections
            .entry(section)
            .or_default()
            .push(format!("- {}: {}{}", facet.key, facet.value, evidence));
    }

    let mut parts = Vec::new();
    for (section, items) in &sections {
        parts.push(format!("### {}\n{}", capitalize(section), items.join("\n")));
    }

    parts.join("\n\n")
}

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Infer the class label for a facet row from its key prefix and facet_type.
///
/// Learning-path rows use a key like `"style/verbosity"` where the prefix
/// directly encodes the class. Legacy provider rows use `"skill:..."` keys
/// and are mapped via `facet_type`.
fn infer_class_from_key(key: &str, facet_type: &FacetType) -> Option<String> {
    // Try key prefix first (learning path: "style/verbosity" → "style").
    if let Some((prefix, _)) = key.split_once('/') {
        let known = matches!(
            prefix,
            "style" | "identity" | "tooling" | "veto" | "goal" | "channel"
        );
        if known {
            return Some(prefix.to_string());
        }
    }
    // Legacy provider rows: skill:* keys → "tooling".
    if key.starts_with("skill:") {
        return Some("tooling".to_string());
    }
    // Fall back on facet_type.
    Some(
        match facet_type {
            FacetType::Role | FacetType::Personality => "identity",
            FacetType::Skill => "tooling",
            FacetType::Preference => "style",
            FacetType::Context => "identity",
        }
        .to_string(),
    )
}

fn merge_segments(existing: Option<String>, new_sid: Option<&str>) -> String {
    match (existing, new_sid) {
        (Some(existing), Some(sid)) => {
            if existing.contains(sid) {
                existing
            } else {
                format!("{existing},{sid}")
            }
        }
        (Some(existing), None) => existing,
        (None, Some(sid)) => sid.to_string(),
        (None, None) => String::new(),
    }
}

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
    }
}

fn row_to_facet(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProfileFacet> {
    let facet_type_str: String = row.get(1)?;
    let state_str: String = row.get(9)?;
    let stability: f64 = row.get(10)?;
    let user_state_str: String = row.get(11)?;
    let evidence_refs_json: Option<String> = row.get(12)?;
    let class: Option<String> = row.get(13)?;
    let cue_families_json: Option<String> = row.get(14)?;

    let evidence_refs = evidence_refs_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok())
        .unwrap_or_default();

    let cue_families = cue_families_json
        .as_deref()
        .and_then(|json| serde_json::from_str(json).ok());

    Ok(ProfileFacet {
        facet_id: row.get(0)?,
        facet_type: FacetType::parse_or_default(&facet_type_str),
        key: row.get(2)?,
        value: row.get(3)?,
        confidence: row.get(4)?,
        evidence_count: row.get(5)?,
        source_segment_ids: row.get(6)?,
        first_seen_at: row.get(7)?,
        last_seen_at: row.get(8)?,
        state: FacetState::parse_or_default(&state_str),
        stability,
        user_state: UserState::parse_or_default(&user_state_str),
        evidence_refs,
        class,
        cue_families,
    })
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
