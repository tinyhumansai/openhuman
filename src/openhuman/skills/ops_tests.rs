use super::*;

fn write(path: &Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, content).unwrap();
}

/// Workspace-only variant of [`load_skills`] used by tests that care only
/// about project-scope semantics. The production [`load_skills`] now
/// consults `dirs::home_dir()`; in unit tests that would non-deterministically
/// pick up whatever skills the developer has installed under their real
/// home. Tests exercising user-scope delegation drive a tempdir through
/// [`discover_skills`] explicitly (see `load_skills_surfaces_user_scope`).
fn load_skills_ws(workspace_dir: &Path) -> Vec<Skill> {
    let trusted = is_workspace_trusted(workspace_dir);
    discover_skills_inner(None, Some(workspace_dir), trusted)
}

#[test]
fn init_skills_dir_creates_dir_and_readme() {
    let dir = tempfile::tempdir().unwrap();
    init_skills_dir(dir.path()).unwrap();
    let skills_dir = dir.path().join("skills");
    assert!(skills_dir.is_dir());
    let readme = skills_dir.join("README.md");
    assert!(readme.exists());
}

#[test]
fn load_skills_legacy_json_still_works() {
    let dir = tempfile::tempdir().unwrap();
    init_skills_dir(dir.path()).unwrap();
    let skill_dir = dir.path().join("skills").join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    write(
        &skill_dir.join("skill.json"),
        r#"{"name":"My Skill","description":"A test","version":"1.0"}"#,
    );
    let skills = load_skills_ws(dir.path());
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "My Skill");
    assert_eq!(skills[0].description, "A test");
    assert!(skills[0].legacy);
    assert_eq!(skills[0].scope, SkillScope::Legacy);
}

#[test]
fn load_skills_parses_skill_md_frontmatter() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    // Trust marker enables project-scope loading.
    write(&ws.join(".openhuman").join("trust"), "");
    let skill_dir = ws.join(".openhuman").join("skills").join("hello-world");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: hello-world\ndescription: Say hi\nmetadata:\n  version: 0.1.0\n  tags: [demo, greeting]\n---\n\nSay hello to the user.\n",
    );
    let skills = load_skills_ws(ws);
    assert_eq!(skills.len(), 1);
    let s = &skills[0];
    assert_eq!(s.name, "hello-world");
    assert_eq!(s.description, "Say hi");
    assert_eq!(s.version, "0.1.0");
    assert_eq!(s.tags, vec!["demo", "greeting"]);
    assert_eq!(s.scope, SkillScope::Project);
    assert!(!s.legacy);
    assert!(s.warnings.is_empty(), "warnings: {:?}", s.warnings);
}

#[test]
fn deprecated_top_level_fields_load_with_migration_warning() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join("trust"), "");
    let skill_dir = ws.join(".openhuman").join("skills").join("legacy-fm");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: legacy-fm\ndescription: uses deprecated top-level fields\nversion: 0.2.0\nauthor: Jane\ntags: [old, school]\n---\n",
    );
    let skills = load_skills_ws(ws);
    assert_eq!(skills.len(), 1);
    let s = &skills[0];
    assert_eq!(s.version, "0.2.0");
    assert_eq!(s.author.as_deref(), Some("Jane"));
    assert_eq!(s.tags, vec!["old", "school"]);
    let warnings = s.warnings.join("\n");
    assert!(warnings.contains("'version' is deprecated"), "{}", warnings);
    assert!(warnings.contains("'author' is deprecated"), "{}", warnings);
    assert!(warnings.contains("'tags' is deprecated"), "{}", warnings);
}

#[test]
fn spec_compliant_fields_parse_into_metadata_map() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    write(
        &path,
        "---\nname: s\ndescription: d\nlicense: MIT\ncompatibility: \"node>=18\"\nmetadata:\n  version: 1.0.0\n  author: Alice\n  tags: [a, b]\n---\n",
    );
    let (fm, _body, _warnings) = parse_skill_md(&path).unwrap();
    assert_eq!(fm.license.as_deref(), Some("MIT"));
    assert_eq!(fm.compatibility.as_deref(), Some("node>=18"));
    assert_eq!(
        fm.metadata.get("version").and_then(|v| v.as_str()),
        Some("1.0.0")
    );
    assert_eq!(
        fm.metadata.get("author").and_then(|v| v.as_str()),
        Some("Alice")
    );
    assert!(fm.extra.is_empty(), "extras leaked: {:?}", fm.extra);
}

