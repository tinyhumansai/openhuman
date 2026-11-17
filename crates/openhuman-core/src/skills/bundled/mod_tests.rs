//! Tests for the compiled-in skill table and its host integration.
//!
//! Bundle validation, digests and materialisation are tested in `tinyskills`.

use super::*;

#[test]
fn every_shipped_bundle_is_valid() {
    // The one assertion that covers the real table. `validate` exists because
    // a bad `dir_name` or a `..` in a file path would write outside the
    // workspace; a shipped build never runs it, so this is where it runs.
    for skill in BUNDLED {
        skill
            .validate()
            .unwrap_or_else(|e| panic!("bundled skill `{}` is invalid: {e}", skill.dir_name));
    }
}

#[test]
fn shipped_bundle_names_are_unique() {
    // Two entries with one `dir_name` would write over each other, and which
    // one won would depend on table order.
    let mut seen = std::collections::HashSet::new();
    for skill in BUNDLED {
        assert!(
            seen.insert(skill.dir_name),
            "two bundled skills both claim `{}`",
            skill.dir_name
        );
    }
}

#[test]
fn install_reports_rather_than_fails_the_boot() {
    // `install` is called from workspace init. A workspace it cannot write
    // must not stop the core from starting.
    let tmp = tempfile::tempdir().expect("tempdir");
    let report = install(tmp.path());
    assert!(
        report.failed.is_empty(),
        "clean workspace must install: {report:?}"
    );
    assert_eq!(
        report.written.len() + report.unchanged.len(),
        BUNDLED.len(),
        "every bundled skill must be accounted for"
    );
}

#[test]
fn a_materialised_bundle_is_discoverable_and_readable_end_to_end() {
    // The test that proves the whole mechanism, because every other test here
    // stops at the filesystem. A bundle can be written correctly and still be
    // useless: discovery could skip the builtin root, the scope could be
    // rejected, the frontmatter could fail to parse, or the resource reader
    // could refuse a path under a root it does not recognise. This walks the
    // real path the model walks — install, discover, read a page.
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    let report = install(workspace);
    assert!(report.failed.is_empty(), "install failed: {report:?}");

    for skill in BUNDLED {
        let found = crate::skills::discover_workflows(None, Some(workspace), false);
        let entry = found
            .iter()
            .find(|w| w.dir_name == skill.dir_name)
            .unwrap_or_else(|| {
                panic!(
                    "`{}` was written but discovery did not find it; discovered: {:?}",
                    skill.dir_name,
                    found.iter().map(|w| &w.dir_name).collect::<Vec<_>>()
                )
            });
        assert_eq!(
            entry.scope,
            tinyskills::SkillScope::Builtin,
            "a materialised bundle must carry the Builtin scope"
        );
        assert!(
            !entry.description.is_empty(),
            "`{}` frontmatter did not parse — discovery found the directory but \
             not its metadata",
            skill.dir_name
        );

        // `home_dir = None` above; the reader resolves through the real
        // discovery pipeline including `dirs::home_dir()`, which is fine here:
        // it can only ADD skills, and we look up ours by name.
        for file in skill.files {
            if file.path == tinyskills::WORKFLOW_MD {
                continue;
            }
            let body = crate::skills::read_workflow_resource(
                workspace,
                skill.dir_name,
                std::path::Path::new(file.path),
            )
            .unwrap_or_else(|e| {
                panic!(
                    "reading `{}` of `{}` failed: {e}",
                    file.path, skill.dir_name
                )
            });
            assert_eq!(
                body, file.contents,
                "`{}` read back different bytes than it ships",
                file.path
            );
        }
    }
}

#[test]
fn a_user_skill_of_the_same_name_shadows_the_builtin() {
    // The precedence promise from the module docs, and the one that makes
    // shipping a bundle safe: adding a builtin can never take a name away from
    // a workspace that already used it.
    let Some(first) = BUNDLED.first() else {
        return;
    };
    let tmp = tempfile::tempdir().expect("tempdir");
    let workspace = tmp.path();
    install(workspace);

    // A legacy-scope skill (`<workspace>/skills/`) is the lowest non-builtin
    // scope, so if even that wins, every other scope does.
    let user_dir = workspace.join("skills").join(first.dir_name);
    std::fs::create_dir_all(&user_dir).expect("mkdir");
    std::fs::write(
        user_dir.join("SKILL.md"),
        format!(
            "---\nname: {}\ndescription: the user's own version\n---\n\nmine\n",
            first.dir_name
        ),
    )
    .expect("write");

    let found = crate::skills::discover_workflows(None, Some(workspace), false);
    let entry = found
        .iter()
        .find(|w| w.dir_name == first.dir_name)
        .expect("found");
    assert_eq!(
        entry.description, "the user's own version",
        "the user's skill must win the name; got scope {:?}",
        entry.scope
    );
}
