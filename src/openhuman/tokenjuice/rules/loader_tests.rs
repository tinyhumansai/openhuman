use super::*;

#[test]
fn builtin_rules_load_successfully() {
    let rules = load_builtin_rules();
    assert!(!rules.is_empty(), "at least one built-in rule expected");
    let ids: Vec<&str> = rules.iter().map(|r| r.rule.id.as_str()).collect();
    assert!(
        ids.contains(&"generic/fallback"),
        "generic/fallback must be present"
    );
}

#[test]
fn fallback_rule_is_last() {
    let rules = load_builtin_rules();
    let last = rules.last().expect("non-empty list");
    assert_eq!(last.rule.id, "generic/fallback");
}

#[test]
fn project_layer_overrides_builtin() {
    // Write a temporary project rules dir with a modified fallback rule
    let dir = tempfile::tempdir().expect("tempdir");
    let override_json = r#"{
        "id": "generic/fallback",
        "family": "override-family",
        "description": "overridden",
        "match": {}
    }"#;
    std::fs::write(dir.path().join("fallback.json"), override_json).unwrap();

    let opts = LoadRuleOptions {
        project_rules_dir: Some(dir.path().to_owned()),
        exclude_user: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    let fb = rules
        .iter()
        .find(|r| r.rule.id == "generic/fallback")
        .expect("fallback rule");
    assert_eq!(fb.rule.family, "override-family");
    assert_eq!(fb.source, RuleOrigin::Project);
}

#[test]
fn rules_sorted_alphabetically_fallback_last() {
    let rules = load_builtin_rules();
    let non_fb: Vec<&str> = rules
        .iter()
        .filter(|r| r.rule.id != "generic/fallback")
        .map(|r| r.rule.id.as_str())
        .collect();
    let mut sorted = non_fb.clone();
    sorted.sort();
    assert_eq!(non_fb, sorted, "rules should be alphabetically sorted");
}

// --- load_rules with disk layers ---

#[test]
fn user_layer_overrides_builtin() {
    let dir = tempfile::tempdir().expect("tempdir");
    let override_json = r#"{
        "id": "git/status",
        "family": "user-overridden",
        "description": "user override",
        "match": {}
    }"#;
    std::fs::write(dir.path().join("git_status.json"), override_json).unwrap();

    let opts = LoadRuleOptions {
        user_rules_dir: Some(dir.path().to_owned()),
        exclude_project: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    let gs = rules
        .iter()
        .find(|r| r.rule.id == "git/status")
        .expect("git/status rule");
    assert_eq!(gs.rule.family, "user-overridden");
    assert_eq!(gs.source, RuleOrigin::User);
}

#[test]
fn invalid_json_files_are_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Write an invalid JSON file
    std::fs::write(dir.path().join("bad.json"), "{ this is not valid json }").unwrap();
    // Write a valid rule
    let valid_json = r#"{
        "id": "test/valid",
        "family": "test",
        "match": {}
    }"#;
    std::fs::write(dir.path().join("valid.json"), valid_json).unwrap();

    let opts = LoadRuleOptions {
        project_rules_dir: Some(dir.path().to_owned()),
        exclude_user: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    // Valid rule should be loaded, invalid should be silently skipped
    assert!(rules.iter().any(|r| r.rule.id == "test/valid"));
}