#[test]
fn project_skills_skipped_when_not_trusted() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    // No trust marker.
    let skill_dir = ws.join(".openhuman").join("skills").join("unsafe");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: unsafe\ndescription: should not load\n---\n",
    );
    let skills = load_skills_ws(ws);
    assert!(skills.is_empty(), "got {skills:?}");
}

#[test]
fn frontmatter_missing_name_warns_and_falls_back() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join("trust"), "");
    let skill_dir = ws.join(".openhuman").join("skills").join("mystery");
    write(
        &skill_dir.join("SKILL.md"),
        "---\ndescription: no name here\n---\n\nbody\n",
    );
    let skills = load_skills_ws(ws);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "mystery");
    assert!(skills[0]
        .warnings
        .iter()
        .any(|w| w.contains("missing 'name'")));
}

#[test]
fn frontmatter_missing_description_uses_first_body_line() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join("trust"), "");
    let skill_dir = ws.join(".openhuman").join("skills").join("s");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: s\n---\n\n# Heading\n\nActual first line.\n",
    );
    let skills = load_skills_ws(ws);
    assert_eq!(skills[0].description, "Actual first line.");
}

#[test]
fn directory_name_mismatch_warns_but_loads() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join("trust"), "");
    let skill_dir = ws.join(".openhuman").join("skills").join("dir-name");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: other-name\ndescription: mismatch\n---\n",
    );
    let skills = load_skills_ws(ws);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "other-name");
    assert!(skills[0]
        .warnings
        .iter()
        .any(|w| w.contains("does not match directory")));
}

#[test]
fn project_scope_shadows_user_scope_on_collision() {
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();
    write(&ws_dir.path().join(".openhuman").join("trust"), "");

    let user_skill = user_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("greet");
    write(
        &user_skill.join("SKILL.md"),
        "---\nname: greet\ndescription: USER COPY\n---\n",
    );

    let proj_skill = ws_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("greet");
    write(
        &proj_skill.join("SKILL.md"),
        "---\nname: greet\ndescription: PROJECT COPY\n---\n",
    );

    let skills = discover_skills(Some(user_dir.path()), Some(ws_dir.path()), true);
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].description, "PROJECT COPY");
    assert!(skills[0].warnings.iter().any(|w| w.contains("shadowed")));
}

#[test]
fn inventory_resources_lists_scripts_and_assets() {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("s");
    write(
        &skill.join("SKILL.md"),
        "---\nname: s\ndescription: d\n---\n",
    );
    write(&skill.join("scripts").join("run.sh"), "echo hi");
    write(&skill.join("references").join("notes.md"), "notes");
    write(&skill.join("assets").join("logo.png"), "");
    write(&skill.join("unrelated").join("x.txt"), "ignored");

    let mut res = inventory_resources(&skill);
    res.sort();
    assert_eq!(res.len(), 3);
    assert!(res.iter().any(|p| p.ends_with("run.sh")));
    assert!(res.iter().any(|p| p.ends_with("notes.md")));
    assert!(res.iter().any(|p| p.ends_with("logo.png")));
    assert!(!res.iter().any(|p| p.ends_with("x.txt")));
}

#[test]
fn parse_skill_md_without_frontmatter_returns_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    write(&path, "just a markdown body\n");
    let (fm, body, _warnings) = parse_skill_md(&path).unwrap();
    assert!(fm.name.is_empty());
    assert!(body.contains("markdown body"));
}

#[test]
fn parse_skill_md_unterminated_frontmatter_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    write(&path, "---\nname: bad\n\nbody without closing marker\n");
    assert!(parse_skill_md(&path).is_none());
}

#[cfg(unix)]
#[test]
fn symlinked_skill_dirs_are_skipped() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join("trust"), "");

    // A real out-of-tree skill that would load fine if linked.
    let external = tempfile::tempdir().unwrap();
    let external_skill = external.path().join("evil");
    write(
        &external_skill.join("SKILL.md"),
        "---\nname: evil\ndescription: should not load via symlink\n---\n",
    );

    // Symlink <ws>/.openhuman/skills/evil -> external/evil
    let skills_root = ws.join(".openhuman").join("skills");
    std::fs::create_dir_all(&skills_root).unwrap();
    symlink(&external_skill, skills_root.join("evil")).unwrap();

    let skills = load_skills_ws(ws);
    assert!(
        skills.is_empty(),
        "symlinked skill dir should be skipped, got: {skills:?}"
    );
}

