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

    pub fn from_str(s: &str) -> Self {
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

        if confidence > existing_confidence {
            // Higher confidence: overwrite value + update metadata.
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
        .query_map([], |row| row_to_facet(row))?
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
        .query_map(params![facet_type.as_str()], |row| row_to_facet(row))?
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
        facet_type: FacetType::from_str(&facet_type_str),
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
mod tests {
    use super::*;

    fn setup_db() -> Arc<Mutex<Connection>> {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        Arc::new(Mutex::new(conn))
    }

    #[test]
    fn insert_and_load_facet() {
        let conn = setup_db();
        profile_upsert(
            &conn,
            "f-1",
            &FacetType::Preference,
            "theme",
            "dark mode",
            0.8,
            Some("seg-1"),
            1000.0,
        )
        .unwrap();

        let facets = profile_load_all(&conn).unwrap();
        assert_eq!(facets.len(), 1);
        assert_eq!(facets[0].key, "theme");
        assert_eq!(facets[0].value, "dark mode");
        assert_eq!(facets[0].evidence_count, 1);
    }

    #[test]
    fn upsert_increments_evidence() {
        let conn = setup_db();
        profile_upsert(
            &conn,
            "f-1",
            &FacetType::Preference,
            "language",
            "Rust",
            0.7,
            Some("seg-1"),
            1000.0,
        )
        .unwrap();

        // Same facet_type + key, lower confidence — value should NOT change.
        profile_upsert(
            &conn,
            "f-2",
            &FacetType::Preference,
            "language",
            "Python",
            0.5,
            Some("seg-2"),
            1001.0,
        )
        .unwrap();

        let facets = profile_facets_by_type(&conn, &FacetType::Preference).unwrap();
        assert_eq!(facets.len(), 1);
        assert_eq!(facets[0].value, "Rust"); // Not overwritten.
        assert_eq!(facets[0].evidence_count, 2);

        // Higher confidence — value SHOULD change.
        profile_upsert(
            &conn,
            "f-3",
            &FacetType::Preference,
            "language",
            "Go",
            0.9,
            Some("seg-3"),
            1002.0,
        )
        .unwrap();

        let facets = profile_facets_by_type(&conn, &FacetType::Preference).unwrap();
        assert_eq!(facets[0].value, "Go");
        assert_eq!(facets[0].evidence_count, 3);
    }

    #[test]
    fn render_profile_context_formats_correctly() {
        let facets = vec![
            ProfileFacet {
                facet_id: "f-1".into(),
                facet_type: FacetType::Preference,
                key: "theme".into(),
                value: "dark mode".into(),
                confidence: 0.8,
                evidence_count: 3,
                source_segment_ids: None,
                first_seen_at: 1000.0,
                last_seen_at: 1002.0,
            },
            ProfileFacet {
                facet_id: "f-2".into(),
                facet_type: FacetType::Role,
                key: "title".into(),
                value: "backend engineer".into(),
                confidence: 0.9,
                evidence_count: 1,
                source_segment_ids: None,
                first_seen_at: 1000.0,
                last_seen_at: 1000.0,
            },
        ];

        let rendered = render_profile_context(&facets);
        assert!(rendered.contains("### Preference"));
        assert!(rendered.contains("theme: dark mode (confirmed 3x)"));
        assert!(rendered.contains("### Role"));
        assert!(rendered.contains("title: backend engineer"));
        // Single evidence should not show "(confirmed 1x)".
        assert!(!rendered.contains("(confirmed 1x)"));
    }

    #[test]
    fn empty_profile_renders_empty() {
        let rendered = render_profile_context(&[]);
        assert!(rendered.is_empty());
    }

    #[test]
    fn profile_upsert_appends_segment_ids() {
        let conn = setup_db();

        // First upsert — creates the facet with seg-1.
        profile_upsert(
            &conn,
            "f-seg-1",
            &FacetType::Preference,
            "editor",
            "neovim",
            0.7,
            Some("seg-1"),
            1000.0,
        )
        .unwrap();

        // Second upsert — same facet_type + key, different segment_id.
        profile_upsert(
            &conn,
            "f-seg-2",
            &FacetType::Preference,
            "editor",
            "neovim",
            0.5,
            Some("seg-2"),
            1001.0,
        )
        .unwrap();

        // Third upsert — again different segment_id.
        profile_upsert(
            &conn,
            "f-seg-3",
            &FacetType::Preference,
            "editor",
            "neovim",
            0.5,
            Some("seg-3"),
            1002.0,
        )
        .unwrap();

        let facets = profile_facets_by_type(&conn, &FacetType::Preference).unwrap();
        assert_eq!(
            facets.len(),
            1,
            "All upserts should resolve to a single row"
        );
        assert_eq!(facets[0].evidence_count, 3);

        let seg_ids = facets[0]
            .source_segment_ids
            .as_deref()
            .expect("source_segment_ids should be present");
        assert!(
            seg_ids.contains("seg-1"),
            "seg-1 should be in source_segment_ids"
        );
        assert!(
            seg_ids.contains("seg-2"),
            "seg-2 should be in source_segment_ids"
        );
        assert!(
            seg_ids.contains("seg-3"),
            "seg-3 should be in source_segment_ids"
        );
    }

    #[test]
    fn profile_facets_by_type_returns_empty_for_no_matches() {
        let conn = setup_db();
        // Insert a Preference facet; querying for Skill should yield nothing.
        profile_upsert(
            &conn,
            "f-pref",
            &FacetType::Preference,
            "theme",
            "dark",
            0.8,
            None,
            1000.0,
        )
        .unwrap();

        let skills = profile_facets_by_type(&conn, &FacetType::Skill).unwrap();
        assert!(
            skills.is_empty(),
            "Querying Skill type should return empty when only Preference exists"
        );
    }

    #[test]
    fn profile_multiple_types_coexist() {
        let conn = setup_db();

        profile_upsert(
            &conn,
            "f-pref",
            &FacetType::Preference,
            "theme",
            "dark mode",
            0.8,
            None,
            1000.0,
        )
        .unwrap();
        profile_upsert(
            &conn,
            "f-skill",
            &FacetType::Skill,
            "language",
            "Rust",
            0.9,
            None,
            1001.0,
        )
        .unwrap();
        profile_upsert(
            &conn,
            "f-role",
            &FacetType::Role,
            "title",
            "backend engineer",
            0.85,
            None,
            1002.0,
        )
        .unwrap();

        let all = profile_load_all(&conn).unwrap();
        assert_eq!(
            all.len(),
            3,
            "All three distinct facet types should be stored"
        );

        let types_present: Vec<String> = all
            .iter()
            .map(|f| f.facet_type.as_str().to_string())
            .collect();
        assert!(types_present.contains(&"preference".to_string()));
        assert!(types_present.contains(&"skill".to_string()));
        assert!(types_present.contains(&"role".to_string()));
    }

    #[test]
    fn render_profile_context_groups_by_type() {
        let conn = setup_db();

        profile_upsert(
            &conn,
            "f-1",
            &FacetType::Preference,
            "theme",
            "dark",
            0.8,
            None,
            1000.0,
        )
        .unwrap();
        profile_upsert(
            &conn,
            "f-2",
            &FacetType::Preference,
            "font",
            "mono",
            0.7,
            None,
            1001.0,
        )
        .unwrap();
        profile_upsert(
            &conn,
            "f-3",
            &FacetType::Role,
            "title",
            "engineer",
            0.9,
            None,
            1002.0,
        )
        .unwrap();

        let all = profile_load_all(&conn).unwrap();
        let rendered = render_profile_context(&all);

        // Each type should appear as a distinct section header.
        assert!(
            rendered.contains("### Preference"),
            "Should have a Preference section"
        );
        assert!(rendered.contains("### Role"), "Should have a Role section");

        // Both preference facets should appear under the Preference section.
        assert!(
            rendered.contains("theme: dark"),
            "theme preference should appear"
        );
        assert!(
            rendered.contains("font: mono"),
            "font preference should appear"
        );

        // Role facet should appear under the Role section.
        assert!(
            rendered.contains("title: engineer"),
            "role facet should appear"
        );

        // The two sections should be separated (not merged into one block).
        let pref_pos = rendered.find("### Preference").unwrap();
        let role_pos = rendered.find("### Role").unwrap();
        assert_ne!(
            pref_pos, role_pos,
            "Preference and Role sections should be at different positions"
        );
    }
}
