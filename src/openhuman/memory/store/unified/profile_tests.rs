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
