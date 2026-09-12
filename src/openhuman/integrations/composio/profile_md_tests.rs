use std::fs;

use tempfile::TempDir;

use super::*;

fn sample(toolkit: &str, conn: &str) -> ProviderUserProfile {
    ProviderUserProfile {
        toolkit: toolkit.into(),
        connection_id: Some(conn.into()),
        display_name: Some("Jane Doe".into()),
        email: Some("jane@example.com".into()),
        username: Some("janedoe".into()),
        avatar_url: None,
        profile_url: Some("https://example.com/jane".into()),
        extras: serde_json::Value::Null,
    }
}

#[test]
fn creates_file_when_missing() {
    let tmp = TempDir::new().unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(body.starts_with("# User Profile"), "body was:\n{body}");
    let start = block_start(CA_BLOCK);
    let end = block_end(CA_BLOCK);
    assert!(body.contains(&start));
    assert!(body.contains(CA_HEADING));
    assert!(body.contains("**Gmail** (c-1):"));
    assert!(body.contains("jane@example.com"));
    assert!(body.contains("@janedoe"));
    assert!(body.contains(&end));
}

#[test]
fn upsert_is_idempotent_for_same_toolkit_connection() {
    let tmp = TempDir::new().unwrap();
    let mut p = sample("gmail", "c-1");
    merge_provider_into_profile_md(tmp.path(), &p).unwrap();
    p.display_name = Some("Jane D.".into());
    merge_provider_into_profile_md(tmp.path(), &p).unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    let occurrences = body.matches("acct:gmail:c-1").count();
    assert_eq!(occurrences, 1, "duplicate bullet:\n{body}");
    assert!(body.contains("Jane D."));
    assert!(!body.contains("Jane Doe"));
}

#[test]
fn multiple_toolkits_render_separate_bullets() {
    let tmp = TempDir::new().unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("twitter", "c-2")).unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(body.contains("acct:gmail:c-1"));
    assert!(body.contains("acct:twitter:c-2"));
    let start = block_start(CA_BLOCK);
    let end = block_end(CA_BLOCK);
    assert_eq!(body.matches(&start).count(), 1);
    assert_eq!(body.matches(&end).count(), 1);
}

#[test]
fn preserves_user_authored_content_outside_block() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("PROFILE.md");
    fs::write(
        &path,
        "# User Profile\n\nSome bio paragraph from LinkedIn.\n\n## Key facts\n- a\n- b\n",
    )
    .unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
    let body = fs::read_to_string(&path).unwrap();
    assert!(body.contains("Some bio paragraph from LinkedIn."));
    assert!(body.contains("## Key facts"));
    assert!(body.contains("- a"));
    assert!(body.contains("acct:gmail:c-1"));
}

#[test]
fn skips_when_no_useful_fields() {
    let tmp = TempDir::new().unwrap();
    let p = ProviderUserProfile {
        toolkit: "gmail".into(),
        connection_id: Some("c-1".into()),
        display_name: Some("   ".into()),
        email: None,
        username: Some("".into()),
        avatar_url: None,
        profile_url: None,
        extras: serde_json::Value::Null,
    };
    merge_provider_into_profile_md(tmp.path(), &p).unwrap();
    assert!(!tmp.path().join("PROFILE.md").exists());
}

#[test]
fn remove_drops_specific_bullet() {
    let tmp = TempDir::new().unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("twitter", "c-2")).unwrap();
    remove_provider_from_profile_md(tmp.path(), "gmail", "c-1").unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(!body.contains("acct:gmail:c-1"));
    assert!(body.contains("acct:twitter:c-2"));
}

#[test]
fn remove_drops_block_when_empty() {
    let tmp = TempDir::new().unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
    remove_provider_from_profile_md(tmp.path(), "gmail", "c-1").unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    let start = block_start(CA_BLOCK);
    let end = block_end(CA_BLOCK);
    assert!(!body.contains(&start), "block remained:\n{body}");
    assert!(!body.contains(&end));
    assert!(body.starts_with("# User Profile"));
}

#[test]
fn remove_is_noop_when_file_missing() {
    let tmp = TempDir::new().unwrap();
    remove_provider_from_profile_md(tmp.path(), "gmail", "c-1").unwrap();
    assert!(!tmp.path().join("PROFILE.md").exists());
}

#[test]
fn skips_when_connection_id_missing() {
    let tmp = TempDir::new().unwrap();
    let p = ProviderUserProfile {
        toolkit: "gmail".into(),
        connection_id: None,
        display_name: Some("Jane".into()),
        email: Some("jane@example.com".into()),
        username: None,
        avatar_url: None,
        profile_url: None,
        extras: serde_json::Value::Null,
    };
    merge_provider_into_profile_md(tmp.path(), &p).unwrap();
    assert!(!tmp.path().join("PROFILE.md").exists());
}