#[cfg(unix)]
#[test]
fn symlinked_resource_roots_are_rejected() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("s");
    write(
        &skill.join("SKILL.md"),
        "---\nname: s\ndescription: d\n---\n",
    );

    // External directory that must not be inventoried.
    let external = tempfile::tempdir().unwrap();
    write(&external.path().join("leaked.txt"), "should not appear");

    // Symlink <skill>/assets -> external
    std::fs::create_dir_all(&skill).unwrap();
    symlink(external.path(), skill.join("assets")).unwrap();

    let res = inventory_resources(&skill);
    assert!(
        res.is_empty(),
        "symlinked resource root must be rejected, got: {res:?}"
    );
}

#[test]
fn load_skills_surfaces_user_scope() {
    // load_skills now delegates to discover_skills with dirs::home_dir(),
    // so user-scope skills reach production callers that still hit the
    // backwards-compat shim. Simulate this with an explicit tempdir home
    // via discover_skills — we can't safely override the process HOME in
    // unit tests.
    let user_dir = tempfile::tempdir().unwrap();
    let ws_dir = tempfile::tempdir().unwrap();

    let user_skill = user_dir
        .path()
        .join(".openhuman")
        .join("skills")
        .join("user-only");
    write(
        &user_skill.join("SKILL.md"),
        "---\nname: user-only\ndescription: from user home\n---\n",
    );

    let skills = discover_skills(
        Some(user_dir.path()),
        Some(ws_dir.path()),
        is_workspace_trusted(ws_dir.path()),
    );
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "user-only");
    assert_eq!(skills[0].scope, SkillScope::User);
}

#[test]
fn hidden_dirs_are_skipped() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join("trust"), "");
    let hidden = ws.join(".openhuman").join("skills").join(".hidden");
    write(
        &hidden.join("SKILL.md"),
        "---\nname: hidden\ndescription: nope\n---\n",
    );
    let skills = load_skills_ws(ws);
    assert!(skills.is_empty());
}

// -- read_skill_resource -------------------------------------------------
//
// These tests exercise the resource-read path via legacy-scope skills
// (`<ws>/skills/<name>/`) because that scope doesn't require the trust
// marker, is fully workspace-scoped, and avoids touching the user's home
// directory. The guarantees tested here apply equally to user- and
// project-scope skills since they all flow through the same
// `canonicalize` + `symlink_metadata` + size check gauntlet.

fn make_legacy_skill(ws: &Path, name: &str) -> PathBuf {
    let skill_dir = ws.join("skills").join(name);
    write(
        &skill_dir.join("SKILL.md"),
        &format!("---\nname: {name}\ndescription: test skill\n---\n# {name}\n"),
    );
    skill_dir
}

#[test]
fn read_skill_resource_happy_path() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    write(
        &skill_dir.join("scripts").join("hello.sh"),
        "#!/bin/sh\necho hi\n",
    );

    let got = read_skill_resource(ws, "demo", Path::new("scripts/hello.sh"))
        .expect("read should succeed");
    assert_eq!(got, "#!/bin/sh\necho hi\n");
}

