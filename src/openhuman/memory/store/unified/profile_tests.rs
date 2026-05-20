//! Tests for the `profile` module — facet upsert with confidence merging.

use super::*;

// ── Migration test ────────────────────────────────────────────────────────────

/// Verify that `migrate_profile_schema` adds Phase 3 columns to a database
/// that was created with the pre-Phase-3 schema (missing state/stability/…).
#[test]
fn migrate_adds_new_columns_to_existing_db() {
    // Create the pre-Phase-3 schema manually (only original columns).
    let pre_phase3_sql = r#"
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
    "#;
    let raw_conn = Connection::open_in_memory().unwrap();
    raw_conn.execute_batch(pre_phase3_sql).unwrap();
    let conn = Arc::new(Mutex::new(raw_conn));

    // Insert a row using the old schema.
    {
        let c = conn.lock();
        c.execute(
            "INSERT INTO user_profile
             (facet_id, facet_type, key, value, confidence, evidence_count,
              first_seen_at, last_seen_at)
             VALUES ('f-old', 'preference', 'theme', 'dark', 0.8, 1, 1000.0, 1000.0)",
            [],
        )
        .unwrap();
    }

    // Run the migration — should succeed without panicking.
    migrate_profile_schema(&conn);

    // The new columns must be present and readable.
    let facets = profile_load_all(&conn).unwrap();
    assert_eq!(facets.len(), 1);
    let f = &facets[0];
    assert_eq!(f.key, "theme");
    // Defaults applied by ALTER TABLE … DEFAULT.
    assert_eq!(f.state, FacetState::Active);
    assert!((f.stability - 0.0).abs() < f64::EPSILON);
    assert_eq!(f.user_state, UserState::Auto);
    assert!(f.evidence_refs.is_empty());
}

/// Running migrate twice is idempotent (no panic on duplicate column).
#[test]
fn migrate_is_idempotent() {
    let conn = setup_db();
    // First call — columns already exist in PROFILE_INIT_SQL.
    migrate_profile_schema(&conn);
    // Second call — must not panic.
    migrate_profile_schema(&conn);
}

// ── New column round-trip ─────────────────────────────────────────────────────

#[test]
fn profile_upsert_full_persists_phase3_fields() {
    use crate::openhuman::learning::candidate::EvidenceRef;
    let conn = setup_db();
    let facet = ProfileFacet {
        facet_id: "f-full".into(),
        facet_type: FacetType::Preference,
        key: "style/verbosity".into(),
        value: "terse".into(),
        confidence: 0.9,
        evidence_count: 3,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1200.0,
        state: FacetState::Active,
        stability: 1.8,
        user_state: UserState::Auto,
        evidence_refs: vec![EvidenceRef::Episodic { episodic_id: 42 }],
        class: Some("style".into()),
        cue_families: None,
    };
    profile_upsert_full(&conn, &facet).unwrap();

    let loaded = profile_load_all(&conn).unwrap();
    assert_eq!(loaded.len(), 1);
    let f = &loaded[0];
    assert_eq!(f.key, "style/verbosity");
    assert_eq!(f.state, FacetState::Active);
    assert!((f.stability - 1.8).abs() < 1e-9);
    assert_eq!(f.user_state, UserState::Auto);
    assert_eq!(f.evidence_refs.len(), 1);
    assert_eq!(
        f.evidence_refs[0],
        EvidenceRef::Episodic { episodic_id: 42 }
    );
}

#[test]
fn profile_select_active_filters_by_state() {
    let conn = setup_db();

    let active = ProfileFacet {
        facet_id: "f-active".into(),
        facet_type: FacetType::Preference,
        key: "style/tone".into(),
        value: "formal".into(),
        confidence: 0.85,
        evidence_count: 2,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1100.0,
        state: FacetState::Active,
        stability: 1.6,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: Some("style".into()),
        cue_families: None,
    };
    let provisional = ProfileFacet {
        facet_id: "f-prov".into(),
        facet_type: FacetType::Preference,
        key: "style/length".into(),
        value: "short".into(),
        confidence: 0.6,
        evidence_count: 1,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1000.0,
        state: FacetState::Provisional,
        stability: 0.8,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: Some("style".into()),
        cue_families: None,
    };
    profile_upsert_full(&conn, &active).unwrap();
    profile_upsert_full(&conn, &provisional).unwrap();

    let actives = profile_select_active(&conn).unwrap();
    assert_eq!(actives.len(), 1);
    assert_eq!(actives[0].key, "style/tone");
}