#[test]
fn preserves_indentation_and_blank_lines_around_block() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("PROFILE.md");
    let original = "# User Profile\n\n    indented bio line\n\n## Notes\n- alpha\n- beta\n\n";
    fs::write(&path, original).unwrap();
    merge_provider_into_profile_md(tmp.path(), &sample("gmail", "c-1")).unwrap();
    let body = fs::read_to_string(&path).unwrap();
    assert!(body.contains("    indented bio line"));
    assert!(body.contains("## Notes\n- alpha\n- beta"));
    let start = block_start(CA_BLOCK);
    let end = block_end(CA_BLOCK);
    assert!(body.contains(&start) && body.contains(&end));
    remove_provider_from_profile_md(tmp.path(), "gmail", "c-1").unwrap();
    let after = fs::read_to_string(&path).unwrap();
    assert!(after.contains("    indented bio line"));
    assert!(after.contains("## Notes\n- alpha\n- beta"));
    assert!(!after.contains(&start));
}

#[test]
fn sanitize_strips_pipes_and_newlines() {
    assert_eq!(sanitize("foo\nbar"), "foo bar");
    assert_eq!(sanitize("a | b"), "a / b");
    assert_eq!(sanitize("  multi   space  "), "multi space");
}

// ── replace_managed_block ─────────────────────────────────────────────────

#[test]
fn replace_managed_block_creates_file_if_missing() {
    let tmp = TempDir::new().unwrap();
    replace_managed_block(
        tmp.path(),
        "style",
        "## Style",
        "- **verbosity**: terse".into(),
    )
    .unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(body.contains("# User Profile"), "missing header:\n{body}");
    assert!(body.contains(&block_start("style")));
    assert!(body.contains("## Style"));
    assert!(body.contains("- **verbosity**: terse"));
    assert!(body.contains(&block_end("style")));
}

#[test]
fn replace_managed_block_appends_block_when_absent() {
    let tmp = TempDir::new().unwrap();
    let path = tmp.path().join("PROFILE.md");
    fs::write(&path, "# User Profile\n\nSome existing text.\n").unwrap();
    replace_managed_block(
        tmp.path(),
        "identity",
        "## Identity",
        "- **name**: Alice".into(),
    )
    .unwrap();
    let body = fs::read_to_string(&path).unwrap();
    // Existing content preserved.
    assert!(body.contains("Some existing text."));
    // New block appended.
    assert!(body.contains(&block_start("identity")));
    assert!(body.contains("## Identity"));
    assert!(body.contains("- **name**: Alice"));
}

#[test]
fn replace_managed_block_replaces_body_in_place() {
    let tmp = TempDir::new().unwrap();
    replace_managed_block(
        tmp.path(),
        "style",
        "## Style",
        "- **verbosity**: verbose".into(),
    )
    .unwrap();
    replace_managed_block(
        tmp.path(),
        "style",
        "## Style",
        "- **verbosity**: terse".into(),
    )
    .unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(body.contains("terse"));
    assert!(!body.contains("verbose"));
    // Only one start marker.
    assert_eq!(body.matches(&block_start("style")).count(), 1);
}

#[test]
fn replace_managed_block_preserves_other_blocks_and_user_text() {
    let tmp = TempDir::new().unwrap();
    // Write two blocks.
    replace_managed_block(
        tmp.path(),
        "style",
        "## Style",
        "- **verbosity**: terse".into(),
    )
    .unwrap();
    replace_managed_block(
        tmp.path(),
        "identity",
        "## Identity",
        "- **name**: Bob".into(),
    )
    .unwrap();
    // Update only style.
    replace_managed_block(
        tmp.path(),
        "style",
        "## Style",
        "- **verbosity**: verbose".into(),
    )
    .unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    // Identity block untouched.
    assert!(body.contains("- **name**: Bob"));
    // Style updated.
    assert!(body.contains("verbose"));
    assert!(!body.contains("terse"));
}

#[test]
fn replace_managed_block_empty_body_renders_placeholder() {
    let tmp = TempDir::new().unwrap();
    replace_managed_block(tmp.path(), "goals", "## Goals", String::new()).unwrap();
    let body = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(body.contains("*(no entries yet)*"));
    // Block markers still present.
    assert!(body.contains(&block_start("goals")));
    assert!(body.contains(&block_end("goals")));
}

#[test]
fn replace_managed_block_idempotent_on_repeat_invocation() {
    let tmp = TempDir::new().unwrap();
    let content = "- **verbosity**: terse".to_string();
    replace_managed_block(tmp.path(), "style", "## Style", content.clone()).unwrap();
    let body1 = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    replace_managed_block(tmp.path(), "style", "## Style", content).unwrap();
    let body2 = fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert_eq!(body1, body2, "second write should be idempotent");
}
