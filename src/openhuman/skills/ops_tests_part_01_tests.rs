use super::*;

#[test]
fn init_skills_dir_creates_dir_and_readme() {
    let dir = tempfile::tempdir().unwrap();
    init_workflows_dir(dir.path()).unwrap();
    let skills_dir = dir.path().join("skills");
    assert!(skills_dir.is_dir());
    let readme = skills_dir.join("README.md");
    assert!(readme.exists());
}

#[test]
fn load_skills_legacy_json_still_works() {
    let dir = tempfile::tempdir().unwrap();
    init_workflows_dir(dir.path()).unwrap();
    let skill_dir = dir.path().join("skills").join("my-skill");
    std::fs::create_dir_all(&skill_dir).unwrap();
    write(
        &skill_dir.join("skill.json"),
        r#"{"name":"My Workflow","description":"A test","version":"1.0"}"#,
    );
    let skills = load_skills_ws(dir.path());
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "My Workflow");
    assert_eq!(skills[0].description, "A test");
    assert!(skills[0].legacy);
    assert_eq!(skills[0].scope, WorkflowScope::Legacy);
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
    assert_eq!(s.scope, WorkflowScope::Project);
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
    let (fm, _body, _warnings) = parse_workflow_md(&path).unwrap();
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

    let skills = discover_workflows(Some(user_dir.path()), Some(ws_dir.path()), true);
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
fn inventory_resources_lists_hermes_resource_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let skill = dir.path().join("s");
    write(
        &skill.join("SKILL.md"),
        "---\nname: s\ndescription: d\n---\n",
    );
    write(&skill.join("templates").join("page.html"), "<html></html>");
    write(&skill.join("examples").join("demo.md"), "demo");
    write(&skill.join("prompts").join("system.md"), "prompt");

    let mut res = inventory_resources(&skill);
    res.sort();
    assert_eq!(res.len(), 3);
    assert!(res.iter().any(|p| p.ends_with("page.html")));
    assert!(res.iter().any(|p| p.ends_with("demo.md")));
    assert!(res.iter().any(|p| p.ends_with("system.md")));
}

#[test]
fn nested_hermes_skill_tree_discovers_metadata_and_resources() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    write(&ws.join(".openhuman").join("trust"), "");
    let skill_dir = ws
        .join(".openhuman")
        .join("skills")
        .join("creative")
        .join("concept-diagrams");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: concept-diagrams\ndescription: Generate diagrams\nversion: 0.1.0\nauthor: Nous\nplatforms: [linux, macos, windows]\nmetadata:\n  hermes:\n    tags: [diagrams, svg]\n    related_skills: [architecture-diagram]\n---\n",
    );
    write(
        &skill_dir.join("templates").join("template.html"),
        "<html></html>",
    );
    write(&skill_dir.join("examples").join("flow.md"), "flow");

    let skills = load_skills_ws(ws);
    assert_eq!(skills.len(), 1);
    let s = &skills[0];
    assert_eq!(s.name, "concept-diagrams");
    assert_eq!(s.version, "0.1.0");
    assert_eq!(s.author.as_deref(), Some("Nous"));
    assert_eq!(s.platforms, vec!["linux", "macos", "windows"]);
    assert_eq!(s.tags, vec!["diagrams", "svg"]);
    assert_eq!(s.related_skills, vec!["architecture-diagram"]);
    assert_eq!(s.source_format, "hermes");
    assert!(s.resources.iter().any(|p| p.ends_with("template.html")));
    assert!(s.resources.iter().any(|p| p.ends_with("flow.md")));
}

#[test]
fn parse_skill_md_without_frontmatter_returns_body() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    write(&path, "just a markdown body\n");
    let (fm, body, _warnings) = parse_workflow_md(&path).unwrap();
    assert!(fm.name.is_empty());
    assert!(body.contains("markdown body"));
}

#[test]
fn parse_skill_md_unterminated_frontmatter_returns_none() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("SKILL.md");
    write(&path, "---\nname: bad\n\nbody without closing marker\n");
    assert!(parse_workflow_md(&path).is_none());
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
    // load_workflow_metadata now delegates to discover_workflows with dirs::home_dir(),
    // so user-scope skills reach production callers that still hit the
    // backwards-compat shim. Simulate this with an explicit tempdir home
    // via discover_workflows — we can't safely override the process HOME in
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

    let skills = discover_workflows(
        Some(user_dir.path()),
        Some(ws_dir.path()),
        is_workspace_trusted(ws_dir.path()),
    );
    assert_eq!(skills.len(), 1);
    assert_eq!(skills[0].name, "user-only");
    assert_eq!(skills[0].scope, WorkflowScope::User);
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

#[test]
fn read_skill_resource_happy_path() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    write(
        &skill_dir.join("scripts").join("hello.sh"),
        "#!/bin/sh\necho hi\n",
    );

    let got = read_workflow_resource(ws, "demo", Path::new("scripts/hello.sh"))
        .expect("read should succeed");
    assert_eq!(got, "#!/bin/sh\necho hi\n");
}