#[test]
fn profile_count_by_class_groups_keys() {
    let conn = setup_db();
    for (id, key) in [
        ("f1", "style/verbosity"),
        ("f2", "style/tone"),
        ("f3", "identity/name"),
        ("f4", "no_slash"),
    ] {
        let f = ProfileFacet {
            facet_id: id.into(),
            facet_type: FacetType::Preference,
            key: key.into(),
            value: "v".into(),
            confidence: 0.8,
            evidence_count: 1,
            source_segment_ids: None,
            first_seen_at: 1000.0,
            last_seen_at: 1000.0,
            state: FacetState::Active,
            stability: 1.6,
            user_state: UserState::Auto,
            evidence_refs: vec![],
            class: None,
            cue_families: None,
        };
        profile_upsert_full(&conn, &f).unwrap();
    }

    let counts = profile_count_by_class(&conn).unwrap();
    assert_eq!(counts.get("style"), Some(&2));
    assert_eq!(counts.get("identity"), Some(&1));
    assert_eq!(counts.get("_other"), Some(&1));
}

#[test]
fn profile_set_user_state_persists() {
    let conn = setup_db();
    profile_upsert(
        &conn,
        "f-us",
        &FacetType::Preference,
        "tool/editor",
        "neovim",
        0.8,
        None,
        1000.0,
    )
    .unwrap();
    let updated = profile_set_user_state(&conn, "tool/editor", UserState::Pinned).unwrap();
    assert!(updated);
    let f = profile_get_by_key(&conn, "tool/editor").unwrap().unwrap();
    assert_eq!(f.user_state, UserState::Pinned);
}

#[test]
fn profile_delete_below_threshold_removes_dropped_only() {
    let conn = setup_db();

    let dropped_low = ProfileFacet {
        facet_id: "f-drop".into(),
        facet_type: FacetType::Preference,
        key: "style/dropped".into(),
        value: "x".into(),
        confidence: 0.3,
        evidence_count: 1,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1000.0,
        state: FacetState::Dropped,
        stability: 0.1,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: Some("style".into()),
        cue_families: None,
    };
    let active_low = ProfileFacet {
        facet_id: "f-act".into(),
        facet_type: FacetType::Preference,
        key: "style/active".into(),
        value: "y".into(),
        confidence: 0.9,
        evidence_count: 5,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 1000.0,
        state: FacetState::Active,
        stability: 0.1,
        user_state: UserState::Auto,
        evidence_refs: vec![],
        class: Some("style".into()),
        cue_families: None,
    };
    profile_upsert_full(&conn, &dropped_low).unwrap();
    profile_upsert_full(&conn, &active_low).unwrap();

    let deleted = profile_delete_below_threshold(&conn, 0.3).unwrap();
    assert_eq!(deleted, 1); // Only the Dropped one.
    let all = profile_load_all(&conn).unwrap();
    assert_eq!(all.len(), 1);
    assert_eq!(all[0].key, "style/active");
}

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
            state: FacetState::Active,
            stability: 0.0,
            user_state: UserState::Auto,
            evidence_refs: vec![],
            class: None,
            cue_families: None,
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
            state: FacetState::Active,
            stability: 0.0,
            user_state: UserState::Auto,
            evidence_refs: vec![],
            class: None,
            cue_families: None,
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