#[test]
fn read_skill_resource_rejects_parent_dir_traversal() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    // Put a secret *outside* the skill root.
    write(&ws.join("secret.txt"), "top secret");
    // Put a resource file inside so the skill has at least one bundled
    // asset (makes the test realistic).
    write(&skill_dir.join("scripts").join("ok.sh"), "ok");

    let err = read_skill_resource(ws, "demo", Path::new("../../secret.txt"))
        .expect_err("parent-dir traversal must be rejected");
    assert!(
        err.contains("..") || err.to_lowercase().contains("escape"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_absolute_paths() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    make_legacy_skill(ws, "demo");

    let err = read_skill_resource(ws, "demo", Path::new("/etc/passwd"))
        .expect_err("absolute path must be rejected");
    assert!(
        err.to_lowercase().contains("absolute"),
        "unexpected error: {err}"
    );
}

#[cfg(unix)]
#[test]
fn read_skill_resource_rejects_symlinked_leaf() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");

    // Target lives outside the skill root.
    let external = tempfile::tempdir().unwrap();
    write(&external.path().join("leaked.txt"), "leaked content");

    // Symlink <skill>/scripts/leak.txt -> external/leaked.txt
    std::fs::create_dir_all(skill_dir.join("scripts")).unwrap();
    symlink(
        external.path().join("leaked.txt"),
        skill_dir.join("scripts/leak.txt"),
    )
    .unwrap();

    let err = read_skill_resource(ws, "demo", Path::new("scripts/leak.txt"))
        .expect_err("symlinked leaf must be rejected");
    assert!(
        err.to_lowercase().contains("symlink") || err.to_lowercase().contains("escape"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_oversized_file() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    // Write MAX + 1 bytes.
    let oversize = vec![b'a'; (MAX_SKILL_RESOURCE_BYTES as usize) + 1];
    let target = skill_dir.join("references").join("big.txt");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, &oversize).unwrap();

    let err = read_skill_resource(ws, "demo", Path::new("references/big.txt"))
        .expect_err("oversized file must be rejected");
    assert!(
        err.to_lowercase().contains("exceeds") || err.to_lowercase().contains("limit"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_non_utf8_bytes() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    // 0xFF is never valid UTF-8 (invalid start byte in any multi-byte
    // sequence).
    let target = skill_dir.join("assets").join("binary.bin");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, [0xFFu8, 0xFE, 0xFD, 0xFC]).unwrap();

    let err = read_skill_resource(ws, "demo", Path::new("assets/binary.bin"))
        .expect_err("non-UTF-8 content must be rejected");
    assert!(
        err.to_lowercase().contains("utf-8"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_unknown_skill() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();

    let err = read_skill_resource(ws, "does-not-exist", Path::new("scripts/x.sh"))
        .expect_err("unknown skill must be rejected");
    assert!(
        err.to_lowercase().contains("not found"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_directory_target() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    std::fs::create_dir_all(skill_dir.join("scripts").join("nested")).unwrap();

    let err = read_skill_resource(ws, "demo", Path::new("scripts/nested"))
        .expect_err("directory target must be rejected");
    assert!(
        err.to_lowercase().contains("not a regular file"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_empty_inputs() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    make_legacy_skill(ws, "demo");

    let err = read_skill_resource(ws, "", Path::new("scripts/x.sh"))
        .expect_err("empty skill_id must be rejected");
    assert!(err.to_lowercase().contains("skill_id"), "unexpected: {err}");

    let err = read_skill_resource(ws, "demo", Path::new(""))
        .expect_err("empty relative_path must be rejected");
    assert!(
        err.to_lowercase().contains("relative_path"),
        "unexpected: {err}"
    );
}

// -- create_skill --------------------------------------------------------

#[test]
fn create_skill_user_scope_scaffolds_skill_md_and_resource_dirs() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateSkillParams {
        name: "My Demo Skill".to_string(),
        description: "Send a friendly greeting to the user.".to_string(),
        scope: SkillScope::User,
        license: Some("MIT".to_string()),
        author: Some("Jane Dev".to_string()),
        tags: vec!["demo".to_string(), "greeting".to_string()],
        allowed_tools: vec!["shell".to_string()],
    };

    let created = create_skill_inner(Some(home.path()), ws.path(), params)
        .expect("create_skill should succeed");

    assert_eq!(created.name, "my-demo-skill");
    assert_eq!(created.scope, SkillScope::User);
    assert_eq!(created.description, "Send a friendly greeting to the user.");
    assert_eq!(created.author.as_deref(), Some("Jane Dev"));
    assert_eq!(
        created.tags,
        vec!["demo".to_string(), "greeting".to_string()]
    );
    assert_eq!(created.tools, vec!["shell".to_string()]);

    let skill_root = home
        .path()
        .join(".openhuman")
        .join("skills")
        .join("my-demo-skill");
    assert!(skill_root.join(SKILL_MD).is_file());
    for sub in RESOURCE_DIRS {
        assert!(skill_root.join(sub).is_dir(), "missing scaffold dir: {sub}");
    }

    // Frontmatter round-trips through the parser.
    let on_disk = std::fs::read_to_string(skill_root.join(SKILL_MD)).unwrap();
    assert!(on_disk.contains("name: my-demo-skill"));
    assert!(on_disk.contains("license: MIT"));
    assert!(on_disk.contains("author: Jane Dev"));
}

#[test]
fn create_skill_rejects_slug_collision() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateSkillParams {
        name: "collider".to_string(),
        description: "first".to_string(),
        scope: SkillScope::User,
        ..Default::default()
    };
    create_skill_inner(Some(home.path()), ws.path(), params.clone()).unwrap();

    let err = create_skill_inner(Some(home.path()), ws.path(), params)
        .expect_err("second create with same name must fail");
    assert!(
        err.to_lowercase().contains("already exists"),
        "unexpected error: {err}"
    );
}

#[test]
fn create_skill_rejects_non_alphanumeric_name() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateSkillParams {
        name: "   ///   ".to_string(),
        description: "nothing useful".to_string(),
        scope: SkillScope::User,
        ..Default::default()
    };
    let err = create_skill_inner(Some(home.path()), ws.path(), params)
        .expect_err("non-alphanumeric name must be rejected");
    // Either the empty-name guard or the slugify guard catches this.
    assert!(
        err.to_lowercase().contains("alphanumeric") || err.to_lowercase().contains("empty"),
        "unexpected error: {err}"
    );
}

#[test]
fn create_skill_rejects_project_scope_without_trust_marker() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    // Intentionally no trust marker.

    let params = CreateSkillParams {
        name: "project-skill".to_string(),
        description: "scoped to ws".to_string(),
        scope: SkillScope::Project,
        ..Default::default()
    };
    let err = create_skill_inner(Some(home.path()), ws.path(), params)
        .expect_err("untrusted workspace must reject project scope");
    assert!(
        err.to_lowercase().contains("trust"),
        "unexpected error: {err}"
    );

    // Confirm nothing was written.
    assert!(!ws
        .path()
        .join(".openhuman")
        .join("skills")
        .join("project-skill")
        .exists());
}

#[test]
fn create_skill_project_scope_writes_under_workspace_when_trusted() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();
    write(&ws.path().join(".openhuman").join(TRUST_MARKER), "");

    let params = CreateSkillParams {
        name: "ws-skill".to_string(),
        description: "project-scoped".to_string(),
        scope: SkillScope::Project,
        ..Default::default()
    };
    let created = create_skill_inner(Some(home.path()), ws.path(), params)
        .expect("trusted project-scope create should succeed");

    assert_eq!(created.name, "ws-skill");
    assert_eq!(created.scope, SkillScope::Project);
    assert!(ws
        .path()
        .join(".openhuman")
        .join("skills")
        .join("ws-skill")
        .join(SKILL_MD)
        .is_file());
}

#[test]
fn create_skill_rejects_legacy_scope() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateSkillParams {
        name: "legacy-skill".to_string(),
        description: "no".to_string(),
        scope: SkillScope::Legacy,
        ..Default::default()
    };
    let err = create_skill_inner(Some(home.path()), ws.path(), params)
        .expect_err("legacy scope must be rejected");
    assert!(
        err.to_lowercase().contains("legacy"),
        "unexpected error: {err}"
    );
}

#[test]
fn create_skill_rejects_empty_description() {
    let home = tempfile::tempdir().unwrap();
    let ws = tempfile::tempdir().unwrap();

    let params = CreateSkillParams {
        name: "ok-name".to_string(),
        description: "   ".to_string(),
        scope: SkillScope::User,
        ..Default::default()
    };
    let err = create_skill_inner(Some(home.path()), ws.path(), params)
        .expect_err("empty description must be rejected");
    assert!(
        err.to_lowercase().contains("description"),
        "unexpected error: {err}"
    );
}

#[test]
fn slugify_collapses_separators_and_trims() {
    assert_eq!(slugify_skill_name("Hello  World").unwrap(), "hello-world");
    assert_eq!(slugify_skill_name("--foo__bar--").unwrap(), "foo-bar");
    assert_eq!(
        slugify_skill_name("ALL CAPS skill!").unwrap(),
        "all-caps-skill"
    );
    assert!(slugify_skill_name("   ").is_err());
    assert!(slugify_skill_name("!!!").is_err());
}

#[test]
fn validate_install_url_accepts_public_https() {
    for url in &[
        "https://registry.npmjs.org/@acme/skill",
        "https://example.com/skill.tar.gz",
        "https://github.com/acme/skill/releases/download/v1/skill.tgz",
        "https://8.8.8.8/x",
    ] {
        validate_install_url(url).unwrap_or_else(|e| panic!("{url} rejected: {e}"));
    }
}

#[test]
fn validate_install_url_rejects_non_https_scheme() {
    for url in &[
        "http://example.com/x",
        "ftp://example.com/x",
        "file:///etc/passwd",
        "git+ssh://git@example.com/repo",
        "javascript:alert(1)",
    ] {
        assert!(
            validate_install_url(url).is_err(),
            "{url} should be rejected"
        );
    }
}

#[test]
fn validate_install_url_rejects_empty_and_oversized() {
    assert!(validate_install_url("").is_err());
    assert!(validate_install_url("   ").is_err());
    let huge = format!("https://example.com/{}", "a".repeat(MAX_INSTALL_URL_LEN));
    assert!(validate_install_url(&huge).is_err());
}

#[test]
fn validate_install_url_rejects_private_and_loopback() {
    for url in &[
        "https://localhost/x",
        "https://foo.localhost/x",
        "https://foo.local/x",
        "https://127.0.0.1/x",
        "https://127.42.1.1/x",
        "https://10.0.0.5/x",
        "https://172.16.0.1/x",
        "https://172.31.255.255/x",
        "https://192.168.1.1/x",
        "https://169.254.169.254/x", // cloud metadata IP
        "https://100.64.0.1/x",      // CGN
        "https://0.0.0.0/x",
        "https://255.255.255.255/x",
        "https://224.0.0.1/x", // multicast
        "https://[::1]/x",
        "https://[::]/x",
        "https://[fe80::1]/x",
        "https://[fc00::1]/x",
        "https://[fd12:3456:789a::1]/x",
        "https://[ff02::1]/x",
    ] {
        assert!(
            validate_install_url(url).is_err(),
            "{url} should be rejected"
        );
    }
}

#[test]
fn validate_install_url_rejects_malformed() {
    // missing scheme -> parse error
    assert!(validate_install_url("not-a-url").is_err());
    // special scheme with empty host -> parse error
    assert!(validate_install_url("https://").is_err());
    // non-https scheme rejected even when otherwise well-formed
    assert!(validate_install_url("ftp://example.com/x").is_err());
    // unparseable bracketed host
    assert!(validate_install_url("https://[not-an-ip]/x").is_err());
}

#[test]
fn normalize_install_url_rewrites_github_blob_to_raw() {
    let out =
        normalize_install_url("https://github.com/owner/repo/blob/main/path/to/SKILL.md").unwrap();
    assert_eq!(
        out,
        "https://raw.githubusercontent.com/owner/repo/main/path/to/SKILL.md"
    );
}

#[test]
fn normalize_install_url_rewrites_github_blob_nested_path() {
    let out = normalize_install_url("https://github.com/owner/repo/blob/feat/x/dir/sub/SKILL.md")
        .unwrap();
    assert_eq!(
        out,
        "https://raw.githubusercontent.com/owner/repo/feat/x/dir/sub/SKILL.md"
    );
}

#[test]
fn normalize_install_url_passes_raw_github_through() {
    let raw = "https://raw.githubusercontent.com/owner/repo/main/SKILL.md";
    assert_eq!(normalize_install_url(raw).unwrap(), raw);
}

#[test]
fn normalize_install_url_rejects_tree_urls() {
    let err = normalize_install_url("https://github.com/owner/repo/tree/main/path").unwrap_err();
    assert!(err.contains("unsupported url form"), "{err}");
    assert!(err.contains("tree/dir"), "{err}");
}

#[test]
fn normalize_install_url_rejects_whole_repo() {
    let err = normalize_install_url("https://github.com/owner/repo").unwrap_err();
    assert!(err.contains("unsupported url form"), "{err}");
    assert!(err.contains("whole-repo"), "{err}");
}

#[test]
fn normalize_install_url_rejects_non_md_suffix() {
    let err = normalize_install_url("https://example.com/skill.txt").unwrap_err();
    assert!(err.contains("unsupported url form"), "{err}");
    assert!(err.contains(".md"), "{err}");
}

#[test]
fn normalize_install_url_accepts_uppercase_md_suffix() {
    let raw = "https://example.com/SKILL.MD";
    assert_eq!(normalize_install_url(raw).unwrap(), raw);
}

#[test]
fn derive_install_slug_prefers_metadata_id() {
    let mut fm = SkillFrontmatter {
        name: "My Skill".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    fm.metadata.insert(
        "id".to_string(),
        serde_yaml::Value::String("canonical-id".to_string()),
    );
    assert_eq!(derive_install_slug(&fm).unwrap(), "canonical-id");
}

#[test]
fn derive_install_slug_sanitizes_name_fallback() {
    let fm = SkillFrontmatter {
        name: "My Cool Skill!!".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    assert_eq!(derive_install_slug(&fm).unwrap(), "my-cool-skill");
}

#[test]
fn derive_install_slug_collapses_runs_and_trims_edges() {
    let fm = SkillFrontmatter {
        name: "---foo__bar  baz---".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    assert_eq!(derive_install_slug(&fm).unwrap(), "foo-bar-baz");
}

#[test]
fn derive_install_slug_rejects_empty_after_sanitize() {
    let fm = SkillFrontmatter {
        name: "!!!".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    let err = derive_install_slug(&fm).unwrap_err();
    assert!(err.contains("invalid SKILL.md"), "{err}");
}

#[test]
fn derive_install_slug_rejects_oversized() {
    let fm = SkillFrontmatter {
        name: "a".repeat(MAX_NAME_LEN + 1),
        description: "x".to_string(),
        ..Default::default()
    };
    let err = derive_install_slug(&fm).unwrap_err();
    assert!(err.contains("invalid SKILL.md"), "{err}");
    assert!(err.contains("exceeds"), "{err}");
}

#[test]
fn derive_install_slug_sanitizes_path_escape_attempts() {
    // `..` and `/` are non-alphanumeric so they collapse to `-` during
    // sanitization — verify no path-escape characters survive.
    let fm = SkillFrontmatter {
        name: "../etc/passwd".to_string(),
        description: "x".to_string(),
        ..Default::default()
    };
    let slug = derive_install_slug(&fm).unwrap();
    assert!(!slug.contains(".."), "slug leaked ..: {slug}");
    assert!(!slug.contains('/'), "slug leaked /: {slug}");
    assert!(!slug.contains('\\'), "slug leaked \\: {slug}");
}

#[test]
fn parse_skill_md_str_happy_path() {
    let content = "---\nname: demo\ndescription: a demo skill\n---\n\n# Body\n";
    let (fm, body, warnings) = parse_skill_md_str(content).unwrap();
    assert_eq!(fm.name, "demo");
    assert_eq!(fm.description, "a demo skill");
    assert!(body.contains("# Body"));
    assert!(warnings.is_empty());
}

#[test]
fn parse_skill_md_str_unterminated_frontmatter_returns_none() {
    let content = "---\nname: demo\ndescription: missing close\n# Body\n";
    assert!(parse_skill_md_str(content).is_none());
}

#[test]
fn parse_skill_md_str_no_frontmatter_treats_whole_as_body() {
    let content = "# Just a body\nno frontmatter here\n";
    let (fm, body, warnings) = parse_skill_md_str(content).unwrap();
    assert!(fm.name.is_empty());
    assert_eq!(body, content);
    assert!(warnings.is_empty());
}

#[test]
fn parse_skill_md_str_bad_yaml_returns_empty_frontmatter_with_warning() {
    let content = "---\nname: [unterminated\ndescription: also bad\n---\n";
    let (fm, _body, warnings) = parse_skill_md_str(content).unwrap();
    assert!(fm.name.is_empty());
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("frontmatter parse error")),
        "expected warning, got {warnings:?}"
    );
}

/// Happy path: install a SKILL.md under a synthetic user home, verify
/// discovery sees it, uninstall, verify discovery no longer sees it and
/// the on-disk dir is gone.
#[test]
fn uninstall_skill_removes_user_scope_dir() {
    let home = tempfile::tempdir().unwrap();
    let skill_dir = home
        .path()
        .join(".openhuman")
        .join("skills")
        .join("weather-helper");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: weather-helper\ndescription: forecasts\n---\n\nbody\n",
    );
    let before = discover_skills(Some(home.path()), None, false);
    assert_eq!(before.len(), 1, "setup: skill should be discoverable");

    let outcome = uninstall_skill(
        UninstallSkillParams {
            name: "weather-helper".into(),
        },
        Some(home.path()),
    )
    .unwrap();
    assert_eq!(outcome.name, "weather-helper");
    assert_eq!(outcome.scope, SkillScope::User);
    assert!(!skill_dir.exists(), "uninstall should remove the dir");

    let after = discover_skills(Some(home.path()), None, false);
    assert!(after.is_empty(), "discovery should no longer see it");
}

/// Names containing path separators or traversal sequences are rejected
/// before any filesystem access.
#[test]
fn uninstall_skill_rejects_path_traversal_names() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".openhuman").join("skills")).unwrap();
    for bad in ["../etc", "foo/bar", "foo\\bar", "..", "foo/../bar"] {
        let err = uninstall_skill(UninstallSkillParams { name: bad.into() }, Some(home.path()))
            .unwrap_err();
        assert!(
            err.contains("path separators") || err.contains("is not installed"),
            "name {bad:?} should be rejected before fs access, got: {err}"
        );
    }
}