#[test]
fn read_skill_resource_uses_directory_id_when_display_name_differs() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = ws.join("skills").join("demo-slug");
    write(
        &skill_dir.join("SKILL.md"),
        "---\nname: Demo Display\ndescription: test skill\n---\n",
    );
    write(&skill_dir.join("references").join("note.md"), "slug read");

    let got = read_workflow_resource(ws, "demo-slug", Path::new("references/note.md"))
        .expect("directory id should resolve");
    assert_eq!(got, "slug read");
}

#[test]
fn read_skill_resource_rejects_directory_name_collision() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();

    let named_demo = ws.join("skills").join("alpha");
    write(
        &named_demo.join("SKILL.md"),
        "---\nname: demo\ndescription: display-name collision\n---\n",
    );
    write(&named_demo.join("references").join("note.md"), "alpha");

    let slug_demo = ws.join("skills").join("demo");
    write(
        &slug_demo.join("SKILL.md"),
        "---\nname: slug-demo\ndescription: slug collision\n---\n",
    );
    write(&slug_demo.join("references").join("note.md"), "demo");

    let err = read_workflow_resource(ws, "demo", Path::new("references/note.md"))
        .expect_err("ambiguous directory/name collision must be rejected");
    assert!(
        err.to_lowercase().contains("matches both"),
        "unexpected error: {err}"
    );
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

    let err = read_workflow_resource(ws, "demo", Path::new("../../secret.txt"))
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

    let absolute = if cfg!(windows) {
        Path::new("C:\\Windows\\System32\\drivers\\etc\\hosts")
    } else {
        Path::new("/etc/passwd")
    };
    let err =
        read_workflow_resource(ws, "demo", absolute).expect_err("absolute path must be rejected");
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

    let err = read_workflow_resource(ws, "demo", Path::new("scripts/leak.txt"))
        .expect_err("symlinked leaf must be rejected");
    assert!(
        err.to_lowercase().contains("symlink") || err.to_lowercase().contains("escape"),
        "unexpected error: {err}"
    );
}

/// The leaf-symlink pre-check (`symlink_metadata` on the requested path) only
/// catches a symlinked *file*. A symlinked *intermediate directory component*
/// slips past it — `leaf_meta.file_type().is_symlink()` is false because the
/// leaf itself is a plain file, just reached through a symlinked ancestor.
/// This is what the second, full-path `canonicalize` + `starts_with` check
/// exists to catch, and is untested independently of the leaf-symlink case.
#[cfg(unix)]
#[test]
fn read_skill_resource_rejects_an_intermediate_symlinked_directory() {
    use std::os::unix::fs::symlink;

    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");

    // Target lives outside the skill root, and the leaf inside it is an
    // ordinary file — nothing about the leaf itself is a symlink.
    let external = tempfile::tempdir().unwrap();
    write(&external.path().join("leaked.txt"), "leaked content");

    // <skill>/scripts -> external, so <skill>/scripts/leaked.txt resolves to
    // a real, non-symlink file that nonetheless lives outside the skill root.
    symlink(external.path(), skill_dir.join("scripts")).unwrap();

    let err = read_workflow_resource(ws, "demo", Path::new("scripts/leaked.txt"))
        .expect_err("an intermediate symlinked directory must not leak a file outside the root");
    assert!(
        err.to_lowercase().contains("symlink")
            || err.to_lowercase().contains("escape")
            || err.to_lowercase().contains("outside"),
        "unexpected error: {err}"
    );
}

#[test]
fn read_skill_resource_rejects_oversized_file() {
    let dir = tempfile::tempdir().unwrap();
    let ws = dir.path();
    let skill_dir = make_legacy_skill(ws, "demo");
    // Write MAX + 1 bytes.
    let oversize = vec![b'a'; (MAX_WORKFLOW_RESOURCE_BYTES as usize) + 1];
    let target = skill_dir.join("references").join("big.txt");
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    std::fs::write(&target, &oversize).unwrap();

    let err = read_workflow_resource(ws, "demo", Path::new("references/big.txt"))
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

    let err = read_workflow_resource(ws, "demo", Path::new("assets/binary.bin"))
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

    let err = read_workflow_resource(ws, "does-not-exist", Path::new("scripts/x.sh"))
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

    let err = read_workflow_resource(ws, "demo", Path::new("scripts/nested"))
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

    let err = read_workflow_resource(ws, "", Path::new("scripts/x.sh"))
        .expect_err("empty skill_id must be rejected");
    assert!(err.to_lowercase().contains("skill_id"), "unexpected: {err}");

    let err = read_workflow_resource(ws, "demo", Path::new(""))
        .expect_err("empty relative_path must be rejected");
    assert!(
        err.to_lowercase().contains("relative_path"),
        "unexpected: {err}"
    );
}