#[test]
fn fresh_db_has_phase3_indexes() {
    let conn = setup_db();
    let c = conn.lock();
    let indexes: Vec<String> = c
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'user_profile'",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(
        indexes.contains(&"idx_profile_state_stability".to_string()),
        "Missing idx_profile_state_stability; found: {indexes:?}"
    );
    assert!(
        indexes.contains(&"idx_profile_key".to_string()),
        "Missing idx_profile_key; found: {indexes:?}"
    );
    assert!(
        indexes.contains(&"idx_profile_state_user_stability".to_string()),
        "Missing idx_profile_state_user_stability; found: {indexes:?}"
    );
    assert!(
        indexes.contains(&"idx_profile_type".to_string()),
        "Missing idx_profile_type; found: {indexes:?}"
    );
}

#[test]
fn phase3_indexes_applied_to_existing_db() {
    use super::super::profile::{PHASE3_COLUMNS_SQL, PHASE3_INDEXES_SQL};
    use rusqlite::Connection;

    let pre_phase3_sql = "
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
        CREATE INDEX IF NOT EXISTS idx_profile_type ON user_profile(facet_type);
    ";
    let raw_conn = Connection::open_in_memory().unwrap();
    raw_conn.execute_batch(pre_phase3_sql).unwrap();

    for sql in PHASE3_COLUMNS_SQL {
        let _ = raw_conn.execute(sql, []);
    }
    for sql in PHASE3_INDEXES_SQL {
        raw_conn.execute_batch(sql).unwrap();
    }

    let indexes: Vec<String> = raw_conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'user_profile'",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(indexes.contains(&"idx_profile_state_stability".to_string()));
    assert!(indexes.contains(&"idx_profile_key".to_string()));
    assert!(indexes.contains(&"idx_profile_state_user_stability".to_string()));
}

#[test]
fn phase3_indexes_idempotent() {
    use super::super::profile::PHASE3_INDEXES_SQL;
    let conn = setup_db();
    let c = conn.lock();
    for sql in PHASE3_INDEXES_SQL {
        c.execute_batch(sql).unwrap();
    }
    for sql in PHASE3_INDEXES_SQL {
        c.execute_batch(sql).unwrap();
    }
}

/// Verify that the real `UnifiedMemory::new` bootstrap path applies Phase 3
/// indexes when opened over a pre-Phase-3 database file (the exact scenario
/// that caused the original crash in initialization ordering).
#[test]
fn unified_memory_new_applies_phase3_indexes_to_existing_db() {
    use super::super::UnifiedMemory;
    use crate::openhuman::embeddings::NoopEmbedding;
    use rusqlite::Connection;
    use std::sync::Arc;

    let dir = tempfile::tempdir().unwrap();
    let workspace = dir.path();

    // Seed a pre-Phase-3 database at the path UnifiedMemory::new will open.
    let memory_dir = workspace.join("memory");
    std::fs::create_dir_all(&memory_dir).unwrap();
    let db_path = memory_dir.join("memory.db");
    {
        let conn = Connection::open(&db_path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS user_profile (
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
            CREATE INDEX IF NOT EXISTS idx_profile_type ON user_profile(facet_type);",
        )
        .unwrap();
    }

    // Call the real bootstrap path — must not fail on a pre-Phase-3 DB.
    let mem = UnifiedMemory::new(workspace, Arc::new(NoopEmbedding), None)
        .expect("UnifiedMemory::new must succeed on a pre-Phase-3 database");

    // The Phase 3 indexes must exist after initialization.
    let conn = mem.conn.lock();
    let indexes: Vec<String> = conn
        .prepare(
            "SELECT name FROM sqlite_master WHERE type = 'index' AND tbl_name = 'user_profile'",
        )
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();

    assert!(
        indexes.contains(&"idx_profile_state_stability".to_string()),
        "Missing idx_profile_state_stability; found: {indexes:?}"
    );
    assert!(
        indexes.contains(&"idx_profile_key".to_string()),
        "Missing idx_profile_key; found: {indexes:?}"
    );
    assert!(
        indexes.contains(&"idx_profile_state_user_stability".to_string()),
        "Missing idx_profile_state_user_stability; found: {indexes:?}"
    );
}