/// Empty and whitespace-only names return a clear required-field error.
#[test]
fn uninstall_skill_rejects_empty_name() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".openhuman").join("skills")).unwrap();
    for bad in ["", "   ", "\t"] {
        let err = uninstall_skill(UninstallSkillParams { name: bad.into() }, Some(home.path()))
            .unwrap_err();
        assert!(err.contains("name is required"), "{bad:?} => {err}");
    }
}

/// Uninstalling a skill that is not installed surfaces a recognizable
/// error rather than a generic I/O failure.
#[test]
fn uninstall_skill_missing_skill_errors_cleanly() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".openhuman").join("skills")).unwrap();
    let err = uninstall_skill(
        UninstallSkillParams {
            name: "ghost".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(err.contains("not installed"), "got: {err}");
}

/// A directory that does not contain a `SKILL.md` is refused — we only
/// remove things that look like skills we installed, not arbitrary
/// directories the user dropped in.
#[test]
fn uninstall_skill_refuses_dir_without_skill_md() {
    let home = tempfile::tempdir().unwrap();
    let bogus = home.path().join(".openhuman").join("skills").join("bogus");
    std::fs::create_dir_all(&bogus).unwrap();
    std::fs::write(bogus.join("random.txt"), "not a skill").unwrap();
    let err = uninstall_skill(
        UninstallSkillParams {
            name: "bogus".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(
        err.contains("does not look like a SKILL.md skill"),
        "got: {err}"
    );
    assert!(bogus.exists(), "non-skill dir should not be deleted");
}

/// A symlink inside the skills root pointing outside the root must be
/// rejected by the raw-path symlink preflight before `canonicalize`
/// would follow the link. The earlier `starts_with` / `is_dir` guards
/// remain as defence-in-depth for anything that slips past the
/// preflight on future refactors.
#[cfg(unix)]
#[test]
fn uninstall_skill_rejects_symlink_escape() {
    let home = tempfile::tempdir().unwrap();
    let skills_root = home.path().join(".openhuman").join("skills");
    std::fs::create_dir_all(&skills_root).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let target = outside.path().join("real");
    write(
        &target.join("SKILL.md"),
        "---\nname: real\ndescription: out of tree\n---\n",
    );
    std::os::unix::fs::symlink(&target, skills_root.join("real")).unwrap();
    let err = uninstall_skill(
        UninstallSkillParams {
            name: "real".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(
        err.contains("symlinked alias")
            || err.contains("path escapes skills root")
            || err.contains("is not a directory"),
        "symlink out of tree must be rejected, got: {err}"
    );
    assert!(target.exists(), "symlink target must not be deleted");
}

/// An in-tree symlink alias (`skills/alias -> skills/real`) must be
/// rejected even though it does not escape the skills root — otherwise
/// the uninstall of `alias` would nuke the real skill directory behind
/// it, violating the invariant that the named slug is deleted.
#[cfg(unix)]
#[test]
fn uninstall_skill_rejects_symlinked_alias_in_tree() {
    let home = tempfile::tempdir().unwrap();
    let skills_root = home.path().join(".openhuman").join("skills");
    std::fs::create_dir_all(&skills_root).unwrap();
    let real_dir = skills_root.join("real");
    write(
        &real_dir.join("SKILL.md"),
        "---\nname: real\ndescription: in tree\n---\n",
    );
    std::os::unix::fs::symlink(&real_dir, skills_root.join("alias")).unwrap();
    let err = uninstall_skill(
        UninstallSkillParams {
            name: "alias".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(
        err.contains("symlinked alias"),
        "in-tree alias must be rejected by preflight, got: {err}"
    );
    assert!(
        real_dir.join("SKILL.md").exists(),
        "real skill behind the alias must survive"
    );
}

/// A symlinked skills *root* (`~/.openhuman/skills -> elsewhere`) must
/// be refused before canonicalisation, since `canonicalize` would
/// resolve it to the target and the `starts_with` guard would then
/// compare against the resolved target, not the nominal root.
#[cfg(unix)]
#[test]
fn uninstall_skill_rejects_symlinked_skills_root() {
    let home = tempfile::tempdir().unwrap();
    let real_root = tempfile::tempdir().unwrap();
    let real_skills = real_root.path().join("skills");
    std::fs::create_dir_all(&real_skills).unwrap();
    write(
        &real_skills.join("real").join("SKILL.md"),
        "---\nname: real\ndescription: in real root\n---\n",
    );
    std::fs::create_dir_all(home.path().join(".openhuman")).unwrap();
    std::os::unix::fs::symlink(&real_skills, home.path().join(".openhuman").join("skills"))
        .unwrap();
    let err = uninstall_skill(
        UninstallSkillParams {
            name: "real".into(),
        },
        Some(home.path()),
    )
    .unwrap_err();
    assert!(
        err.contains("skills root") && err.contains("symlink"),
        "symlinked skills root must be refused, got: {err}"
    );
    assert!(
        real_skills.join("real").join("SKILL.md").exists(),
        "target must survive"
    );
}
