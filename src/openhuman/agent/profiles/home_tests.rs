use super::*;
use tempfile::TempDir;

fn test_profile(id: &str) -> AgentProfile {
    let mut p = crate::openhuman::agent::profiles::store::built_in_default_profile();
    p.id = id.to_string();
    p.name = id.to_string();
    p.built_in = false;
    p.is_master = false;
    p.memory_dir_suffix = None;
    p.soul_md = None;
    p.dedicated_memory = false;
    p.dedicated_workspace = false;
    p
}

#[test]
fn profile_home_and_action_workspace_paths() {
    let ws = Path::new("/tmp/ws");
    let action = Path::new("/tmp/act");
    assert_eq!(
        profile_home(ws, "alice"),
        Path::new("/tmp/ws/personalities/alice")
    );
    assert_eq!(
        profile_action_workspace(action, "alice"),
        Path::new("/tmp/act/profiles/alice")
    );
}

#[test]
fn validate_profile_id_matrix() {
    // Valid.
    let max_len = "x".repeat(64);
    for id in [
        "a",
        "a1",
        "alice",
        "alice-bob",
        "alice_bob",
        "0",
        max_len.as_str(),
    ] {
        assert!(validate_profile_id(id).is_ok(), "expected ok: {id}");
    }
    // Invalid.
    for id in [
        "",          // empty
        "-alice",    // leading dash
        "_alice",    // leading underscore
        "Alice",     // uppercase
        "alice bob", // space
        "alice.bob", // dot
        "alice/bob", // slash
        "über",      // non-ascii
    ] {
        assert!(validate_profile_id(id).is_err(), "expected err: {id}");
    }
    // Too long (65).
    assert!(validate_profile_id(&"a".repeat(65)).is_err());
}

#[test]
fn ensure_profile_home_creates_and_seeds() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let mut profile = test_profile("alice");
    profile.description = "A tidy writer.".to_string();

    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

    let home = profile_home(ws.path(), "alice");
    assert!(home.is_dir());
    let soul = std::fs::read_to_string(home.join("SOUL.md")).unwrap();
    // Default template used (no inline soul_md).
    assert!(soul.contains("alice"));
    assert!(soul.contains("A tidy writer."));
    assert!(home.join("MEMORY.md").exists());
    // No dedicated workspace requested.
    assert!(!profile_action_workspace(action.path(), "alice").exists());
}

#[test]
fn ensure_profile_home_seeds_soul_from_inline() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let mut profile = test_profile("bob");
    profile.soul_md = Some("I am Bob, terse and exact.".to_string());

    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
    let soul = std::fs::read_to_string(profile_home(ws.path(), "bob").join("SOUL.md")).unwrap();
    assert_eq!(soul, "I am Bob, terse and exact.\n");
}

#[test]
fn ensure_default_profile_without_authored_soul_preserves_root_fallback() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    std::fs::write(ws.path().join("SOUL.md"), "Established root identity").unwrap();
    let mut profile = test_profile(DEFAULT_PROFILE_ID);
    profile.built_in = true;
    profile.soul_md = None;

    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

    assert!(profile_home(ws.path(), DEFAULT_PROFILE_ID)
        .join("MEMORY.md")
        .exists());
    assert!(!profile_home(ws.path(), DEFAULT_PROFILE_ID)
        .join("SOUL.md")
        .exists());
    assert_eq!(
        super::super::paths::resolve_personality_soul(ws.path(), &profile),
        None,
        "no profile override leaves prompt construction on the root SOUL.md"
    );
}

#[test]
fn ensure_profile_home_is_idempotent_and_preserves_edits() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let profile = test_profile("carol");

    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure 1");
    let home = profile_home(ws.path(), "carol");
    // User edits both files.
    std::fs::write(home.join("SOUL.md"), "EDITED SOUL").unwrap();
    std::fs::write(home.join("MEMORY.md"), "EDITED MEMORY").unwrap();

    // Second run must not clobber.
    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure 2");
    assert_eq!(
        std::fs::read_to_string(home.join("SOUL.md")).unwrap(),
        "EDITED SOUL"
    );
    assert_eq!(
        std::fs::read_to_string(home.join("MEMORY.md")).unwrap(),
        "EDITED MEMORY"
    );
}

#[test]
fn ensure_profile_home_creates_empty_skills_dir() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let profile = test_profile("frank");

    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

    let skills = profile_skills_dir(ws.path(), "frank");
    assert!(skills.is_dir(), "profile skills dir must be created");
    // Empty — the user drops private SKILL.md bundles here.
    assert_eq!(std::fs::read_dir(&skills).unwrap().count(), 0);
}

#[test]
fn profile_skills_dir_and_root_paths() {
    let ws = Path::new("/tmp/ws");
    assert_eq!(
        profile_skills_dir(ws, "alice"),
        Path::new("/tmp/ws/personalities/alice/skills")
    );
    // Valid id → Some(root); invalid id → None (read paths never load it).
    assert_eq!(
        profile_skills_root(ws, "alice"),
        Some(Path::new("/tmp/ws/personalities/alice/skills").to_path_buf())
    );
    assert_eq!(profile_skills_root(ws, "Bad Id"), None);
}

#[test]
fn ensure_profile_home_creates_dedicated_workspace_when_opted_in() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let mut profile = test_profile("dave");
    profile.dedicated_workspace = true;

    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
    assert!(profile_action_workspace(action.path(), "dave").is_dir());
}

