//! User profile accumulation — structured, evidence-backed profile facets
//! that accumulate across sessions.
//!
//! Profile facets are extracted from conversation events (preferences,
//! facts about the user, skills, roles) and stored with confidence scores
//! and evidence counts. On conflict (same facet_type + key), evidence_count
//! is incremented; the value is only overwritten if the new confidence is
//! higher.

use parking_lot::Mutex;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::sync::Arc;

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
    UNIQUE(facet_type, key)
);

CREATE INDEX IF NOT EXISTS idx_profile_type
    ON user_profile(facet_type);
"#;

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
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Preference => "preference",
            Self::Skill => "skill",
            Self::Role => "role",
            Self::Personality => "personality",
            Self::Context => "context",
        }
    }

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

/// A single profile facet.
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
}

/// Upsert a profile facet. On conflict (same facet_type + key):
/// - Increments evidence_count
/// - Updates last_seen_at
/// - Appends segment_id to source_segment_ids
/// - Only overwrites value if new confidence > existing confidence
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
        let new_segments = match (existing_segments, segment_id) {
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
        };

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
        // Insert new facet.
        let segments = segment_id.unwrap_or("").to_string();
        conn.execute(
            "INSERT INTO user_profile
             (facet_id, facet_type, key, value, confidence, evidence_count,
              source_segment_ids, first_seen_at, last_seen_at)
             VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6, ?7, ?7)",
            params![
                facet_id,
                facet_type.as_str(),
                key,
                value,
                confidence,
                segments,
                now,
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

/// Load all profile facets.
pub fn profile_load_all(conn: &Arc<Mutex<Connection>>) -> anyhow::Result<Vec<ProfileFacet>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT facet_id, facet_type, key, value, confidence, evidence_count,
                source_segment_ids, first_seen_at, last_seen_at
         FROM user_profile
         ORDER BY facet_type, evidence_count DESC",
    )?;
    let rows = stmt
        .query_map([], row_to_facet)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

/// Load profile facets by type.
pub fn profile_facets_by_type(
    conn: &Arc<Mutex<Connection>>,
    facet_type: &FacetType,
) -> anyhow::Result<Vec<ProfileFacet>> {
    let conn = conn.lock();
    let mut stmt = conn.prepare(
        "SELECT facet_id, facet_type, key, value, confidence, evidence_count,
                source_segment_ids, first_seen_at, last_seen_at
         FROM user_profile
         WHERE facet_type = ?1
         ORDER BY evidence_count DESC",
    )?;
    let rows = stmt
        .query_map(params![facet_type.as_str()], row_to_facet)?
        .collect::<Result<Vec<_>, _>>()?;
    Ok(rows)
}

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

fn capitalize(s: &str) -> String {
    let mut chars = s.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().to_string() + chars.as_str(),
    }
}

fn row_to_facet(row: &rusqlite::Row<'_>) -> rusqlite::Result<ProfileFacet> {
    let facet_type_str: String = row.get(1)?;
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
    })
}

#[cfg(test)]
#[path = "profile_tests.rs"]
mod tests;