#[test]
fn schema_and_fixture_json_files_are_skipped() {
    let dir = tempfile::tempdir().expect("tempdir");
    // These should be ignored by list_rule_files
    std::fs::write(
        dir.path().join("rules.schema.json"),
        r#"{"id":"should-skip","family":"skip","match":{}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("example.fixture.json"),
        r#"{"id":"should-skip2","family":"skip","match":{}}"#,
    )
    .unwrap();
    // A normal rule that should be loaded
    std::fs::write(
        dir.path().join("normal.json"),
        r#"{"id":"test/normal","family":"test","match":{}}"#,
    )
    .unwrap();

    let opts = LoadRuleOptions {
        project_rules_dir: Some(dir.path().to_owned()),
        exclude_user: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    // schema/fixture files should not be loaded
    assert!(!rules.iter().any(|r| r.rule.id == "should-skip"));
    assert!(!rules.iter().any(|r| r.rule.id == "should-skip2"));
    // Normal rule should be there
    assert!(rules.iter().any(|r| r.rule.id == "test/normal"));
}

#[test]
fn non_existent_dir_loads_only_builtins() {
    let opts = LoadRuleOptions {
        user_rules_dir: Some(std::path::PathBuf::from(
            "/nonexistent/path/that/does/not/exist",
        )),
        project_rules_dir: Some(std::path::PathBuf::from("/another/nonexistent/path/rules")),
        ..Default::default()
    };
    let rules = load_rules(&opts);
    // Should still have builtins
    assert!(rules.iter().any(|r| r.rule.id == "generic/fallback"));
    assert!(!rules.is_empty());
}

#[test]
fn exclude_user_skips_user_layer() {
    let user_dir = tempfile::tempdir().expect("tempdir");
    let override_json = r#"{"id":"git/status","family":"should-not-see","match":{}}"#;
    std::fs::write(user_dir.path().join("override.json"), override_json).unwrap();

    let opts = LoadRuleOptions {
        user_rules_dir: Some(user_dir.path().to_owned()),
        exclude_user: true,
        exclude_project: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    // user override should NOT be present — original builtin should remain
    let gs = rules
        .iter()
        .find(|r| r.rule.id == "git/status")
        .expect("git/status");
    assert_ne!(gs.rule.family, "should-not-see");
    assert_eq!(gs.source, RuleOrigin::Builtin);
}

#[test]
fn project_layer_wins_over_user_layer() {
    let user_dir = tempfile::tempdir().expect("tempdir");
    let project_dir = tempfile::tempdir().expect("tempdir");

    std::fs::write(
        user_dir.path().join("rule.json"),
        r#"{"id":"git/status","family":"user-family","match":{}}"#,
    )
    .unwrap();
    std::fs::write(
        project_dir.path().join("rule.json"),
        r#"{"id":"git/status","family":"project-family","match":{}}"#,
    )
    .unwrap();

    let opts = LoadRuleOptions {
        user_rules_dir: Some(user_dir.path().to_owned()),
        project_rules_dir: Some(project_dir.path().to_owned()),
        ..Default::default()
    };
    let rules = load_rules(&opts);
    let gs = rules
        .iter()
        .find(|r| r.rule.id == "git/status")
        .expect("git/status");
    // Project wins over user
    assert_eq!(gs.rule.family, "project-family");
    assert_eq!(gs.source, RuleOrigin::Project);
}

#[test]
fn subdirectory_rules_are_discovered() {
    let dir = tempfile::tempdir().expect("tempdir");
    let subdir = dir.path().join("git");
    std::fs::create_dir_all(&subdir).unwrap();
    std::fs::write(
        subdir.join("my_rule.json"),
        r#"{"id":"test/subdir-rule","family":"test","match":{}}"#,
    )
    .unwrap();

    let opts = LoadRuleOptions {
        project_rules_dir: Some(dir.path().to_owned()),
        exclude_user: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    assert!(
        rules.iter().any(|r| r.rule.id == "test/subdir-rule"),
        "subdirectory rule should be discovered"
    );
}

#[test]
fn duplicate_id_last_write_wins() {
    let dir = tempfile::tempdir().expect("tempdir");
    // Same id twice in different files — last-write (by HashMap) wins
    std::fs::write(
        dir.path().join("a_rule.json"),
        r#"{"id":"test/dup","family":"first","match":{}}"#,
    )
    .unwrap();
    std::fs::write(
        dir.path().join("b_rule.json"),
        r#"{"id":"test/dup","family":"second","match":{}}"#,
    )
    .unwrap();

    let opts = LoadRuleOptions {
        project_rules_dir: Some(dir.path().to_owned()),
        exclude_user: true,
        ..Default::default()
    };
    let rules = load_rules(&opts);
    let dups: Vec<_> = rules.iter().filter(|r| r.rule.id == "test/dup").collect();
    // There should be exactly one (deduped)
    assert_eq!(dups.len(), 1, "duplicate id should be deduplicated");
}

#[test]
fn default_user_rules_dir_is_home_based() {
    // Just exercise the path: if home doesn't exist, should still not panic
    let path = super::user_rules_root(None);
    // Should end in .config/tokenjuice/rules
    assert!(path.to_string_lossy().contains("tokenjuice"));
}

#[test]
fn default_project_rules_dir_is_cwd_based() {
    let path = super::project_rules_root(None, None);
    assert!(path.to_string_lossy().contains(".tokenjuice"));
}