#[test]
fn ensure_profile_home_skips_invalid_id() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    // An id that fails validate_profile_id must not materialize a home — the
    // read paths would never load it, so a seeded dir would be dead weight.
    let mut profile = test_profile("placeholder");
    profile.id = "Bad Id".to_string();
    profile.dedicated_workspace = true;

    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");

    assert!(
        !profile_home(ws.path(), "Bad Id").exists(),
        "no home dir should be materialized for an invalid id"
    );
    assert!(
        !profile_action_workspace(action.path(), "Bad Id").exists(),
        "no dedicated workspace should be materialized for an invalid id"
    );
    // The `personalities/` root itself must not be created for it either.
    assert!(!ws.path().join("personalities").join("Bad Id").exists());
}

#[test]
fn sync_soul_md_on_upsert_overwrites_edited_inline_soul() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let mut profile = test_profile("grace");
    profile.soul_md = Some("Original identity.".to_string());
    // Seed the home once (writes SOUL.md from the original inline value).
    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
    let soul_path = profile_home(ws.path(), "grace").join("SOUL.md");
    assert_eq!(
        std::fs::read_to_string(&soul_path).unwrap(),
        "Original identity.\n"
    );

    // User edits the persona in Settings → the stored inline value changes.
    profile.soul_md = Some("Rewritten identity from Settings.".to_string());
    let rewritten =
        sync_soul_md_on_upsert(ws.path(), &profile, Some("Original identity.")).expect("sync");
    assert!(
        rewritten,
        "differing inline soul_md must overwrite the file"
    );
    assert_eq!(
        std::fs::read_to_string(&soul_path).unwrap(),
        "Rewritten identity from Settings.\n"
    );

    // Idempotent: a second sync with the same value is a no-op.
    let again = sync_soul_md_on_upsert(
        ws.path(),
        &profile,
        Some("Rewritten identity from Settings."),
    )
    .expect("sync 2");
    assert!(!again, "matching inline soul_md must not rewrite the file");
}

#[test]
fn sync_soul_md_on_upsert_leaves_file_when_inline_empty() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    // Seed with a default template (no inline soul_md).
    let mut profile = test_profile("heidi");
    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
    let soul_path = profile_home(ws.path(), "heidi").join("SOUL.md");
    // User edits the file manually; inline soul_md stays empty/None.
    std::fs::write(&soul_path, "MANUALLY EDITED SOUL").unwrap();

    profile.soul_md = None;
    let none_written = sync_soul_md_on_upsert(ws.path(), &profile, None).expect("sync none");
    assert!(!none_written);
    profile.soul_md = Some("   ".to_string()); // whitespace-only → treated as empty
    let blank_written = sync_soul_md_on_upsert(ws.path(), &profile, None).expect("sync blank");
    assert!(!blank_written);

    // The manual edit stays authoritative.
    assert_eq!(
        std::fs::read_to_string(&soul_path).unwrap(),
        "MANUALLY EDITED SOUL"
    );
}

#[test]
fn sync_soul_md_on_upsert_persists_empty_tombstone_when_inline_is_cleared() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let mut profile = test_profile("judy");
    profile.soul_md = Some("Settings identity".to_string());
    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
    let soul_path = profile_home(ws.path(), "judy").join("SOUL.md");
    assert!(soul_path.exists());

    profile.soul_md = None;
    assert!(
        sync_soul_md_on_upsert(ws.path(), &profile, Some("Settings identity"))
            .expect("clear synced soul")
    );
    assert_eq!(std::fs::read_to_string(&soul_path).unwrap(), "");

    // Selecting/materializing the profile again must not resurrect the
    // default profile template over the explicit clear.
    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure after clear");
    assert_eq!(std::fs::read_to_string(&soul_path).unwrap(), "");
}

#[test]
fn sync_soul_md_on_upsert_skips_invalid_id() {
    let ws = TempDir::new().unwrap();
    let mut profile = test_profile("placeholder");
    profile.id = "Bad Id".to_string();
    profile.soul_md = Some("ignored".to_string());
    assert!(!sync_soul_md_on_upsert(ws.path(), &profile, None).expect("sync"));
    assert!(!profile_home(ws.path(), "Bad Id").join("SOUL.md").exists());
}

#[test]
fn sync_soul_md_on_upsert_preserves_manual_file_when_inline_unchanged() {
    let ws = TempDir::new().unwrap();
    let action = TempDir::new().unwrap();
    let mut profile = test_profile("ivy");
    profile.soul_md = Some("Stored identity.".to_string());
    ensure_profile_home(ws.path(), action.path(), &profile).expect("ensure");
    let soul_path = profile_home(ws.path(), "ivy").join("SOUL.md");
    std::fs::write(&soul_path, "MANUALLY EDITED IDENTITY\n").unwrap();

    let rewritten = sync_soul_md_on_upsert(ws.path(), &profile, Some("Stored identity."))
        .expect("sync unchanged");

    assert!(!rewritten);
    assert_eq!(
        std::fs::read_to_string(soul_path).unwrap(),
        "MANUALLY EDITED IDENTITY\n"
    );
}

#[test]
fn dedicated_workspace_dir_gates_on_flag_and_id() {
    let action = Path::new("/tmp/act");
    let mut shared = test_profile("eve");
    assert_eq!(dedicated_workspace_dir(action, &shared), None);

    shared.dedicated_workspace = true;
    assert_eq!(
        dedicated_workspace_dir(action, &shared),
        Some(Path::new("/tmp/act/profiles/eve").to_path_buf())
    );

    // Legacy invalid id + dedicated_workspace → None (falls back to shared).
    let mut legacy = test_profile("Legacy Id");
    legacy.id = "Legacy Id".to_string();
    legacy.dedicated_workspace = true;
    assert_eq!(dedicated_workspace_dir(action, &legacy), None);
}
